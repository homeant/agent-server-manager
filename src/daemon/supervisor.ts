import { spawn, ChildProcess } from "node:child_process";
import { EventEmitter } from "node:events";
import fs from "node:fs";
import path from "node:path";
import {
  LogLine,
  ServiceInfo,
  ServiceSpec,
  ServiceSpecInput,
  ServiceStatus,
} from "../protocol.js";
import { LOG_DIR } from "../paths.js";

const RING_SIZE = 2000; // 每个服务内存中保留的日志行数
const STOP_GRACE_MS = 5000; // SIGTERM 后等待退出，超时则 SIGKILL
const SETTLE_MS = 1000; // 启动后观察这段时间，确认进程没有立刻崩溃（如端口被占）

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
}

/**
 * Supervisor 持有所有服务进程。它本身与 IPC 无关，只发出事件：
 *  - "log"    (LogLine)
 *  - "status" (ServiceInfo)
 * 由 daemon 的 IPC 层订阅并转发给 attach 的客户端。
 */
export class Supervisor extends EventEmitter {
  private services = new Map<string, ManagedService>();

  constructor() {
    super();
    fs.mkdirSync(LOG_DIR, { recursive: true });
  }

  list(): ServiceInfo[] {
    return [...this.services.values()].map((s) => this.toInfo(s));
  }

  get(name: string): ServiceInfo | undefined {
    const s = this.services.get(name);
    return s ? this.toInfo(s) : undefined;
  }

  has(name: string): boolean {
    return this.services.has(name);
  }

  /** 注册或更新一个服务定义（不自动启动） */
  register(input: ServiceSpecInput): ServiceInfo {
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
      this.emitStatus(existing);
      return this.toInfo(existing);
    }
    const svc: ManagedService = {
      spec,
      status: "stopped",
      restarts: 0,
      ring: [],
      intentionalStop: false,
      restarting: false,
      exitWaiters: [],
    };
    this.services.set(spec.name, svc);
    this.system(svc, `registered: ${spec.command}  (cwd: ${spec.cwd})`);
    this.emitStatus(svc);
    return this.toInfo(svc);
  }

  async start(name: string): Promise<ServiceInfo> {
    const svc = this.must(name);
    if (svc.status === "running" || svc.status === "starting") {
      return this.toInfo(svc);
    }
    this.spawnProcess(svc);
    await this.waitSettle(name, SETTLE_MS);
    return this.toInfo(svc);
  }

  async stop(name: string): Promise<ServiceInfo> {
    const svc = this.must(name);
    await this.stopProcess(svc);
    return this.toInfo(svc);
  }

  /**
   * 重启：停止后用最新 spec 重新拉起。
   * 关键点 —— 进程是 daemon 的子进程，重启只换底层进程，
   * 已 attach 的终端（人）保持连接不断。
   */
  async restart(name: string): Promise<ServiceInfo> {
    const svc = this.must(name);
    this.system(svc, "restarting...");
    svc.restarting = true;
    try {
      await this.stopProcess(svc);
      svc.restarts++;
      this.spawnProcess(svc);
      await this.waitSettle(name, SETTLE_MS);
    } finally {
      svc.restarting = false;
      this.emitStatus(svc);
    }
    return this.toInfo(svc);
  }

  async remove(name: string): Promise<void> {
    const svc = this.must(name);
    await this.stopProcess(svc);
    svc.logStream?.end();
    this.services.delete(name);
  }

  /** 返回最近 n 行日志 */
  logs(name: string, lines = 200): LogLine[] {
    const svc = this.must(name);
    return svc.ring.slice(-lines);
  }

  /** 停止全部（daemon 退出时调用） */
  async stopAll(): Promise<void> {
    await Promise.all([...this.services.keys()].map((n) => this.stop(n)));
  }

  // ---- 内部 ----

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
          // 仍存在且未被人为启动才重启
          if (this.services.get(svc.spec.name) === svc && svc.status === "exited") {
            this.spawnProcess(svc);
          }
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

  private ingest(
    svc: ManagedService,
    stream: "stdout" | "stderr",
    chunk: Buffer
  ): void {
    const text = chunk.toString("utf8");
    for (const raw of text.split(/\r?\n/)) {
      if (raw === "") continue;
      this.pushLine(svc, stream, raw);
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
