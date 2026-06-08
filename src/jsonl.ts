import { Socket } from "node:net";
import { Frame } from "./protocol.js";

/** 在一个 socket 上按行解析 JSON，每解析出一帧回调一次。 */
export function readFrames(
  socket: Socket,
  onFrame: (frame: Frame) => void
): void {
  let buf = "";
  socket.setEncoding("utf8");
  socket.on("data", (chunk: string) => {
    buf += chunk;
    let idx: number;
    while ((idx = buf.indexOf("\n")) >= 0) {
      const line = buf.slice(0, idx);
      buf = buf.slice(idx + 1);
      if (line.trim() === "") continue;
      try {
        onFrame(JSON.parse(line) as Frame);
      } catch {
        // 忽略坏帧
      }
    }
  });
}

export function writeFrame(socket: Socket, frame: Frame): void {
  if (socket.writable) socket.write(JSON.stringify(frame) + "\n");
}
