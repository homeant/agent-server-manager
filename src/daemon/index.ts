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
        const info = sup.register(req.spec);
        if (req.start) await sup.start(req.spec.name);
        return reply(sock, { id: req.id, ok: true, result: sup.get(req.spec.name) ?? info });
      }
      case "start":
        return reply(sock, { id: req.id, ok: true, result: await sup.start(req.name) });
      case "stop":
        return reply(sock, { id: req.id, ok: true, result: await sup.stop(req.name) });
      case "restart":
        return reply(sock, { id: req.id, ok: true, result: await sup.restart(req.name) });
      case "remove":
        await sup.remove(req.name);
        return reply(sock, { id: req.id, ok: true, result: { removed: req.name } });
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

// 启动前清掉可能残留的 socket 文件
try {
  fs.unlinkSync(SOCKET_PATH);
} catch {
  /* ignore */
}

server.listen(SOCKET_PATH, () => {
  fs.writeFileSync(PID_FILE, String(process.pid));
  // 通知父进程（auto-spawn 时）daemon 已就绪
  if (process.send) process.send("ready");
  console.log(`[svc-daemon] listening on ${SOCKET_PATH} (pid ${process.pid})`);
});

server.on("error", (err) => {
  console.error("[svc-daemon] server error:", err);
  process.exit(1);
});

process.on("SIGINT", () => void shutdown(0));
process.on("SIGTERM", () => void shutdown(0));
