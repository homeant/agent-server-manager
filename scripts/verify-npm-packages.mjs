import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const rootPackage = JSON.parse(fs.readFileSync(path.join(root, "package.json"), "utf8"));
const platforms = [
  "darwin-arm64",
  "darwin-x64",
  "linux-arm64",
  "linux-x64",
  "win32-x64",
];

for (const platform of platforms) {
  const platformRoot = path.join(root, "npm", `asvc-${platform}`);
  const platformPackage = JSON.parse(
    fs.readFileSync(path.join(platformRoot, "package.json"), "utf8")
  );
  assert.equal(platformPackage.version, rootPackage.version, `${platform} version differs`);
  assert.equal(
    rootPackage.optionalDependencies[platformPackage.name],
    rootPackage.version,
    `${platform} optional dependency must use the exact release version`
  );

  const executable = platform.startsWith("win32-") ? "asvc.exe" : "asvc";
  const binary = path.join(platformRoot, "bin", executable);
  const current = platform === `${process.platform}-${process.arch}`;
  if (current || process.env.ASVC_REQUIRE_ALL_BINARIES === "1") {
    assert.ok(fs.statSync(binary).isFile(), `missing native binary: ${binary}`);
    if (!platform.startsWith("win32-")) fs.accessSync(binary, fs.constants.X_OK);
  }
}
