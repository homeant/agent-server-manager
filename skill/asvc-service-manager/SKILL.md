---
name: asvc-service-manager
description: >-
  Manage local development servers through the `asvc` CLI so the human and the agent
  share one daemon instead of spawning duplicate processes. Use whenever starting,
  running, inspecting, restarting, stopping, reading, or following logs for a
  long-running dev process such as npm run dev, vite, next dev, go run, flask, uvicorn,
  or a watch task. Route server processes through asvc instead of direct foreground
  commands, &, or nohup.
---

# Dev Server Manager (`asvc`)

## Core rule

Always route a development server through `asvc`. Agent starts must use detach mode:

```bash
asvc start <name> -c "<command>" -d
```

Without `-d`, `start` attaches to logs and blocks for a human terminal. The exception is
`asvc start --all`, which is already a finite background operation.

Services are keyed by name, so a second start does not create a duplicate. The daemon owns the
process group; restarting a service does not disconnect a human following its logs.

## Resolve the control command

Prefer the native standalone executable in Codex and other non-login environments. It contains
the Rust CLI and daemon and does not require Node or a version manager. The npm package is a
supported installation channel, but its small launcher still lives in the selected npm prefix.

Supported native targets are macOS arm64/x64, Linux arm64/x64, and Windows x64. On macOS,
Homebrew is the preferred installation channel:

```bash
brew install homeant/tap/asvc
```

Do not reinstall merely because `command -v asvc` is empty. First check stable native locations:

```bash
command -v asvc || true
for candidate in \
  "${XDG_BIN_HOME:-$HOME/.local/bin}/asvc" \
  /opt/homebrew/bin/asvc \
  /usr/local/bin/asvc; do
  if [ -x "$candidate" ]; then
    printf '%s\n' "$candidate"
    "$candidate" --version
  fi
done
```

Interpret the result:

- If bare `asvc` works, use it.
- If an absolute native candidate works, use that path immediately; fix PATH separately.
- If only an npm installation exists, invoke a confirmed absolute launcher path. Do not source a
  shell profile, guess a version-manager directory, or duplicate the install just to repair PATH.
- If neither exists, ask before installing unless installation was already requested.

For an npm installation, distinguish an omitted global bin from an absent package only when the
current shell already has a working `npm`. For example:

```bash
npm prefix -g
npm root -g
test -f "$(npm root -g)/@homeant/asvc/package.json"
test -x "$(npm prefix -g)/bin/asvc"
```

Do not add manager-specific fallback logic to asvc. The native command is the stable control
entry; npm/asdf/nvm/fnm/Volta are installation concerns outside the Rust core.

Install only from a trusted GitHub Release and verify `SHA256SUMS`. Never invent a download URL.
Validate an absolute binary without loading a profile:

```bash
ASVC_BIN=/absolute/path/to/asvc
env -i HOME="$HOME" PATH=/usr/bin:/bin "$ASVC_BIN" --version
```

`daemon status` is read-only. `list` and other service operations may auto-start the daemon.

## Service PATH and version managers

The Rust core contains no asdf, nvm, fnm, Volta, or language-specific adapter. Use these rules:

1. `start <name> -c ...` registers or updates a definition and captures the calling process's
   current `PATH`.
2. Later `restart`, daemon restart, and `start <name>` reuse that saved PATH.
3. An explicit `--env PATH=...` overrides the captured value.
4. `--cwd` changes only the working directory. It does not load `.tool-versions`, `.nvmrc`, or a
   shell profile by itself.

This means the first registration must run from an environment where the service command already
resolves to the intended runtime:

```bash
command -v node
node --version
command -v npm
asvc start web -c "npm run dev" --cwd /absolute/path/to/web --port 3000 -d
```

Codex must invoke commands with `login:false` and must never pass `-i` or wrap an operation in
`$SHELL -ic`. Interactive/login initialization loads prompts, aliases, plugins, and arbitrary
startup scripts; it can be noisy, slow, or block automation. This applies to every asvc operation,
including first registration.

If the first registration needs a command missing from the non-login PATH, do not register the
service with the wrong environment. Use one of these deterministic options instead:

- Pass a confirmed absolute runtime command or `--env PATH=...`.
- If the manager exposes a known non-interactive command, use it explicitly.
- Load a manager script only when its exact path is already known; do not guess a profile path.
- Ask the user to perform the first registration from their configured terminal when the intended
  runtime cannot be resolved safely.

Do not change to an unrelated project directory merely to make an npm-installed asvc shim resolve.

When updating a service's runtime, register the definition again from the newly selected user
environment; that replaces the saved PATH without starting a duplicate:

```bash
asvc start web -c "npm run dev" --cwd /absolute/path/to/web --port 3000 -d
```

Updating the asvc binary does not replace an already running daemon. Plan a brief interruption:

```bash
asvc daemon stop
asvc start --all
```

Finally verify the real application process, not only cwd, port, or HTTP status. The listed PID is
usually the shell process-group leader:

```bash
ps -axo pid=,pgid=,command= | awk -v pgid=<service-pid> '$2 == pgid {print}'
lsof -a -p <actual-app-pid> -d txt
ps -p <actual-app-pid> -o command=
```

## Commands

```bash
# Agent start: -d is required
asvc start web -c "npm run dev" --port 3000 -d
asvc start api -c "go run ." --cwd /path/api --env KEY=VAL -d
asvc start web -d

asvc list
asvc info web             # `show` is an alias
asvc logs web
asvc logs web -n 500
asvc logs web -f
asvc restart web
asvc stop web
asvc rm web

asvc start --all
asvc stop --all
asvc rm --all --yes

asvc daemon status
asvc daemon stop
```

Current command shapes:

- `start [NAME]`: accept `-c/--cmd`, `-w/--cwd`, `-p/--port`, repeatable
  `-e/--env KEY=VAL`, `--autorestart`, `-d/--detach`, and `-a/--all`.
- `stop [NAME]`: accept `-a/--all`.
- `restart <NAME>`: require exactly one service name; no bulk form exists.
- `logs <NAME>`: accept `-n/--lines <LINES>` (default 200) and `-f/--follow`.
- `list`: show a compact overview of all services.
- `info <NAME>` (alias `show`): show one service's complete status, resource usage, launch
  configuration, and environment overrides.
- `rm [NAME]`: accept `-a/--all` and `-y/--yes`.
- `daemon status`: check the daemon without auto-starting it.
- `daemon stop`: stop the daemon and the services it manages. There is no `daemon start`
  subcommand; a service operation starts the daemon when needed.

Use plain `logs` or a bounded `-n` value for diagnosis. `logs -f` continuously follows output;
use it only when live streaming is requested and the execution environment can safely manage a
long-running attached command.

`info` prints registered environment overrides, which may contain secrets. Inspect them locally
when needed, but redact sensitive values before quoting or forwarding the output.

Bulk operations continue after individual failures and return non-zero if any item fails.
`rm --all` only previews; `rm --all --yes` performs the destructive operation. Historical logs
remain on disk.

## React to results

After start or restart, the daemon waits about one second to catch immediate failures:

- Exit code 0 means the service stayed running.
- A non-zero exit code means it failed. Read the printed log tail and fix the cause.
- `EADDRINUSE` means the port is already occupied; free it or choose another port before retrying.

Do not assume success after a non-zero result. For later diagnosis use:

```bash
asvc list
asvc info <name>
asvc logs <name> -n 200
```

Leave services running for the human unless the task genuinely requires stopping or removing them.
