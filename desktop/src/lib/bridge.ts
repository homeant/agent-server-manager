import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type {
  BatchResult,
  CliInstallStatus,
  LogLine,
  ServiceInfo,
  ServiceSpec,
} from "./types";

const native = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
const nativeWindow = native ? getCurrentWindow() : undefined;

const mockServices: ServiceInfo[] = [
  {
    name: "valuz-v10-cloud",
    command: "./scripts/dev.sh cloud",
    cwd: "/Users/tianhui/Project/xiaobang/valuz",
    status: "running",
    pid: 60257,
    cpuPercent: 1.7,
    memoryBytes: 188_000_000,
    startedAt: Date.now() - 1000 * 60 * 44,
    restarts: 0,
    autorestart: false,
    port: 8010,
  },
  {
    name: "valuz-v10-webui",
    command: "./scripts/dev.sh --edition commercial webui",
    cwd: "/Users/tianhui/Project/xiaobang/valuz",
    status: "stopped",
    restarts: 0,
    autorestart: false,
    port: 3000,
  },
  {
    name: "valuz-v10-desktop",
    command: "./scripts/dev.sh --edition commercial frontend",
    cwd: "/Users/tianhui/Project/xiaobang/valuz",
    status: "stopped",
    restarts: 0,
    autorestart: false,
  },
  {
    name: "valuz-v10-all",
    command: "./scripts/dev.sh --edition commercial all",
    cwd: "/Users/tianhui/Project/xiaobang/valuz",
    status: "stopped",
    restarts: 0,
    autorestart: false,
  },
  {
    name: "valuz-v10-finance",
    command: "VALUZ_ENV=dev ./scripts/dev.sh --edition finance all",
    cwd: "/Users/tianhui/Project/xiaobang/valuz",
    status: "stopped",
    restarts: 0,
    autorestart: false,
  },
  {
    name: "valuz-v10-docker",
    command: "docker compose -f docker/cloud-runtime.yml up",
    cwd: "/Users/tianhui/Project/xiaobang/valuz",
    status: "running",
    pid: 60711,
    cpuPercent: 0.4,
    memoryBytes: 92_000_000,
    startedAt: Date.now() - 1000 * 60 * 13,
    restarts: 0,
    autorestart: false,
  },
  {
    name: "valuz-v10-local-runtime",
    command: "VALUZ_ENV=dev VALUZ_SERVER_BASE_URL=http://127.0.0.1:8001 ./scripts/dev.sh finance backend",
    cwd: "/Users/tianhui/Project/xiaobang/valuz",
    status: "running",
    pid: 60822,
    cpuPercent: 4.2,
    memoryBytes: 326_000_000,
    startedAt: Date.now() - 1000 * 60 * 8,
    restarts: 1,
    autorestart: true,
    port: 8001,
  },
  {
    name: "valuz-v10-desktop-app",
    command: "VALUZ_ENV=dev ./scripts/dev.sh finance frontend",
    cwd: "/Users/tianhui/Project/xiaobang/valuz",
    status: "running",
    pid: 60918,
    cpuPercent: 2.4,
    memoryBytes: 246_000_000,
    startedAt: Date.now() - 1000 * 60 * 3,
    restarts: 0,
    autorestart: false,
    port: 1420,
  },
];

const mockLogs: LogLine[] = [
  { name: "valuz-v10-cloud", stream: "system", line: "service started · pid 60257", ts: Date.now() - 1000 * 60 * 44 },
  { name: "valuz-v10-cloud", stream: "stdout", line: "cloud runtime listening on http://127.0.0.1:8010", ts: Date.now() - 1000 * 60 * 43 },
  { name: "valuz-v10-cloud", stream: "stdout", line: "connected to local development database", ts: Date.now() - 1000 * 12 },
  { name: "valuz-v10-cloud", stream: "stdout", line: "health check passed", ts: Date.now() - 1000 * 4 },
];

function fake<T>(value: T): Promise<T> {
  return new Promise((resolve) => window.setTimeout(() => resolve(value), 120));
}

async function call<T>(command: string, args: Record<string, unknown> = {}, fallback: T): Promise<T> {
  if (!native) {
    return fake(fallback);
  }
  return invoke<T>(command, args);
}

export const api = {
  getServices: () => call<ServiceInfo[]>("get_services", {}, mockServices),
  getLogs: (name: string, lines = 240) =>
    call<LogLine[]>("get_logs", { name, lines }, mockLogs.filter((log) => log.name === name)),
  daemonStatus: () => call<boolean>("daemon_status", {}, true),
  setLocale: (locale: "en" | "zh-CN") => call<void>("set_locale", { locale }, undefined),
  cliInstallStatus: () => call<CliInstallStatus>("cli_install_status", {}, {
    supported: true,
    installed: false,
    path: "/usr/local/bin/asvc",
  }),
  installCli: () => call<CliInstallStatus>("install_cli", {}, {
    supported: true,
    installed: true,
    path: "/usr/local/bin/asvc",
  }),
  startDragging: () => nativeWindow ? nativeWindow.startDragging() : fake(undefined),
  toggleMaximize: () => nativeWindow ? nativeWindow.toggleMaximize() : fake(undefined),
  startService: (name: string) => call<ServiceInfo>("start_service", { name }, mockService(name, "running")),
  stopService: (name: string) => call<ServiceInfo>("stop_service", { name }, mockService(name, "stopped")),
  restartService: (name: string) => call<ServiceInfo>("restart_service", { name }, mockService(name, "running")),
  startAll: () => call<BatchResult>("start_all", {}, batch("start")),
  stopAll: () => call<BatchResult>("stop_all", {}, batch("stop")),
  removeService: (name: string) => call<Record<string, string>>("remove_service", { name }, { removed: name }),
  registerService: (spec: ServiceSpec) => call<ServiceInfo>("register_service", { spec }, { ...spec, status: "running", restarts: 0 }),
};

function mockService(name: string, status: ServiceInfo["status"]): ServiceInfo {
  const service = mockServices.find((item) => item.name === name) ?? {
    name,
    command: "",
    cwd: "",
    restarts: 0,
    autorestart: false,
  };
  return { ...service, status };
}

function batch(action: string): BatchResult {
  return {
    action,
    items: mockServices.map((service) => ({
      name: service.name,
      outcome: action === "start" ? "started" : "stopped",
      info: { ...service, status: action === "start" ? "running" : "stopped" },
    })),
  };
}
