import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import test from "node:test";
import {
  buildSystemEntry,
  findExecutable,
  installSystemEntry,
} from "../dist/system-entry.js";

test("findExecutable resolves from a minimal PATH without shell profiles", (t) => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "asvc-entry-"));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const bin = path.join(root, "bin");
  fs.mkdirSync(bin);
  const asdf = path.join(bin, "asdf");
  fs.writeFileSync(asdf, "#!/bin/sh\nexit 0\n", { mode: 0o755 });

  assert.equal(findExecutable("asdf", { PATH: `${bin}:/usr/bin:/bin` }), asdf);
  assert.equal(findExecutable("asvc", { PATH: "/usr/bin:/bin" }), undefined);
});

test("stable entry invokes asdf exec asvc for version and list under minimal PATH", (t) => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "asvc-entry-"));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const bin = path.join(root, "bin");
  fs.mkdirSync(bin);
  const asdf = path.join(bin, "asdf");
  fs.writeFileSync(
    asdf,
    [
      "#!/bin/sh",
      'test "$1" = "exec"',
      'test "$2" = "asvc"',
      "shift 2",
      'printf "resolved:%s\\n" "$*"',
      "",
    ].join("\n"),
    { mode: 0o755 }
  );

  const minimalPath = `${bin}:/usr/bin:/bin`;
  const installed = installSystemEntry({ env: { PATH: minimalPath } });
  assert.equal(installed.changed, true);
  assert.equal(installed.path, path.join(bin, "asvc"));
  assert.equal(
    installSystemEntry({ env: { PATH: minimalPath } }).changed,
    false
  );

  const env = { PATH: minimalPath };
  const version = spawnSync("asvc", ["--version"], { env, encoding: "utf8" });
  const list = spawnSync("asvc", ["list"], { env, encoding: "utf8" });
  assert.equal(version.status, 0, version.stderr);
  assert.equal(version.stdout, "resolved:--version\n");
  assert.equal(list.status, 0, list.stderr);
  assert.equal(list.stdout, "resolved:list\n");
});

test("stable entry refuses to replace an unrelated command without force", (t) => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "asvc-entry-"));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const bin = path.join(root, "bin");
  fs.mkdirSync(bin);
  fs.writeFileSync(path.join(bin, "asvc"), "foreign\n", { mode: 0o755 });

  assert.throws(
    () => installSystemEntry({ asdfPath: "/opt/tools/asdf", binDir: bin }),
    /目标已存在/
  );
  const result = installSystemEntry({
    asdfPath: "/opt/tools/asdf",
    binDir: bin,
    force: true,
  });
  assert.equal(result.changed, true);
  assert.equal(fs.readFileSync(result.path, "utf8"), buildSystemEntry("/opt/tools/asdf"));
});
