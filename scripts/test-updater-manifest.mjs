import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const version = JSON.parse(fs.readFileSync(path.join(root, "package.json"), "utf8")).version;
const releaseRoot = fs.mkdtempSync(path.join(os.tmpdir(), "asvc-updater-manifest-"));

try {
  const fixtures = [
    `asvc-desktop-v${version}-darwin-arm64-Asvc.app.tar.gz`,
    `asvc-desktop-v${version}-darwin-x64-Asvc.app.tar.gz`,
    `asvc-desktop-v${version}-win32-x64-Asvc-setup.exe`,
  ];
  for (const name of fixtures) {
    fs.writeFileSync(path.join(releaseRoot, name), "update");
    fs.writeFileSync(path.join(releaseRoot, `${name}.sig`), `signature:${name}\n`);
  }

  const result = spawnSync(process.execPath, [path.join(root, "scripts", "generate-updater-manifest.mjs")], {
    cwd: root,
    env: {
      ...process.env,
      ASVC_RELEASE_ROOT: releaseRoot,
      ASVC_RELEASE_DATE: "2026-08-10T18:00:00Z",
      GITHUB_REF_NAME: `v${version}`,
      GITHUB_REPOSITORY: "homeant/agent-server-manager",
    },
    encoding: "utf8",
  });
  assert.equal(result.status, 0, result.stderr || result.stdout);

  const manifest = JSON.parse(fs.readFileSync(path.join(releaseRoot, "latest.json"), "utf8"));
  assert.equal(manifest.version, version);
  assert.equal(manifest.pub_date, "2026-08-10T18:00:00Z");
  assert.deepEqual(Object.keys(manifest.platforms).sort(), [
    "darwin-aarch64",
    "darwin-x86_64",
    "windows-x86_64",
  ]);
  assert.match(manifest.platforms["darwin-aarch64"].url, /releases\/download\/v/);
  assert.match(manifest.platforms["windows-x86_64"].signature, /^signature:/);
  console.log("updater manifest fixture passed");
} finally {
  fs.rmSync(releaseRoot, { recursive: true, force: true });
}
