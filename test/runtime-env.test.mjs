import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { buildServiceEnv } from "../dist/daemon/runtime-env.js";

test("puts the stable asdf shims ahead of the daemon-selected tool install", (t) => {
  const home = fs.mkdtempSync(path.join(os.tmpdir(), "asvc-runtime-env-"));
  t.after(() => fs.rmSync(home, { recursive: true, force: true }));

  const shims = path.join(home, ".asdf", "shims");
  fs.mkdirSync(shims, { recursive: true });
  const selectedInstall = path.join(home, ".asdf", "installs", "nodejs", "25.9.0", "bin");

  const env = buildServiceEnv({
    HOME: home,
    PATH: [selectedInstall, "/usr/bin", shims].join(path.delimiter),
    ASDF_INSTALL_VERSION: "25.9.0",
  });

  assert.deepEqual(env.PATH.split(path.delimiter), [shims, selectedInstall, "/usr/bin"]);
});

test("uses ASDF_DATA_DIR when configured", (t) => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "asvc-runtime-env-"));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));

  const dataDir = path.join(root, "asdf-data");
  const shims = path.join(dataDir, "shims");
  fs.mkdirSync(shims, { recursive: true });

  const env = buildServiceEnv({ ASDF_DATA_DIR: dataDir, PATH: "/usr/bin" });
  assert.equal(env.PATH, `${shims}${path.delimiter}/usr/bin`);
});

test("keeps an explicitly configured service PATH as an escape hatch", (t) => {
  const home = fs.mkdtempSync(path.join(os.tmpdir(), "asvc-runtime-env-"));
  t.after(() => fs.rmSync(home, { recursive: true, force: true }));
  fs.mkdirSync(path.join(home, ".asdf", "shims"), { recursive: true });

  const env = buildServiceEnv(
    { HOME: home, PATH: "/daemon/node25/bin:/usr/bin" },
    { PATH: "/service/runtime/bin:/usr/bin" }
  );

  assert.equal(env.PATH, "/service/runtime/bin:/usr/bin");
});
