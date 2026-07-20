---
name: asvc
description: >-
  Manage local development servers through the `asvc` CLI so
  the human and the agent share ONE managed daemon instead of each spawning their
  own processes. Use this skill whenever you are about to start, run, restart, stop,
  or tail the logs of a long-running dev process — `npm run dev`, `vite`, `next dev`,
  `go run`, `flask run`, `uvicorn`, a watch task, any local server — even if the
  user just says "run the app", "start the backend", "restart it", "why won't the
  server come up", or "show me the server logs". Especially use it before launching
  any server in the background with `&` or as a blocking foreground process: route
  it through `asvc` instead. This prevents duplicate processes and port conflicts
  between you and the user, and lets either side restart a service without killing
  the other's terminal.
---

# Dev Server Manager (`asvc`)

## Why this exists

In a normal dev session both you (the agent) and the human end up starting servers.
That causes **port conflicts** and **orphaned processes** nobody is tracking. This tool
fixes that: a single background **daemon** owns every dev process, keyed by **service
name**. You and the human both talk to that daemon through one `asvc` command.

Two properties matter and shape how you should behave:

1. **Name-deduplication.** Starting a service that is already running is a no-op, not a
   second copy. So routing every server through `asvc` means there is never a duplicate
   fighting over a port.
2. **Restart doesn't disconnect anyone.** The service is a child of the daemon, not of
   any terminal. When you `asvc restart web`, the human who is watching logs in their
   terminal stays connected — they just see a `restarting` marker and the new logs.
   That is the whole point: you can restart freely without disrupting the human.

## The one rule that matters for you

When you start a server, **always** go through `asvc` and **always** pass `-d` (detach):

```bash
asvc start <name> -c "<command>" -d
```

`-d` runs it in the background and returns immediately with an exit code. Without `-d`
the command runs in the **foreground and blocks forever** — that mode exists for humans
who want to watch logs live, and it will hang your turn. So for a single-service start,
`-d` is mandatory. The only exception is `asvc start --all`, which is itself a finite
background batch operation.

Never start a dev server by running it directly (`npm run dev`, `npm run dev &`,
`nohup ...`). Route it through `asvc` so it joins the managed set.

## Resolving the `asvc` command

`asvc` comes from the npm package `@homeant/asvc`. It should already be on `PATH`.
If `command -v asvc` finds nothing, install it globally:

```bash
npm install -g @homeant/asvc
```

(If you are working inside the agent-server-manager repo itself, `npm run build && npm link`
uses the local build instead.)

If asdf reports that `asvc` exists only under a different Node version, do **not** change
to another project directory just to make the shim run. That changes only the control CLI's
resolution and does not select the managed service's runtime. Use a stable `asvc` executable
outside the asdf shims, or install `@homeant/asvc` under the current Node version.

For released `asvc` versions that do not yet isolate the service runtime, make the registered
command explicit whenever the service cwd has `.tool-versions`:

```bash
ASVC_BIN=/absolute/path/to/asvc-outside-asdf-shims
ASDF_BIN="$(command -v asdf)"
"$ASVC_BIN" start web -c "$ASDF_BIN exec npm run dev" --cwd /absolute/path/to/web --port 3000 -d
"$ASVC_BIN" restart web
```

Resolve both binaries to absolute, non-shim paths before registration. Updating a running
service's definition does not change its current process; restart it, then verify the expected
runtime and the actual managed process executable:

```bash
cd /absolute/path/to/web
asdf exec node -p 'process.version + " " + process.execPath'
lsof -a -p <service-pid> -d txt
```

A correct cwd, an open port, or HTTP 200 does not prove the runtime version.

## Commands

```bash
# Start a server (registers on first use; -d = background, REQUIRED for the agent)
asvc start web -c "npm run dev" --port 3000 -d        # cwd defaults to current dir
asvc start api -c "go run ." --cwd /path/api --env KEY=VAL -d
asvc start web -d                                      # already-registered → omit -c

asvc list                  # all services + status + process-group CPU/memory usage
asvc logs web              # last 200 log lines (snapshot, returns)
asvc logs web -n 500       # last 500 lines
asvc restart web           # restart after a code change (won't disconnect the human)
asvc stop web              # stop the process (definition kept, can start again)
asvc rm web                # stop and forget the service

asvc start --all           # start every registered service in the background
asvc stop --all            # stop every service, keeping definitions
asvc rm --all --yes        # stop and forget every service (logs are retained)

asvc daemon status         # is the daemon up?
asvc daemon stop           # stop daemon + all services
```

`start` flags: `-c/--cmd` (command, run via shell), `-w/--cwd` (default: current dir),
`-p/--port` (display/diagnostics only), `-e/--env KEY=VAL` (repeatable),
`--autorestart` (respawn on crash), `-d/--detach` (background — use this).

Bulk commands are always finite operations that print a per-service summary and return.
`asvc start --all` is background-only, so it does not need `-d` and never attaches to logs.
Bulk operations continue after individual failures and return non-zero if any service fails.
Removing every registration requires the explicit safety flag `--yes`; `asvc rm --all`
only previews the number of affected services. Removal retains historical log files.

## Reacting to results

`asvc start -d`, `restart`, and `stop` print a status line and return an exit code.
After starting/restarting, the daemon waits ~1s to confirm the process didn't crash:

- **Exit code 0** → the service is running. Good.
- **Non-zero exit code** → it failed to stay up. The output includes the last log lines.
  A common cause is the port being taken by something the daemon doesn't manage (the
  human's own process, an orphan, another service): the output will flag `EADDRINUSE`.
  Don't treat it as running — fix the cause (free the port or pick another) and retry.

So you can rely on the exit code: no need to optimistically assume success and then dig
through logs. If it failed, read the printed log tail, act on it, and try again.

## Typical workflow

1. Start each server once, in its directory: `asvc start web -c "npm run dev" --port 3000 -d`.
2. Make code changes.
3. `asvc restart web` to pick them up — the human watching `asvc logs web -f` is not disturbed.
4. If something looks wrong: `asvc logs web -n 200` to inspect, or `asvc list` to see status.
5. Leave services running for the human; use `asvc stop`/`asvc rm` only when genuinely done.

## Examples

**Start the frontend and backend of a project**
```bash
asvc start web -c "npm run dev" --port 3000 -d
asvc start api -c "uvicorn app:app --reload --port 8000" --cwd ./backend --port 8000 -d
asvc list
```

**Restart after editing backend code**
```bash
asvc restart api          # exit 0 → running; non-zero → read the printed logs
```

**Diagnose a server that won't start**
```bash
asvc start api -c "uvicorn app:app --port 8000" --port 8000 -d   # non-zero exit + EADDRINUSE?
asvc logs api -n 100                                             # confirm the cause
# free port 8000 or choose another, then retry
```
