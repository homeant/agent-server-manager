---
name: dev-server-manager
description: >-
  Manage local development servers through the `svc` CLI (dev-server-manager) so
  the human and the agent share ONE managed daemon instead of each spawning their
  own processes. Use this skill whenever you are about to start, run, restart, stop,
  or tail the logs of a long-running dev process — `npm run dev`, `vite`, `next dev`,
  `go run`, `flask run`, `uvicorn`, a watch task, any local server — even if the
  user just says "run the app", "start the backend", "restart it", "why won't the
  server come up", or "show me the server logs". Especially use it before launching
  any server in the background with `&` or as a blocking foreground process: route
  it through `svc` instead. This prevents duplicate processes and port conflicts
  between you and the user, and lets either side restart a service without killing
  the other's terminal.
---

# Dev Server Manager (`svc`)

## Why this exists

In a normal dev session both you (the agent) and the human end up starting servers.
That causes **port conflicts** and **orphaned processes** nobody is tracking. This tool
fixes that: a single background **daemon** owns every dev process, keyed by **service
name**. You and the human both talk to that daemon through one `svc` command.

Two properties matter and shape how you should behave:

1. **Name-deduplication.** Starting a service that is already running is a no-op, not a
   second copy. So routing every server through `svc` means there is never a duplicate
   fighting over a port.
2. **Restart doesn't disconnect anyone.** The service is a child of the daemon, not of
   any terminal. When you `svc restart web`, the human who is watching logs in their
   terminal stays connected — they just see a `restarting` marker and the new logs.
   That is the whole point: you can restart freely without disrupting the human.

## The one rule that matters for you

When you start a server, **always** go through `svc` and **always** pass `-d` (detach):

```bash
svc start <name> -c "<command>" -d
```

`-d` runs it in the background and returns immediately with an exit code. Without `-d`
the command runs in the **foreground and blocks forever** — that mode exists for humans
who want to watch logs live, and it will hang your turn. So for you, `-d` is mandatory.

Never start a dev server by running it directly (`npm run dev`, `npm run dev &`,
`nohup ...`). Route it through `svc` so it joins the managed set.

## Resolving the `svc` command

Prefer `svc` if it is on `PATH` (installed via `npm link`). If `command -v svc` finds
nothing, invoke it through Node from the repo:

```bash
node /Users/tianhui/Project/tianhui/server-manager/dist/cli.js <args...>
```

If neither works, the project hasn't been built — run `npm install && npm run build` in
that repo first (and suggest `npm link` so `svc` is available everywhere).

## Commands

```bash
# Start a server (registers on first use; -d = background, REQUIRED for the agent)
svc start web -c "npm run dev" --port 3000 -d        # cwd defaults to current dir
svc start api -c "go run ." --cwd /path/api --env KEY=VAL -d
svc start web -d                                      # already-registered → omit -c

svc list                  # all services + status (running/exited/...) 
svc logs web              # last 200 log lines (snapshot, returns)
svc logs web -n 500       # last 500 lines
svc restart web           # restart after a code change (won't disconnect the human)
svc stop web              # stop the process (definition kept, can start again)
svc rm web                # stop and forget the service

svc daemon status         # is the daemon up?
svc daemon stop           # stop daemon + all services
```

`start` flags: `-c/--cmd` (command, run via shell), `-w/--cwd` (default: current dir),
`-p/--port` (display/diagnostics only), `-e/--env KEY=VAL` (repeatable),
`--autorestart` (respawn on crash), `-d/--detach` (background — use this).

## Reacting to results

`svc start -d`, `restart`, and `stop` print a status line and return an exit code.
After starting/restarting, the daemon waits ~1s to confirm the process didn't crash:

- **Exit code 0** → the service is running. Good.
- **Non-zero exit code** → it failed to stay up. The output includes the last log lines.
  A common cause is the port being taken by something the daemon doesn't manage (the
  human's own process, an orphan, another service): the output will flag `EADDRINUSE`.
  Don't treat it as running — fix the cause (free the port or pick another) and retry.

So you can rely on the exit code: no need to optimistically assume success and then dig
through logs. If it failed, read the printed log tail, act on it, and try again.

## Typical workflow

1. Start each server once, in its directory: `svc start web -c "npm run dev" --port 3000 -d`.
2. Make code changes.
3. `svc restart web` to pick them up — the human watching `svc logs web -f` is not disturbed.
4. If something looks wrong: `svc logs web -n 200` to inspect, or `svc list` to see status.
5. Leave services running for the human; use `svc stop`/`svc rm` only when genuinely done.

## Examples

**Start the frontend and backend of a project**
```bash
svc start web -c "npm run dev" --port 3000 -d
svc start api -c "uvicorn app:app --reload --port 8000" --cwd ./backend --port 8000 -d
svc list
```

**Restart after editing backend code**
```bash
svc restart api          # exit 0 → running; non-zero → read the printed logs
```

**Diagnose a server that won't start**
```bash
svc start api -c "uvicorn app:app --port 8000" --port 8000 -d   # non-zero exit + EADDRINUSE?
svc logs api -n 100                                             # confirm the cause
# free port 8000 or choose another, then retry
```
