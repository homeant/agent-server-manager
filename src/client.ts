import net, { Socket } from "node:net";
import { spawn } from "node:child_process";
import fs from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";
import { readFrames, writeFrame } from "./jsonl.js";
import {
  Event,
  Request,
  Response,
  isEvent,
  isResponse,
} from "./protocol.js";
import { DAEMON_LOG, SOCKET_PATH, SVC_HOME } from "./paths.js";

type EventHandler = (ev: Event) => void;

/** 单服务请求超时。restart 最慢包含 SIGTERM 宽限 5s + 稳定窗口 1s。 */
const REQUEST_TIMEOUT_MS = 30_000;
/** 批量请求会分组处理任意数量的服务，给出更宽裕的上限。 */
const BATCH_REQUEST_TIMEOUT_MS = 5 * 60_000;

/** 分发式 Omit：保留联合各成员各自的字段（去掉 id） */
type DistributiveOmit<T, K extends PropertyKey> = T extends unknown
  ? Omit<T, K>
  : never;
type RequestInput = DistributiveOmit<Request, "id">;

export class Client {
  private sock!: Socket;
  private nextId = 1;
  private pending = new Map<number, (res: Response) => void>();
  private eventHandlers = new Set<EventHandler>();
  private closed = false;

  /** 连接到 daemon；若未运行则自动拉起后重试。 */
  static async connect(autoSpawn = true): Promise<Client> {
    const c = new Client();
    try {
      await c.tryConnect();
      return c;
    } catch (err) {
      if (!autoSpawn) throw err;
      await spawnDaemon();
      await c.connectWithRetry(5000);
      return c;
    }
  }

  /**
   * 反复尝试连接直到成功或超时。
   * 不能用「socket 文件是否存在」判断就绪 —— 文件可能是上个 daemon
   * 异常退出的残留，存在但没人监听。
   */
  private async connectWithRetry(timeoutMs: number): Promise<void> {
    const deadline = Date.now() + timeoutMs;
    for (;;) {
      try {
        await this.tryConnect();
        return;
      } catch {
        if (Date.now() >= deadline) throw new Error("等待 daemon 启动超时");
        await new Promise((r) => setTimeout(r, 100));
      }
    }
  }

  private tryConnect(): Promise<void> {
    return new Promise((resolve, reject) => {
      const sock = net.createConnection(SOCKET_PATH);
      sock.once("connect", () => {
        this.sock = sock;
        this.wire();
        resolve();
      });
      sock.once("error", reject);
    });
  }

  private wire(): void {
    readFrames(this.sock, (frame) => {
      if (isResponse(frame)) {
        const cb = this.pending.get(frame.id);
        if (cb) {
          this.pending.delete(frame.id);
          cb(frame);
        }
      } else if (isEvent(frame)) {
        for (const h of this.eventHandlers) h(frame);
      }
    });
    this.sock.on("close", () => {
      this.closed = true;
      for (const cb of this.pending.values())
        cb({ id: -1, ok: false, error: "连接已关闭" });
      this.pending.clear();
    });
  }

  onEvent(handler: EventHandler): () => void {
    this.eventHandlers.add(handler);
    return () => this.eventHandlers.delete(handler);
  }

  request<T = unknown>(req: RequestInput): Promise<T> {
    const id = this.nextId++;
    const full = { ...req, id } as Request;
    return new Promise<T>((resolve, reject) => {
      if (this.closed) return reject(new Error("连接已关闭"));
      const timeout =
        req.type === "startAll" || req.type === "stopAll" || req.type === "removeAll"
          ? BATCH_REQUEST_TIMEOUT_MS
          : REQUEST_TIMEOUT_MS;
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`请求超时（${req.type}）：daemon 无响应`));
      }, REQUEST_TIMEOUT_MS);
      this.pending.set(id, (res) => {
        clearTimeout(timer);
        if (res.ok) resolve(res.result as T);
        else reject(new Error(res.error));
      });
      writeFrame(this.sock, full);
    });
  }

  close(): void {
    this.closed = true;
    this.sock?.end();
  }
}

function daemonEntry(): string {
  // 与本文件同目录结构：dist/client.js -> dist/daemon/index.js
  const here = path.dirname(fileURLToPath(import.meta.url));
  return path.join(here, "daemon", "index.js");
}

async function spawnDaemon(): Promise<void> {
  fs.mkdirSync(SVC_HOME, { recursive: true });
  const out = fs.openSync(DAEMON_LOG, "a");
  const child = spawn(process.execPath, [daemonEntry()], {
    detached: true,
    stdio: ["ignore", out, out, "ipc"],
  });
  await new Promise<void>((resolve) => {
    let settled = false;
    const done = () => {
      if (settled) return;
      settled = true;
      // 断开 IPC channel，否则它会让本进程(CLI)无法退出
      try {
        child.disconnect();
      } catch {
        /* 可能已断开 */
      }
      child.unref();
      resolve();
    };
    child.once("message", (m) => {
      if (m === "ready") done();
    });
    // 兜底：即便没收到 ipc message 也继续，由 waitForSocket 把关
    setTimeout(done, 1500);
  });
}
