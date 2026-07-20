#!/usr/bin/env node
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { Command } from "commander";
import { Client } from "./client.js";
import {
  BatchItemResult,
  BatchResult,
  Event,
  LogLine,
  ServiceInfo,
  ServiceSpecInput,
  ServiceStatus,
} from "./protocol.js";

// dist/cli.js → ../package.json，版本号跟随包本身
const pkg = JSON.parse(
  fs.readFileSync(
    path.join(path.dirname(fileURLToPath(import.meta.url)), "..", "package.json"),
    "utf8"
  )
) as { version: string };

const program = new Command();
program
  .name("asvc")
  .description(
    "本地开发服务管理器。人和 agent 共用同一套命令：asvc start 启动即注册（前台像原始命令，\n" +
      "-d 后台给 agent 用），asvc logs/list/restart/stop 查看与管理。所有操作走同一个 daemon、\n" +
      "按服务名去重，重启服务不会断开你前台/跟随的终端。"
  )
  .version(pkg.version);

// ---- 颜色（仅 TTY 时启用，零依赖）----
const tty = process.stdout.isTTY;
const c = {
  dim: (s: string) => (tty ? `\x1b[2m${s}\x1b[0m` : s),
  red: (s: string) => (tty ? `\x1b[31m${s}\x1b[0m` : s),
  green: (s: string) => (tty ? `\x1b[32m${s}\x1b[0m` : s),
  yellow: (s: string) => (tty ? `\x1b[33m${s}\x1b[0m` : s),
  cyan: (s: string) => (tty ? `\x1b[36m${s}\x1b[0m` : s),
  bold: (s: string) => (tty ? `\x1b[1m${s}\x1b[0m` : s),
};

const stripAnsi = (s: string) => s.replace(/\x1b\[[0-9;]*m/g, "");
const padVisible = (s: string, width: number) =>
  s + " ".repeat(Math.max(0, width - stripAnsi(s).length));

function statusColor(s: ServiceStatus): string {
  switch (s) {
    case "running":
      return c.green(s);
    case "starting":
    case "stopping":
      return c.yellow(s);
    case "exited":
    case "errored":
      return c.red(s);
    default:
      return c.dim(s);
  }
}

function fmtTime(ts: number): string {
  const d = new Date(ts);
  const p = (n: number, len = 2) => String(n).padStart(len, "0");
  return (
    `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ` +
    `${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}.${p(d.getMilliseconds(), 3)}`
  );
}

function fmtUptime(startedAt?: number): string {
  if (!startedAt) return "-";
  const s = Math.floor((Date.now() - startedAt) / 1000);
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m${s % 60}s`;
  const h = Math.floor(m / 60);
  return `${h}h${m % 60}m`;
}

function fmtCpu(percent?: number): string {
  return percent === undefined ? "-" : `${percent.toFixed(1)}%`;
}

function fmtMemory(bytes?: number): string {
  if (bytes === undefined) return "-";
  const mib = bytes / (1024 * 1024);
  return mib < 1024
    ? `${mib.toFixed(mib < 10 ? 1 : 0)}M`
    : `${(mib / 1024).toFixed(1)}G`;
}

/**
 * 路径展示：home 缩成 ~；超长时收缩中间（保头部根段 + 尾部目录名），
 * 头尾都有信息量，比直接截尾更利于一眼认出是哪个项目。
 */
function shortenPath(p: string, max = 36): string {
  const home = os.homedir();
  if (p === home) return "~";
  if (p.startsWith(home + "/")) p = "~" + p.slice(home.length);
  if (p.length <= max) return p;
  const parts = p.split("/");
  // 绝对路径首段是空串（leading /），头部取到第一个真实段，如 "/tmp"
  const headEnd = parts[0] === "" ? 1 : 0;
  const head = parts.slice(0, headEnd + 1).join("/") || "/";
  let tail = "";
  for (let i = parts.length - 1; i > headEnd; i--) {
    const cand = parts[i] + (tail ? "/" + tail : "");
    if (head.length + 3 + cand.length > max) break;
    tail = cand;
  }
  if (!tail) {
    // 末段本身就超长，退化为字符级中间截断
    const half = Math.floor((max - 1) / 2);
    return p.slice(0, max - 1 - half) + "…" + p.slice(-half);
  }
  return `${head}/…/${tail}`;
}

async function withClient<T>(fn: (client: Client) => Promise<T>): Promise<T> {
  const client = await Client.connect();
  try {
    return await fn(client);
  } finally {
    client.close();
  }
}

function fail(msg: string): never {
  console.error(c.red("错误: ") + msg);
  process.exit(1);
}

function usageFail(msg: string): never {
  console.error(c.red("错误: ") + msg);
  process.exit(2);
}

function batchDetail(item: BatchItemResult): string {
  switch (item.outcome) {
    case "started":
      return (
        statusColor(item.info?.status ?? "running") +
        (item.info?.pid ? c.dim(` (pid ${item.info.pid})`) : "")
      );
    case "stopped":
      return statusColor(item.info?.status ?? "stopped");
    case "removed":
      return "已删除注册";
    case "skipped":
      if (item.reason === "already-running") return "已在运行";
      if (item.reason === "already-starting") return "正在启动";
      return "未运行";
    case "failed": {
      const exit = item.info?.lastExitCode;
      return (
        (item.error ?? "操作失败") +
        (exit != null ? c.dim(` (exit ${exit})`) : "")
      );
    }
    default: {
      const _exhaustive: never = item.outcome;
      return _exhaustive;
    }
  }
}

function printBatchResult(result: BatchResult): void {
  if (result.items.length === 0) {
    console.log(c.dim("暂无已注册服务，无需操作。"));
    return;
  }
  const actionLabel =
    result.action === "start" ? "启动" : result.action === "stop" ? "停止" : "删除注册";
  console.log(
    result.action === "remove"
      ? `删除全部注册（${result.items.length}）`
      : `${actionLabel}全部服务（${result.items.length}）`
  );
  console.log();
  for (const item of result.items) {
    const marker =
      item.outcome === "failed"
        ? c.red("✗")
        : item.outcome === "skipped"
          ? c.dim("-")
          : c.green("✓");
    const outcome =
      item.outcome === "started"
        ? "started"
        : item.outcome === "stopped"
          ? "stopped"
          : item.outcome === "removed"
            ? "removed"
            : item.outcome;
    const coloredOutcome =
      item.outcome === "failed"
        ? c.red(outcome)
        : item.outcome === "skipped"
          ? c.dim(outcome)
          : c.green(outcome);
    console.log(
      `${marker} ${item.name.padEnd(16)} ${padVisible(coloredOutcome, 10)} ${batchDetail(item)}`
    );
  }
  const succeeded = result.items.filter((i) =>
    ["started", "stopped", "removed"].includes(i.outcome)
  ).length;
  const skipped = result.items.filter((i) => i.outcome === "skipped").length;
  const failed = result.items.filter((i) => i.outcome === "failed").length;
  console.log();
  console.log(`结果：成功 ${succeeded}，跳过 ${skipped}，失败 ${failed}`);
  if (failed > 0) process.exitCode = 1;
}

/**
 * 上报一次启动/重启结果。daemon 已等过稳定窗口：
 * - 进程在跑 → 打印成功，退出码 0。
 * - 进程没起来（典型：端口被占 EADDRINUSE）→ 打印最近日志到 stderr，
 *   给出端口提示，并把退出码置为 1，让走 Bash 的 agent 直接判失败。
 */
async function reportStart(
  client: Client,
  info: ServiceInfo,
  verb: string
): Promise<void> {
  if (info.status === "running" || info.status === "starting") {
    console.log(
      `${c.cyan(info.name)} ${verb} → ${statusColor(info.status)}` +
        (info.pid ? c.dim(` (pid ${info.pid})`) : "")
    );
    return;
  }
  const logs = await client
    .request<LogLine[]>({ type: "logs", name: info.name, lines: 20 })
    .catch(() => [] as LogLine[]);
  console.error(
    c.red(
      `${info.name} ${verb}失败 → ${info.status}` +
        (info.lastExitCode != null ? ` (exit ${info.lastExitCode})` : "")
    )
  );
  console.error(c.dim("最近日志："));
  for (const l of logs) {
    const body =
      l.stream === "stderr" ? c.red(l.line) : l.stream === "system" ? c.yellow(l.line) : l.line;
    console.error("  " + body);
  }
  if (logs.some((l) => /EADDRINUSE|address already in use/i.test(l.line))) {
    console.error(
      c.yellow("端口疑似被占用（EADDRINUSE）：请换端口，或先停掉占用该端口的进程后重试。")
    );
  }
  process.exitCode = 1;
}

/** 把命令行选项组装成服务定义；同时校验 env/port 格式。 */
function buildSpec(
  name: string,
  cmd: string,
  opts: { cwd: string; port?: string; env?: string[]; autorestart?: boolean }
): ServiceSpecInput {
  const env: Record<string, string> = {};
  for (const kv of opts.env ?? []) {
    const i = kv.indexOf("=");
    if (i < 0) fail(`环境变量格式应为 KEY=VAL: ${kv}`);
    env[kv.slice(0, i)] = kv.slice(i + 1);
  }
  const port = opts.port ? parseInt(opts.port, 10) : undefined;
  if (opts.port && Number.isNaN(port)) fail(`端口不是数字: ${opts.port}`);
  return {
    name,
    command: cmd,
    cwd: opts.cwd,
    env: Object.keys(env).length ? env : undefined,
    port,
    autorestart: opts.autorestart,
  };
}

/**
 * 前台运行：像直接敲原始命令一样，占住终端实时刷日志。
 * - Ctrl-C → 停止服务并退出（和原生命令一致）。
 * - 服务自己崩溃/被停 → 前台退出（崩溃用其退出码）。
 * - 被 agent `asvc restart` → 终端不退出，看到 restart 标记后新日志继续。
 */
async function startForeground(
  name: string,
  spec?: ServiceSpecInput
): Promise<void> {
  const client = await Client.connect();
  let finished = false;
  const finish = (code: number) => {
    if (finished) return;
    finished = true;
    client.close();
    process.exit(code);
  };

  // 先订阅事件，确保不漏掉启动后的日志
  client.onEvent((ev: Event) => {
    if (ev.event === "log" && ev.name === name) {
      printLogLine(ev);
      return;
    }
    if (ev.event !== "status" || ev.info.name !== name) return;
    const s = ev.info.status;
    printStatusLine(ev.info);
    if (s === "running" || s === "starting" || s === "stopping") return;
    // exited / errored / stopped
    if (ev.info.restarting) return; // agent 正在重启，别退出
    if (s === "exited" && ev.info.autorestart) return; // 等自动重启
    finish(s === "exited" ? ev.info.lastExitCode ?? 1 : 0);
  });

  // 已注册则更新定义（先不启动，待 attach 订阅后再启动，避免漏日志）
  if (spec) await client.request({ type: "register", spec, start: false });

  // attach：补历史日志 + 拿当前状态
  const { info, backlog } = await client.request<{
    info: ServiceInfo;
    backlog: LogLine[];
  }>({ type: "attach", name, backlog: 500 });
  for (const l of backlog) printLogLine(l);

  // 启动（已在跑则无操作）。daemon 内部会等稳定窗口。
  const started = await client.request<ServiceInfo>({ type: "start", name });
  void info;

  // 启动即失败（如端口被占）→ 打印提示并以非零码退出
  if (started.status === "exited" || started.status === "errored") {
    if (backlog.some((l) => /EADDRINUSE|address already in use/i.test(l.line))) {
      console.error(
        c.yellow("端口疑似被占用（EADDRINUSE）：请换端口，或先停掉占用该端口的进程后重试。")
      );
    }
    finish(started.lastExitCode ?? 1);
    return;
  }

  console.error(c.dim(`— ${name} 前台运行中（Ctrl-C 停止服务）—`));
  process.on("SIGINT", async () => {
    console.error(c.dim("\n正在停止服务…"));
    try {
      await client.request({ type: "stop", name });
    } catch {
      /* ignore */
    }
    finish(0);
  });
  await new Promise(() => {}); // 阻塞，保持前台
}

// ---- list ----
program
  .command("list")
  .alias("ls")
  .description("列出所有服务及状态")
  .action(async () => {
    await withClient(async (client) => {
      const list = await client.request<ServiceInfo[]>({ type: "list" });
      if (list.length === 0) {
        console.log(c.dim('（暂无服务。用 asvc start <name> -c "<命令>" 启动一个）'));
        return;
      }
      const rows = list.map((s) => ({
        NAME: s.name,
        STATUS: statusColor(s.status),
        PID: s.pid ? String(s.pid) : "-",
        CPU: fmtCpu(s.cpuPercent),
        MEMORY: fmtMemory(s.memoryBytes),
        UPTIME: s.status === "running" ? fmtUptime(s.startedAt) : "-",
        PORT: s.port ? String(s.port) : "-",
        AUTO_RESTARTS: String(s.restarts),
        CWD: shortenPath(s.cwd),
        COMMAND: s.command.length > 40 ? s.command.slice(0, 39) + "…" : s.command,
      }));
      printTable(rows);
    });
  });

// ---- logs ----
program
  .command("logs <name>")
  .description("查看服务日志")
  .option("-f, --follow", "持续跟随（服务被 agent 重启也不会断开）")
  .option("-n, --lines <n>", "显示最近 N 行", "200")
  .action(async (name: string, opts: { follow?: boolean; lines: string }) => {
    const lines = parseInt(opts.lines, 10) || 200;
    const client = await Client.connect();
    try {
      if (!opts.follow) {
        const log = await client.request<LogLine[]>({ type: "logs", name, lines });
        for (const l of log) printLogLine(l);
        client.close();
        return;
      }
      // follow: 先订阅事件，再 attach 拿 backlog，之后流式
      client.onEvent((ev: Event) => {
        if (ev.event === "log" && ev.name === name) printLogLine(ev);
        else if (ev.event === "status" && ev.info.name === name)
          printStatusLine(ev.info);
      });
      const { backlog } = await client.request<{
        info: ServiceInfo;
        backlog: LogLine[];
      }>({ type: "attach", name, backlog: lines });
      for (const l of backlog) printLogLine(l);
      console.error(c.dim(`— 跟随中（Ctrl-C 退出，不影响服务）—`));
      // 保持进程存活
      process.on("SIGINT", () => {
        client.close();
        process.exit(0);
      });
      await new Promise(() => {});
    } catch (err) {
      client.close();
      fail(err instanceof Error ? err.message : String(err));
    }
  });

// ---- restart ----
program
  .command("restart <name>")
  .description("重启服务（停掉旧进程并用最新定义重新拉起）")
  .action(async (name: string) => {
    await withClient(async (client) => {
      const info = await client.request<ServiceInfo>({ type: "restart", name });
      await reportStart(client, info, "重启");
    }).catch((e) => fail(e.message));
  });

// ---- stop ----
program
  .command("stop [name]")
  .description("关闭服务（进程停止，定义保留，可再启动/重启）")
  .option("-a, --all", "停止所有已注册服务")
  .action(async (name: string | undefined, opts: { all?: boolean }) => {
    if (opts.all && name) usageFail("服务名与 --all 不能同时使用");
    if (!opts.all && !name) usageFail("请提供服务名，或使用 --all 停止全部服务");
    if (opts.all) {
      await withClient(async (client) => {
        const result = await client.request<BatchResult>({ type: "stopAll" });
        printBatchResult(result);
      }).catch((e) => fail(e.message));
      return;
    }
    const serviceName = name!;
    await withClient(async (client) => {
      const info = await client.request<ServiceInfo>({ type: "stop", name: serviceName });
      console.log(`${c.cyan(info.name)} 已停止 → ${statusColor(info.status)}`);
    }).catch((e) => fail(e.message));
  });

// ---- start（启动即注册：给命令就自动注册并启动）----
program
  .command("start [name]")
  .description(
    "启动服务（启动即注册：带 -c 给命令就自动注册，按名去重不会重复拉起）。\n" +
      "默认前台运行、实时刷日志，像直接敲原始命令，Ctrl-C 停止服务。\n" +
      "加 -d/--detach 则后台启动、跑完即返回带退出码（agent 用这个）。\n" +
      "已注册过的服务再次启动可省略 -c。"
  )
  .option("-c, --cmd <command>", '启动命令，经 shell 执行，如 "npm run dev"')
  .option("-w, --cwd <dir>", "工作目录，默认当前目录", process.cwd())
  .option("-p, --port <port>", "服务端口（仅用于展示/排查）")
  .option("-e, --env <kv...>", "环境变量，形如 KEY=VAL，可多个")
  .option("--autorestart", "进程异常退出时自动重启")
  .option("-d, --detach", "后台启动并立即返回（带退出码），不占用终端")
  .option("-a, --all", "批量启动所有已注册服务（始终后台运行）")
  .action(
    async (
      name: string | undefined,
      opts: {
        cmd?: string;
        cwd: string;
        port?: string;
        env?: string[];
        autorestart?: boolean;
        detach?: boolean;
        all?: boolean;
      },
      command: Command
    ) => {
      if (opts.all && name) usageFail("服务名与 --all 不能同时使用");
      if (!opts.all && !name) usageFail("请提供服务名，或使用 --all 启动全部服务");
      if (opts.all) {
        const hasDefinitionFlags =
          opts.cmd !== undefined ||
          opts.port !== undefined ||
          opts.env !== undefined ||
          opts.autorestart !== undefined ||
          command.getOptionValueSource("cwd") === "cli";
        if (hasDefinitionFlags) {
          usageFail("--all 不能与 -c/--cmd、-w/--cwd、-p/--port、-e/--env 或 --autorestart 同时使用");
        }
        await withClient(async (client) => {
          const result = await client.request<BatchResult>({ type: "startAll" });
          printBatchResult(result);
        }).catch((e) => fail(e.message));
        return;
      }
      const serviceName = name!;
      const spec = opts.cmd ? buildSpec(serviceName, opts.cmd, opts) : undefined;
      const onErr = (e: Error) => {
        if (!spec && /未知服务/.test(e.message)) {
          fail(
            `${e.message}\n该服务尚未注册。首次启动请用 -c 指定命令，如：` +
              `asvc start ${serviceName} -c "npm run dev"`
          );
        }
        fail(e.message);
      };
      if (opts.detach) {
        // 后台：注册（若给了命令）并启动，跑完返回退出码
        await withClient(async (client) => {
          const info = spec
            ? await client.request<ServiceInfo>({ type: "register", spec, start: true })
            : await client.request<ServiceInfo>({ type: "start", name: serviceName });
          await reportStart(client, info, "启动");
        }).catch(onErr);
        return;
      }
      // 前台：像原始命令一样实时刷日志，Ctrl-C 停服务
      await startForeground(serviceName, spec).catch(onErr);
    }
  );

// ---- rm ----
program
  .command("rm [name]")
  .alias("remove")
  .description("移除服务（先停止再从注册表删除）")
  .option("-a, --all", "删除所有服务注册（运行中的服务会先停止）")
  .option("-y, --yes", "确认删除全部注册，仅与 --all 一起使用")
  .action(async (name: string | undefined, opts: { all?: boolean; yes?: boolean }) => {
    if (opts.all && name) usageFail("服务名与 --all 不能同时使用");
    if (!opts.all && !name) usageFail("请提供服务名，或使用 --all 删除全部注册");
    if (opts.yes && !opts.all) usageFail("--yes 只能与 --all 一起使用");
    if (opts.all) {
      await withClient(async (client) => {
        if (!opts.yes) {
          const list = await client.request<ServiceInfo[]>({ type: "list" });
          if (list.length === 0) {
            console.log(c.dim("暂无已注册服务，无需操作。"));
            return;
          }
          console.error(`即将停止并删除 ${list.length} 个已注册服务。`);
          console.error("服务日志不会删除。");
          console.error(`确认执行请使用：${c.bold("asvc rm --all --yes")}`);
          process.exitCode = 2;
          return;
        }
        const result = await client.request<BatchResult>({ type: "removeAll" });
        printBatchResult(result);
      }).catch((e) => fail(e.message));
      return;
    }
    const serviceName = name!;
    await withClient(async (client) => {
      await client.request({ type: "remove", name: serviceName });
      console.log(`${c.cyan(serviceName)} 已移除`);
    }).catch((e) => fail(e.message));
  });

// ---- completion ----

const COMPLETION_ZSH = `
_asvc() {
  local -a _asvc_cmds
  _asvc_cmds=(
    'start:启动服务（前台；-d 后台；首次需 -c）'
    'stop:停止服务'
    'restart:重启服务'
    'logs:查看日志'
    'list:列出所有服务'
    'rm:移除服务'
    'daemon:管理后台 daemon'
  )
  if (( CURRENT == 2 )); then
    _describe -t commands 'asvc command' _asvc_cmds
    return
  fi
  local cmd=\${words[2]}
  case \$cmd in
    start|stop|restart|logs|rm|remove)
      if (( CURRENT == 3 )); then
        local -a _svcs
        _svcs=( \${(f)"\$(command asvc __complete services 2>/dev/null)"} )
        [[ \$cmd == start || \$cmd == stop || \$cmd == rm || \$cmd == remove ]] && _svcs+=(--all)
        (( \${#_svcs} )) && _describe -t services 'service' _svcs
        return
      fi
      ;;
    daemon)
      (( CURRENT == 3 )) && _values 'daemon command' status stop
      return
      ;;
    completion)
      (( CURRENT == 3 )) && _values 'shell' zsh bash
      return
      ;;
  esac
  case \$cmd in
    start)
      _arguments \\
        '(-c --cmd)'{-c,--cmd}'[启动命令（经 shell 执行）]:command:' \\
        '(-w --cwd)'{-w,--cwd}'[工作目录]:dir:_files -/' \\
        '(-p --port)'{-p,--port}'[服务端口]:port:' \\
        '*'{-e,--env}'[环境变量 KEY=VAL]:env:' \\
        '--autorestart[进程异常退出时自动重启]' \\
        '(-d --detach)'{-d,--detach}'[后台启动并立即返回]' \\
        '(-a --all)'{-a,--all}'[批量启动所有已注册服务]'
      ;;
    stop)
      _arguments '(-a --all)'{-a,--all}'[停止所有已注册服务]'
      ;;
    rm|remove)
      _arguments \\
        '(-a --all)'{-a,--all}'[删除所有服务注册]' \\
        '(-y --yes)'{-y,--yes}'[确认删除全部注册]'
      ;;
    logs)
      _arguments \\
        '(-f --follow)'{-f,--follow}'[持续跟随]' \\
        '(-n --lines)'{-n,--lines}'[显示最近 N 行]:lines:'
      ;;
  esac
}
compdef _asvc asvc
`.trim();

const COMPLETION_BASH = `
_asvc() {
  local cur="\${COMP_WORDS[COMP_CWORD]}"
  if [ "\$COMP_CWORD" -eq 1 ]; then
    COMPREPLY=( \$(compgen -W "start stop restart logs list ls rm daemon" -- "\$cur") )
    return
  fi
  local cmd="\${COMP_WORDS[1]}"
  case "\$cmd" in
    start|stop|restart|logs|rm|remove)
      if [ "\$COMP_CWORD" -eq 2 ]; then
        local choices="\$(asvc __complete services 2>/dev/null)"
        case "\$cmd" in
          start|stop|rm|remove) choices="\$choices --all" ;;
        esac
        COMPREPLY=( \$(compgen -W "\$choices" -- "\$cur") )
      elif [ "\$cmd" = "rm" ] || [ "\$cmd" = "remove" ]; then
        COMPREPLY=( \$(compgen -W "--yes" -- "\$cur") )
      fi
      ;;
    daemon)
      [ "\$COMP_CWORD" -eq 2 ] && COMPREPLY=( \$(compgen -W "status stop" -- "\$cur") )
      ;;
    completion)
      [ "\$COMP_CWORD" -eq 2 ] && COMPREPLY=( \$(compgen -W "zsh bash" -- "\$cur") )
      ;;
  esac
}
complete -F _asvc asvc
`.trim();

program
  .command("completion <shell>")
  .description('输出 shell 补全脚本。zsh: 在 ~/.zshrc 加 eval "$(asvc completion zsh)"；bash 同理')
  .action((shell: string) => {
    if (shell === "zsh") console.log(COMPLETION_ZSH);
    else if (shell === "bash") console.log(COMPLETION_BASH);
    else fail(`不支持的 shell: ${shell}（支持 zsh / bash）`);
  });

// 补全脚本的回调入口：列出已注册服务名（daemon 未运行则静默输出空，绝不 auto-spawn）
program
  .command("__complete <what>", { hidden: true })
  .action(async (what: string) => {
    if (what !== "services") return;
    try {
      const client = await Client.connect(false);
      const list = await client.request<ServiceInfo[]>({ type: "list" });
      client.close();
      for (const s of list) console.log(s.name);
    } catch {
      /* daemon 未运行 */
    }
  });

// ---- daemon 子命令 ----
const daemon = program.command("daemon").description("管理后台 daemon");
daemon
  .command("status")
  .description("检查 daemon 是否在运行")
  .action(async () => {
    try {
      await withClient(async (client) => {
        await client.request({ type: "ping" });
        console.log(c.green("daemon 运行中"));
      });
    } catch {
      console.log(c.red("daemon 未运行"));
    }
  });
daemon
  .command("stop")
  .description("停止 daemon（会先关闭所有服务）")
  .action(async () => {
    try {
      const client = await Client.connect(false);
      await client.request({ type: "shutdown" });
      client.close();
      console.log(c.yellow("daemon 已停止（所有服务已关闭）"));
    } catch {
      console.log(c.dim("daemon 未运行"));
    }
  });

program.parseAsync(process.argv);

// ---- 渲染辅助 ----

function printLogLine(l: LogLine): void {
  const ts = c.dim(fmtTime(l.ts));
  let body: string;
  if (l.stream === "system") body = c.yellow(`» ${l.line}`);
  else if (l.stream === "stderr") body = c.red(l.line);
  else body = l.line;
  console.log(`${ts} ${body}`);
}

function printStatusLine(info: ServiceInfo): void {
  console.log(c.dim(fmtTime(Date.now())) + " " + c.cyan(`[${info.name}] `) + statusColor(info.status));
}

function printTable(rows: Record<string, string>[]): void {
  const cols = Object.keys(rows[0]);
  const width: Record<string, number> = {};
  for (const col of cols) {
    width[col] = Math.max(col.length, ...rows.map((r) => stripAnsi(r[col]).length));
  }
  console.log(c.bold(cols.map((col) => padVisible(col, width[col])).join("  ")));
  for (const r of rows) {
    console.log(cols.map((col) => padVisible(r[col], width[col])).join("  "));
  }
}
