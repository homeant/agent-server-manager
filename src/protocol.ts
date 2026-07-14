/**
 * IPC 协议：daemon 与各 asvc CLI 客户端之间通过 unix domain socket
 * 传输 newline-delimited JSON（每行一条消息）。
 *
 * - 请求/响应：客户端发 Request（带 id），daemon 回 Response（同 id）。
 * - 事件推送：daemon 主动向已 attach 的连接推 Event（无 id）。
 */

export type ServiceStatus =
  | "stopped" // 已注册但未运行
  | "starting" // 正在拉起
  | "running" // 运行中
  | "stopping" // 正在停止
  | "exited" // 进程自行退出（含正常/异常）
  | "errored"; // 拉起失败

/** 服务定义 —— 动态注册，无需配置文件 */
export interface ServiceSpec {
  name: string;
  /** 通过 shell 执行的命令，例如 "npm run dev" */
  command: string;
  /** 工作目录，默认注册时客户端所在目录 */
  cwd: string;
  /** 追加的环境变量 */
  env?: Record<string, string>;
  /** 声明的端口（仅用于展示与冲突提示，可选） */
  port?: number;
  /** 进程异常退出时是否自动重启，默认 false */
  autorestart?: boolean;
}

/** 服务运行时状态（list / 事件中返回） */
export interface ServiceInfo extends ServiceSpec {
  status: ServiceStatus;
  pid?: number;
  /** 本次启动的时间戳(ms)，running 时有效 */
  startedAt?: number;
  /** 最近一次退出码 */
  lastExitCode?: number | null;
  lastExitSignal?: string | null;
  /** 程序自动重启次数（仅统计 --autorestart 触发的重启） */
  restarts: number;
  /** 是否正处于「重启中」（停旧进程→起新进程之间）。
   *  前台 attach 的客户端据此区分「服务真退出了」与「只是被重启」。 */
  restarting?: boolean;
}

export interface LogLine {
  name: string;
  stream: "stdout" | "stderr" | "system";
  line: string;
  ts: number;
}

// ---- 请求 ----

export type Request =
  | { id: number; type: "ping" }
  | { id: number; type: "list" }
  | { id: number; type: "register"; spec: ServiceSpecInput; start?: boolean }
  | { id: number; type: "start"; name: string }
  | { id: number; type: "stop"; name: string }
  | { id: number; type: "restart"; name: string }
  | { id: number; type: "remove"; name: string }
  | { id: number; type: "logs"; name: string; lines?: number }
  | { id: number; type: "attach"; name: string; backlog?: number }
  | { id: number; type: "detach"; name: string }
  | { id: number; type: "shutdown" };

/** register 时允许省略 cwd（由 daemon 用客户端传入值），其余同 ServiceSpec */
export interface ServiceSpecInput {
  name: string;
  command: string;
  cwd: string;
  env?: Record<string, string>;
  port?: number;
  autorestart?: boolean;
}

// ---- 响应 ----

export interface ResponseOk<T = unknown> {
  id: number;
  ok: true;
  result: T;
}
export interface ResponseErr {
  id: number;
  ok: false;
  error: string;
}
export type Response<T = unknown> = ResponseOk<T> | ResponseErr;

// ---- 事件（daemon → 已 attach 的客户端）----

export type Event =
  | ({ event: "log" } & LogLine)
  | { event: "status"; info: ServiceInfo };

export type Frame = Request | Response | Event;

export function isRequest(f: Frame): f is Request {
  return "type" in f && "id" in f;
}
export function isResponse(f: Frame): f is Response {
  return "ok" in f && "id" in f;
}
export function isEvent(f: Frame): f is Event {
  return "event" in f;
}
