export type ServiceStatus =
  | "stopped"
  | "starting"
  | "running"
  | "stopping"
  | "exited"
  | "errored";

export type LogStream = "stdout" | "stderr" | "system";

export interface ServiceSpec {
  name: string;
  command: string;
  cwd: string;
  env?: Record<string, string>;
  port?: number;
  autorestart: boolean;
}

export interface ServiceInfo extends ServiceSpec {
  status: ServiceStatus;
  pid?: number;
  cpuPercent?: number;
  memoryBytes?: number;
  startedAt?: number;
  lastExitCode?: number;
  lastExitSignal?: string;
  restarts: number;
  restarting?: boolean;
}

export interface LogLine {
  name: string;
  stream: LogStream;
  line: string;
  ts: number;
}

export interface BatchItemResult {
  name: string;
  outcome: string;
  info?: ServiceInfo;
  reason?: string;
  error?: string;
}

export interface BatchResult {
  action: string;
  items: BatchItemResult[];
}
