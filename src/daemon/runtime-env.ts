import fs from "node:fs";
import os from "node:os";
import path from "node:path";

/**
 * Build the environment inherited by a managed service.
 *
 * The daemon itself may have been launched through an asdf shim. In that case
 * asdf puts the daemon's selected tool installation (for example Node 25) near
 * the front of PATH. Reusing that PATH verbatim pins every child service to the
 * daemon's tool version, even when the service cwd selects another version in
 * .tool-versions.
 *
 * Put asdf's stable shim directory first instead. Each shim resolves its tool
 * from the child process cwd, so restarts and batch starts honor the service's
 * own .tool-versions. Explicit service env values are applied last; in
 * particular, --env PATH=... remains an intentional escape hatch.
 */
export function buildServiceEnv(
  daemonEnv: NodeJS.ProcessEnv,
  serviceEnv?: Record<string, string>
): NodeJS.ProcessEnv {
  const inherited = { ...daemonEnv };
  const dataDir =
    inherited.ASDF_DATA_DIR || path.join(inherited.HOME || os.homedir(), ".asdf");
  const shimsDir = path.join(dataDir, "shims");

  if (fs.existsSync(shimsDir)) {
    inherited.PATH = prependPath(shimsDir, inherited.PATH);
  }

  return { ...inherited, ...(serviceEnv ?? {}) };
}

function prependPath(entry: string, current?: string): string {
  const entries = (current ?? "").split(path.delimiter).filter(Boolean);
  const normalizedEntry = path.resolve(entry);
  const withoutDuplicate = entries.filter((item) => path.resolve(item) !== normalizedEntry);
  return [entry, ...withoutDuplicate].join(path.delimiter);
}
