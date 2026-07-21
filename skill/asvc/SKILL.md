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

`asvc` comes from the npm package `@homeant/asvc`. Resolve it before deciding to install.
In agent or Codex non-login shells, `PATH` may omit the selected Node/global npm bin supplied by
asdf, nvm, fnm, Volta, or another manager even when asvc is already installed. Never reinstall
solely because `command -v asvc` returns nothing, and never assume the customer uses asdf.

Use this diagnostic order:

```bash
command -v asvc || true
command -v node || true
command -v npm || true
command -v asdf || true

# A previous setup-entry may exist even when its directory is still absent from PATH.
for candidate in \
  "${XDG_BIN_HOME:-$HOME/.local/bin}/asvc" \
  /opt/homebrew/bin/asvc \
  /usr/local/bin/asvc; do
  if [ -x "$candidate" ]; then
    "$candidate" --version
  fi
done

if command -v asdf >/dev/null 2>&1; then
  asdf current nodejs 2>/dev/null || true
  asdf shimversions asvc 2>/dev/null || true
  asdf which asvc 2>/dev/null || true
fi
```

Interpret the result as follows:

- `command -v asvc` succeeds: invoke bare `asvc`.
- Bare asvc is missing and asdf is available, but `asdf which asvc` succeeds: asvc is installed
  for the selected Node.js version and only the shim PATH is missing. For an immediate command,
  invoke `asdf exec asvc <args>`; do not reinstall.
- `asdf shimversions asvc` lists versions but `asdf which asvc` fails: asvc exists only under a
  different Node.js version. Install it under the currently selected version.
- asdf is absent: this is not an error. If another manager is in use, select/load that manager's
  installed Node in a normal shell and check its global packages before deciding to install.
- No Node manager/runtime is visible in the non-login shell and no stable entry exists: bootstrap
  the entry once from a shell where the intended Node and asvc are available. Do not guess a
manager or repeatedly install packages from the stripped shell.

For nvm specifically, `nvm` is normally a shell function and is expected to be absent when its
profile was not loaded. In a shell where nvm is already initialized, classify the state with:

```bash
nvm current
command -v node
command -v asvc || true
npm ls -g --depth=0 @homeant/asvc
```

If `npm ls` and `asvc --version` succeed, the package is installed; run `asvc setup-entry` and do
not reinstall it. Only run `npm install -g @homeant/asvc@latest` after the global-package check
fails for the intentionally selected nvm Node. Do not blindly source a guessed nvm profile from
an agent shell.

For asvc 0.3.7+, install a manager-neutral stable entry for future non-login shells. Invoke it
through whichever installation is currently reachable (bare command shown; `asdf exec asvc` is
only an asdf-specific bootstrap fallback):

```bash
asvc setup-entry
hash -r 2>/dev/null || true
command -v asvc
asvc --version
```

`setup-entry` installs a small wrapper in the Homebrew prefix's bin when available, otherwise in
`$XDG_BIN_HOME` or `~/.local/bin`. It pins the actual Node executable and asvc CLI module which ran
the setup command; at invocation time it calls no version-manager command. Thus asdf, nvm, fnm,
Volta, and system-Node installations use the same non-login entry mechanism. It refuses to
overwrite an unrelated command unless explicitly given `--force`, and warns when the selected
directory is not currently in PATH. Rerun it before/after removing the pinned Node version or
moving the global asvc installation.

Record the absolute entry path printed by `setup-entry`. A loaded nvm/asdf shell may still resolve
`command -v asvc` to its own global bin first, so that check alone does not verify the wrapper.
Verify the printed entry itself in a stripped environment (replace the example path):

```bash
ASVC_STABLE=/absolute/path/printed/by/setup-entry
env -i HOME="$HOME" PATH="$(dirname "$ASVC_STABLE"):/usr/bin:/bin" \
  "$ASVC_STABLE" --version
env -i HOME="$HOME" PATH="$(dirname "$ASVC_STABLE"):/usr/bin:/bin" \
  "$ASVC_STABLE" list
```

If asdf diagnostics prove installation is actually required, first expose its selected Node.js
bin directory so both npm and its `#!/usr/bin/env node` interpreter resolve reliably:

```bash
ASVC_NODE_BIN_DIR="$(dirname "$(asdf which node)")"
PATH="$ASVC_NODE_BIN_DIR:$PATH" npm install -g @homeant/asvc@latest
asdf reshim nodejs <current-version>
```

(If you are working inside the agent-server-manager repo itself, build and link using the same
selected-Node PATH technique.)

### Service runtime selection and asdf

asdf isolates globally installed npm packages by Node.js version. If projects using multiple
Node.js versions invoke `asvc` through asdf shims, install the same package version under each.
Once the manager-neutral stable entry is installed, this duplication is no longer required for
the control CLI. All CLI installations connect to one global daemon; they do not create one
daemon per Node.js version.

Before using asvc in a project selected by `.tool-versions`, check the current coverage:

```bash
asdf current nodejs
asdf shimversions asvc
asvc --version
```

If `asvc` is missing under the selected Node.js version, install and reshim it there:

```bash
ASVC_NODE_BIN_DIR="$(dirname "$(asdf which node)")"
PATH="$ASVC_NODE_BIN_DIR:$PATH" npm install -g @homeant/asvc@latest
asdf reshim nodejs <current-version>
```

Repeat this for every Node.js version whose project directories will invoke `asvc`, and keep
the installed asvc versions aligned. If the asdf shim says `asvc` exists only under another
Node.js version, do **not** change to another project directory to make it run. That only changes
control-CLI resolution and says nothing about the managed service's runtime.

With asvc 0.3.4 or newer, register the normal bare service command. The daemon puts the asdf
shims first for the service process, so the service's `cwd` and `.tool-versions` select Node.js:

```bash
asvc start web -c "npm run dev" --cwd /absolute/path/to/web --port 3000 -d
```

Do not add `asdf exec` to service definitions on 0.3.4+ unless there is a specific override.
An explicit `--env PATH=...` intentionally takes precedence over asvc's automatic shim setup.

This automatic project-runtime behavior is specifically an asdf adapter. Do not claim that cwd
alone applies nvm `.nvmrc`, fnm, or other manager configuration. For those managers, register a
command or `--env PATH=...` that explicitly selects the intended runtime until a corresponding
service-runtime adapter exists. The stable control entry and service runtime selection are
separate concerns.

For asvc 0.3.3 or older, use an absolute asdf binary in the registered command as a temporary
compatibility measure, then restart the service so the updated definition takes effect:

```bash
ASDF_BIN="$(command -v asdf)"
asvc start web -c "$ASDF_BIN exec npm run dev" --cwd /absolute/path/to/web --port 3000 -d
asvc restart web
```

Updating the npm package does not replace an already running daemon. When upgrading asvc itself,
plan for a brief service interruption: stop the daemon, then start all registered services so the
new daemon code is loaded:

```bash
asvc daemon stop
asvc start --all
```

Finally verify the expected runtime and actual process executable. Determine the expected value
through the project's actual manager in a shell where that manager is loaded, then inspect the
service process independently:

```bash
# asdf project:
cd /absolute/path/to/web
asdf exec node -p 'process.version + " " + process.execPath'

# nvm project, in an nvm-initialized shell:
nvm use
node -p 'process.version + " " + process.execPath'

# authoritative managed process check, regardless of manager:
lsof -a -p <service-pid> -d txt
ps -p <service-pid> -o command=
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
