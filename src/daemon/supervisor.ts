import { execFile, spawn, ChildProcess } from "node:child_process";
import { EventEmitter } from "node:events";
import fs from "node:fs";
import path from "node:path";
import {
  BatchItemResult,
  BatchResult,
  LogLine,
  ServiceInfo,
  ServiceSpec,
  ServiceSpecInput,
  ServiceStatus,
} from "../protocol.js";
import { LOG_DIR, REGISTRY_FILE } from "../paths.js";

const RING_SIZE = 2000; // 每个服务内存中保留的日志行数
const STOP_GRACE_MS = 5000; // SIGTERM 后等待退出，超时则 SIGKILL
const SETTLE_MS = 1000; // 启动后观察这段时间，确认进程没有立刻崩溃（如端口被占）
const BATCH_CONCURRENCY = 4;

interface ProcessUsage {
  cpuPercent: number;
  memoryBytes: number;
}

interface ManagedService {
  spec: ServiceSpec;
  status: ServiceStatus;
  child?: ChildProcess;
  pid?: number;
  startedAt?: number;
  lastExitCode?: number | null;
  lastExitSignal?: string | null;
  restarts: number;
  ring: LogLine[];
  logStream?: fs.WriteStream;
  /** 标记本次停止是「人为重启/停止」，用于区分崩溃 */
  intentionalStop: boolean;
  /** 正处于 restart() 流程中（停旧→起新之间），用于告知前台客户端别退出 */
  restarting: boolean;
  /** 等待进程退出的 promise 解析器 */
  exitWaiters: Array<() => void>;
  /** 各输出流中尚未遇到换行的半行（chunk 边界可能切在行中间） */
  partial: { stdout: string; stderr: string };
  /** 操作队列：start/stop/restart/remove 按序执行，避免并发交错 */
  opQueue: Promise<unknown>;
}

/**
 * Supervisor 持有所有服务进程。它本身与 IPC 无关，只发出事件：
 *  - "log"    (LogLine)
 *  - "status" (ServiceInfo)
 * 由 daemon 的 IPC 层订阅并转发给 attach 的客户端。
 */
export class Supervisor extends EventEmitter {
  private services = new Map<string, ManagedService>();
  /**
   * 所有外部状态修改请求串行进入。批量操作可在内部并发，但其名单快照和执行期间
   * 不会被另一条 register/start/stop/restart/remove 请求穿插。
   */
  private mutationQueue: Promise<unknown> = Promise.resolve();

  constructor() {
    super();
    fs.mkdirSync(LOG_DIR, { recursive: true });
    this.loadRegistry();
  }

  async list(): Promise<ServiceInfo[]> {
    const services = [...this.services.values()];
    const usage = await this.readProcessUsage();
    return services.map((s) => {
      const info = this.toInfo(s);
      const stats = s.pid === undefined ? undefined : usage.get(s.pid);
      return stats ? { ...info, ...stats } : info;
    });
  }

  get(name: string): ServiceInfo | undefined {
    const s = this.services.get(name);
    return s ? this.toInfo(s) : undefined;
  }

  has(name: string): boolean {
    return this.services.has(name);
  }

  /** 注册或更新一个服务定义；start=true 时在同一修改队列中接着启动。 */
  register(input: ServiceSpecInput, start = false): Promise<ServiceInfo> {
    return this.runMutation(async () => {
      const info = this.registerNow(input);
      return start ? this.startOne(input.name) : info;
    });
  }

  private registerNow(input: ServiceSpecInput): ServiceInfo {
    const existing = this.services.get(input.name);
    const spec: ServiceSpec = {
      name: input.name,
      command: input.command,
      cwd: input.cwd,
      env: input.env,
      port: input.port,
      autorestart: input.autorestart ?? false,
    };
    if (existing) {
      // 运行中只更新「下一次启动」生效的定义，不打断当前进程
      existing.spec = spec;
      this.persistRegistry();
      this.emitStatus(existing);
      return this.toInfo(existing);
    }
    const svc = this.newService(spec);
    this.services.set(spec.name, svc);
    this.persistRegistry();
    this.system(svc, `registered: ${spec.command}  (cwd: ${spec.cwd})`);
    this.emitStatus(svc);
    return this.toInfo(svc);
  }

  async start(name: string): Promise<ServiceInfo> {
    return this.runMutation(() => this.startOne(name));
  }

  private async startOne(name: string): Promise<ServiceInfo> {
    const svc = this.must(name);
    return this.runExclusive(svc, async () => {
      if (svc.status === "running" || svc.status === "starting") {
        return this.toInfo(svc);
      }
      this.spawnProcess(svc);
      await this.waitSettle(name, SETTLE_MS);
      return this.toInfo(svc);
    });
  }

  async stop(name: string): Promise<ServiceInfo> {
    return this.runMutation(() => this.stopOne(name));
  }

  private async stopOne(name: string): Promise<ServiceInfo> {
    const svc = this.must(name);
    return this.runExclusive(svc, async () => {
      await this.stopProcess(svc);
      return this.toInfo(svc);
    });
  }

  /**
   * 重启：停止后用最新 spec 重新拉起。
   * 关键点 —— 进程是 daemon 的子进程，重启只换底层进程，
   * 已 attach 的终端（人）保持连接不断。
   */
  async restart(name: string): Promise<ServiceInfo> {
    return this.runMutation(() => this.restartOne(name));
  }

  private async restartOne(name: string): Promise<ServiceInfo> {
    const svc = this.must(name);
    return this.runExclusive(svc, async () => {
      this.system(svc, "restarting...");
      svc.restarting = true;
      try {
        await this.stopProcess(svc);
        svc.restarts = 0;
        this.spawnProcess(svc);
        await this.waitSettle(name, SETTLE_MS);
      } finally {
        svc.restarting = false;
        this.emitStatus(svc);
      }
      return this.toInfo(svc);
    });
  }

  async remove(name: string): Promise<void> {
    await this.runMutation(() => this.removeOne(name, true));
  }

  private async removeOne(name: string, persist: boolean): Promise<void> {
    const svc = this.must(name);
    await this.runExclusive(svc, async () => {
      await this.stopProcess(svc);
      svc.logStream?.end();
      this.services.delete(name);
      if (persist) this.persistRegistry();
    });
  }

  startAll(): Promise<BatchResult> {
    return this.runMutation(async () => {
      const names = [...this.services.keys()];
      const items = await this.mapBatch(names, async (name): Promise<BatchItemResult> => {
        const before = this.must(name);
        if (before.status === "running" || before.status === "starting") {
          return {
            name,
            outcome: "skipped",
            reason: before.status === "running" ? "already-running" : "already-starting",
            info: this.toInfo(before),
          };
        }
        try {
          const info = await this.startOne(name);
          if (info.status === "running" || info.status === "starting") {
            return { name, outcome: "started", info };
          }
          return {
            name,
            outcome: "failed",
            info,
            error: `启动后状态为 ${info.status}`,
          };
        } catch (err) {
          return { name, outcome: "failed", error: this.errorMessage(err) };
        }
      });
      return { action: "start", items };
    });
  }

  stopAll(): Promise<BatchResult> {
    return this.runMutation(async () => {
      const names = [...this.services.keys()];
      const items = await this.mapBatch(names, async (name): Promise<BatchItemResult> => {
        const before = this.must(name);
        if (!before.child || before.pid === undefined) {
          return {
            name,
            outcome: "skipped",
            reason: "not-running",
            info: this.toInfo(before),
          };
        }
        try {
          const info = await this.stopOne(name);
          return { name, outcome: "stopped", info };
        } catch (err) {
          return {
            name,
            outcome: "failed",
            info: this.get(name),
            error: this.errorMessage(err),
          };
        }
      });
      return { action: "stop", items };
    });
  }

  removeAll(): Promise<BatchResult> {
    return this.runMutation(async () => {
      const names = [...this.services.keys()];
      const items = await this.mapBatch(names, async (name): Promise<BatchItemResult> => {
        try {
          await this.removeOne(name, false);
          return { name, outcome: "removed" };
        } catch (err) {
          return {
            name,
            outcome: "failed",
            info: this.get(name),
            error: this.errorMessage(err),
          };
        }
      });
      this.persistRegistry();
      return { action: "remove", items };
    });
  }

  /** 返回最近 n 行日志 */
  logs(name: string, lines = 200): LogLine[] {
    const svc = this.must(name);
    return svc.ring.slice(-lines);
  }

  // ---- 内部 ----

  private runMutation<T>(fn: () => Promise<T> | T): Promise<T> {
    const run = this.mutationQueue.then(fn, fn);
    this.mutationQueue = run.catch(() => undefined);
    return run;
  }

  /** 有界并发，结果顺序始终与注册表快照一致。 */
  private async mapBatch<T>(names: string[], fn: (name: string) => Promise<T>): Promise<T[]> {
    const results = new Array<T>(names.length);
    let next = 0;
    const worker = async () => {
      for (;;) {
        const index = next++;
        if (index >= names.length) return;
        results[index] = await fn(names[index]);
      }
    };
    await Promise.all(
      Array.from({ length: Math.min(BATCH_CONCURRENCY, names.length) }, () => worker())
    );
    return results;
  }

  private errorMessage(err: unknown): string {
    return err instanceof Error ? err.message : String(err);
  }

  /**
   * detached 服务的 pid 同时也是进程组 id。一次 ps 扫描汇总所有进程组，
   * 因而 npm/node 等命令派生出来的子进程也计入服务资源使用量。
   * ps 不可用或采集失败时返回空结果，不影响 list 的基础状态展示。
   */
  private readProcessUsage(): Promise<Map<number, ProcessUsage>> {
    return new Promise((resolve) => {
      execFile(
        "ps",
        ["-axo", "pgid=,pcpu=,rss="],
        {
          encoding: "utf8",
          env: { ...process.env, LC_ALL: "C" },
          maxBuffer: 10 * 1024 * 1024,
          timeout: 2000,
        },
        (err, stdout) => {
          const usage = new Map<number, ProcessUsage>();
          if (err) return resolve(usage);
          for (const line of stdout.split("\n")) {
            const match = line.trim().match(/^(\d+)\s+([\d.]+)\s+(\d+)$/);
            if (!match) continue;
            const pgid = Number(match[1]);
            const current = usage.get(pgid) ?? { cpuPercent: 0, memoryBytes: 0 };
            current.cpuPercent += Number(match[2]);
            current.memoryBytes += Number(match[3]) * 1024;
            usage.set(pgid, current);
          }
          resolve(usage);
        }
      );
    });
  }

  private newService(spec: ServiceSpec): ManagedService {
    return {
      spec,
      status: "stopped",
      restarts: 0,
      ring: [],
      intentionalStop: false,
      restarting: false,
      exitWaiters: [],
      partial: { stdout: "", stderr: "" },
      opQueue: Promise.resolve(),
    };
  }

  /**
   * 同一服务上的操作排队执行：stop 进行到一半时来了 start，
   * 必须等旧进程完全退出再拉新，否则会出现两个进程、状态互相覆盖。
   */
  private runExclusive<T>(svc: ManagedService, fn: () => Promise<T>): Promise<T> {
    const run = svc.opQueue.then(fn, fn);
    svc.opQueue = run.catch(() => undefined);
    return run;
  }

  /** daemon 启动时恢复上次的服务定义（只恢复注册，不自动拉起） */
  private loadRegistry(): void {
    let specs: unknown;
    try {
      specs = JSON.parse(fs.readFileSync(REGISTRY_FILE, "utf8"));
    } catch {
      return; // 不存在或损坏，从空注册表开始
    }
    if (!Array.isArray(specs)) return;
    for (const spec of specs as ServiceSpec[]) {
      if (!spec || typeof spec.name !== "string" || typeof spec.command !== "string") {
        continue;
      }
      const svc = this.newService(spec);
      this.services.set(spec.name, svc);
      this.system(svc, `restored from registry: ${spec.command}  (cwd: ${spec.cwd})`);
    }
  }

  private persistRegistry(): void {
    const specs = [...this.services.values()].map((s) => s.spec);
    const tmp = REGISTRY_FILE + ".tmp";
    try {
      fs.writeFileSync(tmp, JSON.stringify(specs, null, 2));
      fs.renameSync(tmp, REGISTRY_FILE);
    } catch {
      // 持久化失败不影响当前运行，下次写入再试
    }
  }

  /**
   * 启动后观察一小段时间：若进程在窗口内退出/报错（典型如端口被占
   * EADDRINUSE），提前返回，让调用方拿到真实的失败状态而非乐观的 running。
   */
  private waitSettle(name: string, ms: number): Promise<void> {
    return new Promise((resolve) => {
      const onStatus = (info: ServiceInfo) => {
        if (info.name !== name) return;
        if (info.status === "exited" || info.status === "errored") {
          cleanup();
          resolve();
        }
      };
      const timer = setTimeout(() => {
        cleanup();
        resolve();
      }, ms);
      const cleanup = () => {
        clearTimeout(timer);
        this.off("status", onStatus);
      };
      this.on("status", onStatus);
    });
  }

  private spawnProcess(svc: ManagedService): void {
    svc.status = "starting";
    svc.intentionalStop = false;
    svc.partial = { stdout: "", stderr: "" };
    this.emitStatus(svc);

    const child = spawn(svc.spec.command, {
      cwd: svc.spec.cwd,
      env: { ...process.env, ...(svc.spec.env ?? {}) },
      shell: true,
      detached: true, // 独立进程组，便于整组终止
      stdio: ["ignore", "pipe", "pipe"],
    });

    svc.child = child;
    svc.pid = child.pid;
    svc.startedAt = Date.now();

    const logStream = fs.createWriteStream(
      path.join(LOG_DIR, `${svc.spec.name}.log`),
      { flags: "a" }
    );
    svc.logStream = logStream;

    child.stdout?.on("data", (b) => this.ingest(svc, "stdout", b));
    child.stderr?.on("data", (b) => this.ingest(svc, "stderr", b));

    child.on("spawn", () => {
      svc.status = "running";
      this.system(svc, `started (pid ${svc.pid})`);
      this.emitStatus(svc);
    });

    child.on("error", (err) => {
      svc.status = "errored";
      this.system(svc, `spawn error: ${err.message}`);
      this.emitStatus(svc);
      this.resolveExitWaiters(svc);
    });

    child.on("exit", (code, signal) => {
      this.flushPartial(svc);
      svc.lastExitCode = code;
      svc.lastExitSignal = signal;
      svc.child = undefined;
      svc.pid = undefined;
      const wasIntentional = svc.intentionalStop;
      svc.status = wasIntentional ? "stopped" : "exited";
      this.system(
        svc,
        `exited (code ${code ?? "null"}${signal ? `, signal ${signal}` : ""})`
      );
      svc.logStream?.end();
      svc.logStream = undefined;
      this.emitStatus(svc);
      this.resolveExitWaiters(svc);

      // 崩溃自动重启
      if (!wasIntentional && svc.spec.autorestart) {
        this.system(svc, "autorestart...");
        svc.restarts++;
        setTimeout(() => {
          void this.runMutation(() => {
            // 仍存在且未被人为启动才重启
            if (this.services.get(svc.spec.name) === svc && svc.status === "exited") {
              this.spawnProcess(svc);
            }
          });
        }, 500);
      }
    });
  }

  private async stopProcess(svc: ManagedService): Promise<void> {
    const child = svc.child;
    if (!child || child.pid === undefined) {
      svc.status = "stopped";
      this.emitStatus(svc);
      return;
    }
    svc.intentionalStop = true;
    svc.status = "stopping";
    this.emitStatus(svc);

    const pid = child.pid;
    const exited = new Promise<void>((resolve) =>
      svc.exitWaiters.push(resolve)
    );

    this.killGroup(pid, "SIGTERM");

    const timer = setTimeout(() => {
      this.system(svc, "SIGTERM 超时，强制 SIGKILL");
      this.killGroup(pid, "SIGKILL");
    }, STOP_GRACE_MS);

    await exited;
    clearTimeout(timer);
  }

  /** detached:true 时杀整个进程组（负 pid） */
  private killGroup(pid: number, signal: NodeJS.Signals): void {
    try {
      process.kill(-pid, signal);
    } catch {
      // 进程组可能已不存在，退化为单进程
      try {
        process.kill(pid, signal);
      } catch {
        /* 已退出 */
      }
    }
  }

  private resolveExitWaiters(svc: ManagedService): void {
    const waiters = svc.exitWaiters;
    svc.exitWaiters = [];
    for (const w of waiters) w();
  }

  /**
   * stdout/stderr 是字节流，一行日志可能被切在两个 chunk 里：
   * 只有遇到换行才提交，末尾的半行留在 partial，等下个 chunk 或进程退出时补全。
   */
  private ingest(
    svc: ManagedService,
    stream: "stdout" | "stderr",
    chunk: Buffer
  ): void {
    const parts = (svc.partial[stream] + chunk.toString("utf8")).split(/\r?\n/);
    svc.partial[stream] = parts.pop() ?? "";
    for (const raw of parts) {
      if (raw === "") continue;
      this.pushLine(svc, stream, raw);
    }
  }

  /** 进程退出时把没带换行的最后半行也写出来 */
  private flushPartial(svc: ManagedService): void {
    for (const stream of ["stdout", "stderr"] as const) {
      const rest = svc.partial[stream];
      if (rest === "") continue;
      svc.partial[stream] = "";
      this.pushLine(svc, stream, rest);
    }
  }

  private system(svc: ManagedService, line: string): void {
    this.pushLine(svc, "system", line);
  }

  private pushLine(
    svc: ManagedService,
    stream: LogLine["stream"],
    line: string
  ): void {
    const entry: LogLine = {
      name: svc.spec.name,
      stream,
      line,
      ts: Date.now(),
    };
    svc.ring.push(entry);
    if (svc.ring.length > RING_SIZE) svc.ring.shift();
    svc.logStream?.write(line + "\n");
    this.emit("log", entry);
  }

  private emitStatus(svc: ManagedService): void {
    this.emit("status", this.toInfo(svc));
  }

  private toInfo(svc: ManagedService): ServiceInfo {
    return {
      ...svc.spec,
      status: svc.status,
      pid: svc.pid,
      startedAt: svc.startedAt,
      lastExitCode: svc.lastExitCode ?? null,
      lastExitSignal: svc.lastExitSignal ?? null,
      restarts: svc.restarts,
      restarting: svc.restarting,
    };
  }

  private must(name: string): ManagedService {
    const svc = this.services.get(name);
    if (!svc) throw new Error(`未知服务: ${name}`);
    return svc;
  }
}
