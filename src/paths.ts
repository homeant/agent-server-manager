import os from "node:os";
import path from "node:path";

/**
 * daemon 的家目录。默认 ~/.server-manager，可用 SVC_HOME 覆盖。
 * 全局单实例：一台机器一个 daemon，便于跨项目去重、避免端口冲突。
 */
export const SVC_HOME = process.env.SVC_HOME
  ? path.resolve(process.env.SVC_HOME)
  : path.join(os.homedir(), ".server-manager");

export const SOCKET_PATH =
  process.env.SVC_SOCKET || path.join(SVC_HOME, "daemon.sock");

export const PID_FILE = path.join(SVC_HOME, "daemon.pid");
export const LOG_DIR = path.join(SVC_HOME, "logs");
export const DAEMON_LOG = path.join(SVC_HOME, "daemon.log");
