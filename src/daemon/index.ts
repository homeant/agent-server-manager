#!/usr/bin/env node
import net, { Socket } from "node:net";
import fs from "node:fs";
import { Supervisor } from "./supervisor.js";
import { readFrames, writeFrame } from "../jsonl.js";
import { Request, Response, Event } from "../protocol.js";
import { PID_FILE, SOCKET_PATH, SVC_HOME } from "../paths.js";

fs.mkdirSync(SVC_HOME, { recursive: true });

const sup = new Supervisor();

/** 每个连接记录它 attach 了哪些服务（用于过滤事件推送） */
const attachments = new Map<Socket, Set<string>>();

// 把 supervisor 的事件广播给关心的连接
sup.on("log", (line) => {
  const ev: Event = { event: "log", ...line };
  for (const [sock, names] of attachments) {
    if (names.has(line.name)) writeFrame(sock, ev);
  }
});
sup.on("status", (info) => {
  const ev: Event = { event: "status", info };
  for (const [sock, names] of attachments) {
    if (names.has(info.name)) writeFrame(sock, ev);
  }
});

function reply(sock: Socket, res: Response): void {
  writeFrame(sock, res);
}

async function handle(sock: Socket, req: Request): Promise<void> {
  try {
    switch (req.type) {
      case "ping":
        return reply(sock, { id: req.id, ok: true, result: { pong: true } });
      case "list":
        return reply(sock, { id: req.id, ok: true, result: sup.list() });
      case "register": {
        const info = await sup.register(req.spec, req.start);
        return reply(sock, { id: req.id, ok: true, result: info });
      }
      case "start":
        return reply(sock, { id: req.id, ok: true, result: await sup.start(req.name) });
      case "startAll":
        return reply(sock, { id: req.id, ok: true, result: await sup.startAll() });
      case "stop":
        return reply(sock, { id: req.id, ok: true, result: await sup.stop(req.name) });
      case "stopAll":
        return reply(sock, { id: req.id, ok: true, result: await sup.stopAll() });
      case "restart":
        return reply(sock, { id: req.id, ok: true, result: await sup.restart(req.name) });
      case "remove":
        await sup.remove(req.name);
        return reply(sock, { id: req.id, ok: true, result: { removed: req.name } });
      case "removeAll":
        return reply(sock, { id: req.id, ok: true, result: await sup.removeAll() });
      case "logs":
        return reply(sock, { id: req.id, ok: true, result: sup.logs(req.name, req.lines) });
      case "attach": {
        if (!sup.has(req.name)) throw new Error(`未知服务: ${req.name}`);
        let set = attachments.get(sock);
        if (!set) attachments.set(sock, (set = new Set()));
        set.add(req.name);
        // 先把状态 + 历史日志补给客户端，再开始流式
        const info = sup.get(req.name)!;
        const backlog = sup.logs(req.name, req.backlog ?? 200);
        return reply(sock, { id: req.id, ok: true, result: { info, backlog } });
      }
      case "detach": {
        attachments.get(sock)?.delete(req.name);
        return reply(sock, { id: req.id, ok: true, result: { detached: req.name } });
      }
      case "shutdown": {
        reply(sock, { id: req.id, ok: true, result: { shuttingDown: true } });
        await shutdown(0);
        return;
      }
      default: {
        const _exhaustive: never = req;
        void _exhaustive;
        return reply(sock, { id: (req as Request).id, ok: false, error: "未知请求类型" });
      }
    }
  } catch (err) {
    reply(sock, {
      id: req.id,
      ok: false,
      error: err instanceof Error ? err.message : String(err),
    });
  }
}

const server = net.createServer((sock) => {
  attachments.set(sock, new Set());
  readFrames(sock, (frame) => {
    if ("type" in frame && "id" in frame) void handle(sock, frame as Request);
  });
  const cleanup = () => attachments.delete(sock);
  sock.on("close", cleanup);
  sock.on("error", cleanup);
});

async function shutdown(code: number): Promise<void> {
  try {
    await sup.stopAll();
  } catch {
    /* ignore */
  }
  try {
    server.close();
  } catch {
    /* ignore */
  }
  try {
    fs.unlinkSync(SOCKET_PATH);
  } catch {
    /* ignore */
  }
  try {
    fs.unlinkSync(PID_FILE);
  } catch {
    /* ignore */
  }
  process.exit(code);
}

/**
 * 单实例保护：先试着连一下 socket。
 * - 连得上 → 已有活着的 daemon，本进程直接退出（不能删活 socket 抢管，
 *   否则旧 daemon 和它的子进程全部失管）。
 * - 连不上 → socket 是上次异常退出的残留，清掉再监听。
 */
function checkExistingDaemon(): Promise<boolean> {
  return new Promise((resolve) => {
    const probe = net.createConnection(SOCKET_PATH);
    probe.once("connect", () => {
      probe.destroy();
      resolve(true);
    });
    probe.once("error", () => resolve(false));
  });
}

void (async () => {
  if (await checkExistingDaemon()) {
    console.log("[asvc-daemon] 已有 daemon 在运行，本进程退出");
    // auto-spawn 的父进程在等 ready：现存 daemon 即「就绪」，让它直接去连
    if (process.send) process.send("ready");
    process.exit(0);
  }
  try {
    fs.unlinkSync(SOCKET_PATH);
  } catch {
    /* 不存在 */
  }
  server.listen(SOCKET_PATH, () => {
    fs.writeFileSync(PID_FILE, String(process.pid));
    // 通知父进程（auto-spawn 时）daemon 已就绪
    if (process.send) process.send("ready");
    console.log(`[asvc-daemon] listening on ${SOCKET_PATH} (pid ${process.pid})`);
  });
})();

server.on("error", (err) => {
  console.error("[asvc-daemon] server error:", err);
  process.exit(1);
});

process.on("SIGINT", () => void shutdown(0));
process.on("SIGTERM", () => void shutdown(0));
