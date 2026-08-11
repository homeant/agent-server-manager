import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const releaseRoot = process.env.ASVC_RELEASE_ROOT
  ? path.resolve(process.env.ASVC_RELEASE_ROOT)
  : path.join(root, "release");
const version = JSON.parse(fs.readFileSync(path.join(root, "package.json"), "utf8")).version;
const tag = process.env.GITHUB_REF_NAME || `v${version}`;
const repository = process.env.GITHUB_REPOSITORY || "homeant/agent-server-manager";

function findUpdateBundle(platform, suffix) {
  const prefix = `asvc-desktop-v${version}-${platform}-`;
  const candidates = fs
    .readdirSync(releaseRoot)
    .filter((name) => name.startsWith(prefix) && name.endsWith(suffix) && !name.endsWith(".sig"));
  if (candidates.length !== 1) {
    throw new Error(`expected one ${platform} updater bundle ending in ${suffix}, found: ${candidates.join(", ") || "none"}`);
  }
  const bundle = candidates[0];
  const signatureFile = path.join(releaseRoot, `${bundle}.sig`);
  if (!fs.existsSync(signatureFile)) {
    throw new Error(`missing updater signature: ${signatureFile}`);
  }
  const signature = fs.readFileSync(signatureFile, "utf8").trim();
  if (!signature) throw new Error(`empty updater signature: ${signatureFile}`);
  return {
    signature,
    url: `https://github.com/${repository}/releases/download/${tag}/${encodeURIComponent(bundle)}`,
  };
}

const manifest = {
  version,
  notes: `Asvc ${version} includes the matching Desktop app, CLI and daemon. See the GitHub release notes for details.`,
  pub_date: process.env.ASVC_RELEASE_DATE || new Date().toISOString(),
  platforms: {
    "darwin-aarch64": findUpdateBundle("darwin-arm64", ".app.tar.gz"),
    "darwin-x86_64": findUpdateBundle("darwin-x64", ".app.tar.gz"),
    "windows-x86_64": findUpdateBundle("win32-x64", ".exe"),
  },
};

const destination = path.join(releaseRoot, "latest.json");
fs.writeFileSync(destination, `${JSON.stringify(manifest, null, 2)}\n`);
console.log(`updater manifest: ${destination}`);
