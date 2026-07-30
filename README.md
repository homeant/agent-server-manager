# asvc

统一管理本地开发服务的 **守护进程(daemon) + 一套 `asvc` CLI**。人和 agent 共用同一套命令。

解决的问题：开发时 agent 会帮你启动服务，你自己也会启动，导致**端口冲突**、**进程难管理**。
本工具让人和 agent 都通过**同一个 daemon** 操作服务：

- **启动即注册**：只有一个 `asvc start`。带 `-c` 给命令就自动注册并启动，按名去重不重复拉起。
- **人**（`asvc start <name> -c "..."`）：前台运行，像直接敲 `npm run dev` 一样实时刷日志，Ctrl-C 停。
- **agent**（`asvc start <name> -c "..." -d`）：加 `-d` 后台启动、跑完返回拿退出码。
- 任何人都能 `asvc logs` / `asvc list` / `asvc restart` / `asvc stop` 操作同一批服务。
- daemon 按**服务名去重**：同名服务只有一个实例，agent 和你都不会把它启动两遍 → 不再撞端口。

同一个 `asvc start`，靠 `-d` 区分前台/后台：

| 命令 | 谁用 | 行为 |
| --- | --- | --- |
| `asvc start <name> -c "..."` | 人 | **前台**：占住终端实时刷日志，像原始命令；Ctrl-C 停止服务 |
| `asvc start <name> -c "..." -d` | agent | **后台**：启动后立即返回，带退出码（成功 0 / 启动失败 1） |

> 为什么是 CLI 而不是 MCP：agent（如 Claude Code）本来就有 shell，直接跑 `asvc` 命令最简单——
> 不用配 MCP server、不用单独的协议，人和 agent 认知统一，看到的输出也一样。

## 核心设计：重启不断开你的终端

服务进程是 **daemon 的子进程**，不是某个终端 shell 的子进程。
你在终端 `asvc start web` 前台跑着（或在另一个终端 `asvc logs web -f` 跟随）时，
agent 在另一头 `asvc restart web`，daemon 只是把底层进程换了一个，
**你的终端不会断**——只会看到一行 `» restarting...` 然后新日志继续刷。
（注意：只有你自己按 Ctrl-C 才会停服务并退出；agent 的 restart 不会让你的终端退出。）

```
   人的终端  ──┐
   (asvc CLI)   │        ┌─────────────┐      ┌── 服务进程 web (pid A)
               ├─ IPC ─▶│   daemon    │─────▶├── 服务进程 api (pid B)
   agent     ──┘ (sock) │  (单实例)    │      └── ...
   (asvc CLI)            └─────────────┘
```

## 安装

### Homebrew（macOS，推荐）

通过 Homeant 的标准 GitHub tap 安装；直接安装会自动添加并仅信任这个 Formula：

```bash
brew install homeant/tap/asvc
```

升级使用 `brew upgrade homeant/tap/asvc`。公式会按 Apple Silicon 或 Intel Mac 下载对应的
GitHub Release 原生二进制，并校验 SHA-256。手动推送 `v*` tag 后，发布工作流会在 Release
成功创建后同步更新 [`homeant/homebrew-tap`](https://github.com/homeant/homebrew-tap)。

### GitHub Release 原生文件

asvc 核心使用 Rust，CLI 和 daemon 是同一个原生可执行文件，不依赖客户机器上的 Node、asdf、
nvm、fnm 或 Volta。每个 tag 发布以下产物：

| 系统 | 架构 | Release 资产 |
| --- | --- | --- |
| macOS 11+ | arm64 | `asvc-v<version>-darwin-arm64.tar.gz` |
| macOS 11+ | x64 | `asvc-v<version>-darwin-x64.tar.gz` |
| Linux | arm64 | `asvc-v<version>-linux-arm64.tar.gz` |
| Linux | x64 | `asvc-v<version>-linux-x64.tar.gz` |
| Windows | x64 | `asvc-v<version>-win32-x64.zip` |

Linux 产物使用 musl 静态链接，以覆盖常见 glibc 和 musl 发行版。下载后用同一 Release 中的
`SHA256SUMS` 校验，再把 `asvc`（Windows 为 `asvc.exe`）放进稳定的 PATH 目录。

本机从源码构建当前平台：

```bash
cargo test --locked
cargo build --release --locked
node scripts/package-platform.mjs
```

### npm 安装

```bash
npm install -g @homeant/asvc
```

npm 主包只包含一个很小的启动器，并通过 optional dependency 选择当前系统对应的原生包：
macOS arm64/x64、Linux arm64/x64 或 Windows x64。启动器立即执行 Rust 二进制；CLI 核心、
daemon 和服务监督都不在 Node 中运行。daemon 会在首次执行需要它的 `asvc` 命令时自动拉起。

npm 的全局命令遵循标准 prefix 规则：包安装到当前 Node 的
`{prefix}/lib/node_modules`，命令链接到 `{prefix}/bin`。因此使用 npm 渠道时，asdf/nvm
管理的不同 Node prefix 仍可能有各自的全局安装；这是 npm 安装入口的属性，不是 Rust
daemon 的运行时依赖。需要完全绕开 Node manager 时，直接安装上面的原生文件。

### 非登录 shell

Codex/agent 的非登录 shell 可能不加载用户 profile，因此 `PATH` 里可能没有 npm 全局 bin
或项目运行时。把原生 `asvc` 安装到非登录 shell 已包含的稳定目录，即可让控制命令不依赖
任何 Node manager。

服务运行环境采用通用规则，不识别 asdf、nvm、fnm 或项目版本文件：

- `start <name> -c ...` 注册或更新服务时，保存调用者当前的 `PATH`。
- 后续 `restart` 和 daemon 重启后的 `start` 继续使用这个 PATH。
- `--env PATH=...` 显式值优先。
- Codex 不得给 shell 传 `-i`，包括首次注册。交互 shell 会加载不可控的 prompt、插件、
  别名和启动脚本，不适合作为自动化环境。

因此 `cwd` 只负责工作目录，不会自行解释 `.tool-versions` 或 `.nvmrc`；运行时选择由注册时
的用户环境或显式服务命令负责。非登录 PATH 不正确时，应使用确认过的绝对运行时、
`--env PATH=...`、已知的 manager 非交互命令，或者让用户在自己的终端完成首次注册；不要
用 `-i` 隐式加载整套用户配置。

### Shell 补全（Tab 提示）

```bash
# zsh：加到 ~/.zshrc
eval "$(asvc completion zsh)"
# bash：加到 ~/.bashrc
eval "$(asvc completion bash)"
```

补全覆盖子命令、各命令的 flag，以及**服务名**——`asvc restart <TAB>` 会实时列出
daemon 里已注册的服务（daemon 未运行时安静地不补全，不会触发自动拉起）。

从源码开发时直接构建 Rust 二进制：

```bash
cargo test --locked
cargo build --release --locked
./target/release/asvc --version
```

## 命令一览（`asvc`）

```bash
# 启动即注册（前台，人用）：像直接敲原始命令，实时刷日志，Ctrl-C 停止服务
asvc start web -c "npm run dev" --port 3000              # cwd 默认当前目录
asvc start web                                           # 已注册过则可省略 -c

# 启动即注册（后台，agent 用）：加 -d，跑完即返回、带退出码
asvc start web -c "npm run dev" --port 3000 -d
asvc start api -c "go run ." --cwd /path/api --env KEY=VAL --autorestart -d

asvc list                 # 列出所有服务、状态及进程组 CPU/内存用量
asvc info web             # 查看 web 的完整详情（也可用 asvc show web）
asvc logs web             # 看 web 最近 200 行日志
asvc logs web -f          # 持续跟随（agent 重启服务也不断开）
asvc logs web -n 500      # 看最近 500 行
asvc restart web          # 重启（前台/跟随的终端都不会断）
asvc stop web             # 停止（保留定义，可再启动）
asvc rm web               # 移除（先停后删）

asvc start --all          # 后台启动所有已注册服务，完成后返回汇总
asvc stop --all           # 停止所有服务，保留定义
asvc rm --all --yes       # 停止并删除所有注册（日志保留）

asvc daemon status        # 看 daemon 是否运行
asvc daemon stop          # 停 daemon（会先关闭所有服务）

asvc skill install                         # 安装当前版本内嵌的 skill 到 Codex
asvc skill install --target claude         # 安装到 Claude Code
asvc skill status                          # 查看内嵌/托管版本和同步状态
```

`start` 参数：`-c/--cmd`（命令，经 shell 执行）、`-w/--cwd`（默认当前目录）、
`-p/--port`（仅展示/排查）、`-e/--env KEY=VAL`（可多个）、`--autorestart`、`-d/--detach`（后台）。

### 批量管理

批量操作复用原有命令的 `--all` 参数，不需要记额外的 `start-all` 命令：

```bash
asvc start --all          # 始终是后台批量模式，不会占住终端跟随日志
asvc stop --all           # 已停止的服务会跳过
asvc rm --all             # 只预览影响范围，不执行
asvc rm --all --yes       # 确认：先停止，再删除所有注册
```

daemon 会在一次批量请求开始时固定服务名单，最多同时处理 4 个服务；单个服务失败
不会阻断其他服务。结果按注册顺序逐项展示，只要有一项失败，命令退出码就是 1。
空注册表属于成功的无操作，退出码为 0。

`rm --all --yes` 只删除服务定义，保留 `$ASVC_HOME/logs` 中的历史日志；如果某个
运行中服务停止失败，该项不会从注册表移除，以免留下无人管理的进程。`start --all`
不能与 `-c/-w/-p/-e/--autorestart` 同时使用。

## 启动失败 / 端口冲突，agent 当场知道

`register` / `start` / `restart` 在拉起进程后会**观察约 1 秒**确认进程没有立刻崩溃，再返回：

- 进程稳定运行 → 打印成功，**退出码 0**。
- 进程在窗口内退出（最常见就是端口被别的进程占了，`EADDRINUSE`）→ 打印最近日志，
  识别到 `EADDRINUSE` 时给出「请换端口或先停掉占用方」的提示，并以**退出码 1** 退出。

这样不依赖事先声明 `port`：无论端口被谁占（你手动起的、孤儿进程、还是另一个服务），
只要进程因此启动失败，走 Bash 的 agent 都能直接从**非零退出码 + 输出**判定失败和原因，
而不是拿到一个乐观的 “running” 再去翻日志。`asvc start`（前台）同样：启动即失败会打印提示并以非零码退出。

## 让 agent 用起来（Codex / Claude Code Skill）

npm 包和原生二进制都内嵌同版本的 [`skill/asvc/SKILL.md`](skill/asvc/SKILL.md)。
它教 agent 在启动、查询、重启和查看日志时使用 `asvc`，并强调后台启动必须带 `-d`、
按退出码判断成败。

首次安装需要显式选择目标；`--target` 可以重复：

```bash
asvc skill install
asvc skill install --target claude
asvc skill install --target codex --target claude
```

Codex 使用当前标准用户目录 `~/.agents/skills/asvc`，Claude Code 使用
`~/.claude/skills/asvc`。macOS/Linux 在目标目录创建指向 `$ASVC_HOME/skills/asvc`
的软连接；Windows 不依赖 Developer Mode 或管理员权限，使用受管文件副本。

安装后，升级版 CLI 首次执行常规服务命令时，会根据版本和 SHA-256 自动同步
asvc 托管且未经修改的 skill。
如果目标路径原本已存在且不归 asvc 托管，安装会拒绝覆盖。受管内容被用户修改后，
自动同步会跳过；主动安装时可以确认覆盖。使用 `asvc skill status` 查看状态。
`asvc skill uninstall --target <codex|claude>` 始终要求确认；非交互环境需要显式添加
`--yes`。

之后 agent 在「启动服务 / 重启 / 看日志 / 排查起不来」时会自动参考该 skill。
也可以在项目 `CLAUDE.md` 里加一句兜底约定：

```md
启动/重启/停止开发服务一律用 `asvc`，agent 用 `asvc start <name> -c "<命令>" -d`
（务必带 -d 后台），重启 `asvc restart <name>`，看日志 `asvc logs <name> -n 200`。
```

## 工作流示例

1. 你在 web 目录 `asvc start web -c "npm run dev" --port 3000` → 前台跑起来，实时刷日志。
   （或让 agent `asvc start web -c "npm run dev" --port 3000 -d` 后台起，你再 `asvc logs web -f` 看。）
2. agent 改完代码执行 `asvc restart web`
   → 你的终端看到 `» restarting...`，随后新日志继续，**终端不断开**。
3. 你不想看了按 Ctrl-C → 服务停止、终端退出（和原始命令一致）。
4. 你随时 `asvc restart web` / `asvc stop web` 手动接管。

## 数据与配置

- 全局单实例，家目录默认 `~/.asvc`（可用 `ASVC_HOME` 覆盖）。
- IPC：macOS/Linux 使用 `$ASVC_HOME/daemon.sock`（可用 `ASVC_SOCKET` 覆盖）；Windows
  使用仅监听 loopback 的动态 TCP 端口，并把端口写入 `$ASVC_HOME/daemon.port`。
- 每个服务日志落盘 `$ASVC_HOME/logs/<name>.log`，内存保留最近 2000 行供快速查询。
- 受管 skill 和安装清单保存在 `$ASVC_HOME/skills/asvc`。
- 服务定义**动态注册**，无需手写配置文件；注册表持久化在 `$ASVC_HOME/registry.json`，
  daemon 重启后自动恢复定义（不自动拉起进程，`asvc start <name>` 即可再启动，无需重新带 `-c`）。

## 实现要点

- IPC：Unix domain socket / Windows loopback TCP + newline-delimited JSON（请求/响应 + 事件推送）。
- 进程树：macOS/Linux 使用独立进程组并先 `SIGTERM`、超时再 `SIGKILL`；Windows 使用
  `taskkill /T`，超时后增加 `/F`。
- 服务环境：注册定义时保存调用者 PATH，daemon 继承其余通用环境，服务 `--env` 最后覆盖；
  核心不包含任何 Node 版本管理器适配。
- 启动后 1s 稳定窗口：进程在窗口内退出即判失败，返回真实状态而非乐观 running。
- Rust + Tokio；macOS、Linux、Windows 每个平台产物都是单个原生可执行文件。
```
