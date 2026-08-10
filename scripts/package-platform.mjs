import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const key = `${process.platform}-${process.arch}`;
const supported = new Set([
  "darwin-arm64",
  "darwin-x64",
  "linux-arm64",
  "linux-x64",
  "win32-x64",
]);

if (!supported.has(key)) {
  throw new Error(`unsupported packaging platform: ${key}`);
}

const executable = process.platform === "win32" ? "asvc.exe" : "asvc";
const cargoTarget = process.env.ASVC_BUILD_TARGET;
const source = cargoTarget
  ? path.join(root, "target", cargoTarget, "release", executable)
  : path.join(root, "target", "release", executable);
const releaseRoot = path.join(root, "release");
const releaseDir = path.join(releaseRoot, key);
const packageDir = path.join(root, "npm", `asvc-${key}`);
const packageBin = path.join(packageDir, "bin", executable);
const releaseBin = path.join(releaseDir, executable);
const version = JSON.parse(fs.readFileSync(path.join(root, "package.json"), "utf8")).version;
const platformVersion = JSON.parse(
  fs.readFileSync(path.join(packageDir, "package.json"), "utf8")
).version;

if (platformVersion !== version) {
  throw new Error(`version mismatch: root=${version}, ${key}=${platformVersion}`);
}
if (!fs.existsSync(source)) {
  throw new Error(`missing release binary: ${source}`);
}

fs.rmSync(releaseDir, { recursive: true, force: true });
fs.mkdirSync(releaseDir, { recursive: true });
fs.mkdirSync(path.dirname(packageBin), { recursive: true });
fs.copyFileSync(source, releaseBin);
fs.copyFileSync(source, packageBin);
if (process.platform !== "win32") {
  fs.chmodSync(releaseBin, 0o755);
  fs.chmodSync(packageBin, 0o755);
}

function run(command, args) {
  const result = spawnSync(command, args, { cwd: root, stdio: "inherit" });
  if (result.status !== 0) {
    throw new Error(`${command} ${args.join(" ")} failed with status ${result.status}`);
  }
}

if (process.platform === "darwin") {
  run("codesign", ["--force", "--sign", "-", releaseBin]);
  run("codesign", ["--force", "--sign", "-", packageBin]);
}

const archiveBase = `asvc-v${version}-${key}`;
const archive = path.join(
  releaseRoot,
  process.platform === "win32" ? `${archiveBase}.zip` : `${archiveBase}.tar.gz`
);
fs.rmSync(archive, { force: true });
if (process.platform === "win32") {
  // Windows tar treats a drive-letter path such as D:\\... as a remote
  // archive spec (host:path). Keep both paths relative to the repository so
  // the same command works from GitHub Actions' Windows checkout drive.
  const archiveRelative = path.relative(root, archive).split(path.sep).join("/");
  const releaseDirRelative = path
    .relative(root, releaseDir)
    .split(path.sep)
    .join("/");
  run("tar.exe", [
    "-a",
    "-c",
    "-f",
    archiveRelative,
    "-C",
    releaseDirRelative,
    executable,
  ]);
} else {
  run("tar", ["-czf", archive, "-C", releaseDir, executable]);
}

function sha256(file) {
  return crypto.createHash("sha256").update(fs.readFileSync(file)).digest("hex");
}

const checksumFile = path.join(releaseRoot, `SHA256SUMS-${key}`);
fs.writeFileSync(
  checksumFile,
  `${sha256(archive)}  ${path.basename(archive)}\n`
);

console.log(`platform: ${key}`);
console.log(`standalone: ${releaseBin}`);
console.log(`npm binary: ${packageBin}`);
console.log(`archive: ${archive}`);
console.log(`checksums: ${checksumFile}`);
