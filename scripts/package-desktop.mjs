import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const version = JSON.parse(fs.readFileSync(path.join(root, "package.json"), "utf8")).version;
const platform = process.env.ASVC_DESKTOP_PLATFORM || `${process.platform}-${process.arch}`;
const tauriTarget = process.env.ASVC_TAURI_TARGET;
const tauriTargetRoot = path.join(root, "src-tauri", "target");
const bundleCandidates = [
  path.join(tauriTargetRoot, ...(tauriTarget ? [tauriTarget] : []), "release", "bundle"),
  path.join(tauriTargetRoot, "release", "bundle"),
  ...(fs.existsSync(tauriTargetRoot)
    ? fs
        .readdirSync(tauriTargetRoot, { withFileTypes: true })
        .filter((entry) => entry.isDirectory() && entry.name !== "release")
        .map((entry) => path.join(tauriTargetRoot, entry.name, "release", "bundle"))
    : []),
];
const bundleRoot = bundleCandidates.find(
  (candidate) => fs.existsSync(candidate)
);
const releaseRoot = path.join(root, "release");

if (!bundleRoot) {
  throw new Error(`desktop bundle directory does not exist under ${tauriTargetRoot}`);
}

const packageBinary = path.join(
  root,
  "src-tauri",
  "binaries",
  process.platform === "win32" ? "asvc.exe" : "asvc"
);
if (!fs.existsSync(packageBinary)) {
  throw new Error(`desktop bundle was not prepared with an asvc CLI: ${packageBinary}`);
}

function filesUnder(directory) {
  const result = [];
  for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
    const file = path.join(directory, entry.name);
    if (entry.isDirectory()) result.push(...filesUnder(file));
    else result.push(file);
  }
  return result;
}

const bundleFiles = filesUnder(bundleRoot).filter((file) => {
  const lower = file.toLowerCase();
  return (
    lower.endsWith(".dmg") ||
    lower.endsWith(".deb") ||
    lower.endsWith(".rpm") ||
    lower.endsWith(".appimage") ||
    lower.endsWith(".msi") ||
    lower.endsWith(".exe") ||
    lower.endsWith(".app.tar.gz")
  );
});

if (bundleFiles.length === 0) {
  throw new Error(`no desktop installer was produced under ${bundleRoot}`);
}

fs.mkdirSync(releaseRoot, { recursive: true });
const artifactPrefix = `asvc-desktop-v${version}-${platform}-`;
for (const entry of fs.readdirSync(releaseRoot, { withFileTypes: true })) {
  if (
    (entry.isFile() && entry.name.startsWith(artifactPrefix)) ||
    entry.name === `SHA256SUMS-desktop-${platform}`
  ) {
    fs.rmSync(path.join(releaseRoot, entry.name), { force: true });
  }
}
const copied = [];
for (const source of bundleFiles) {
  const destination = path.join(
    releaseRoot,
    `asvc-desktop-v${version}-${platform}-${path.basename(source)}`
  );
  fs.copyFileSync(source, destination);
  copied.push(destination);
}

function sha256(file) {
  return crypto.createHash("sha256").update(fs.readFileSync(file)).digest("hex");
}

const checksumFile = path.join(releaseRoot, `SHA256SUMS-desktop-${platform}`);
fs.writeFileSync(
  checksumFile,
  copied.map((file) => `${sha256(file)}  ${path.basename(file)}`).join("\n") + "\n"
);

for (const file of copied) console.log(`desktop artifact: ${file}`);
console.log(`desktop checksums: ${checksumFile}`);
