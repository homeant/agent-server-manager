use std::{
    collections::BTreeMap,
    io::{self, IsTerminal, Write},
    path::PathBuf,
    process::ExitCode,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Result, anyhow, bail};
use chrono::{Local, TimeZone};
use clap::{Args, Parser, Subcommand, ValueEnum};
use serde::Deserialize;
use serde_json::json;

use crate::{
    client::Client,
    model::{BatchResult, Event, LogLine, LogStream, ServiceInfo, ServiceSpec, ServiceStatus},
    paths::Paths,
    skill::{self, SkillTarget},
};

#[derive(Parser)]
#[command(
    name = "asvc",
    version,
    about = "本地开发服务管理器：人和 agent 通过同一个 daemon 管理服务",
    long_about = "本地开发服务管理器。asvc start 启动即注册；默认前台跟随，-d 后台运行。\n所有操作走同一个 daemon，并按服务名去重。"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// 启动服务；带 -c 时同时注册或更新定义
    Start(StartArgs),
    /// 停止服务，保留定义
    Stop(TargetArgs),
    /// 重启服务
    Restart { name: String },
    /// 查看服务日志
    Logs(LogsArgs),
    /// 列出服务及状态
    #[command(alias = "ls")]
    List,
    /// 查看单个服务的完整详情
    #[command(alias = "show")]
    Info { name: String },
    /// 停止并移除服务定义
    #[command(alias = "remove")]
    Rm(RemoveArgs),
    /// 输出 shell 补全脚本
    Completion { shell: Shell },
    /// 管理后台 daemon
    Daemon {
        #[command(subcommand)]
        command: DaemonCommand,
    },
    /// 安装和管理随当前 CLI 版本提供的 agent skill
    Skill {
        #[command(subcommand)]
        command: SkillCommand,
    },
    #[command(name = "__complete", hide = true)]
    Complete {
        #[command(subcommand)]
        command: CompleteCommand,
    },
}

#[derive(Args)]
struct StartArgs {
    name: Option<String>,
    #[arg(short = 'c', long = "cmd")]
    command: Option<String>,
    #[arg(short = 'w', long)]
    cwd: Option<PathBuf>,
    #[arg(short = 'p', long)]
    port: Option<u16>,
    #[arg(short = 'e', long = "env")]
    env: Vec<String>,
    #[arg(long)]
    autorestart: bool,
    #[arg(short = 'd', long)]
    detach: bool,
    #[arg(short = 'a', long)]
    all: bool,
}

#[derive(Args)]
struct TargetArgs {
    name: Option<String>,
    #[arg(short = 'a', long)]
    all: bool,
}

#[derive(Args)]
struct RemoveArgs {
    name: Option<String>,
    #[arg(short = 'a', long)]
    all: bool,
    #[arg(short = 'y', long)]
    yes: bool,
}

#[derive(Args)]
struct LogsArgs {
    name: String,
    #[arg(short = 'f', long)]
    follow: bool,
    #[arg(short = 'n', long, default_value_t = 200)]
    lines: usize,
}

#[derive(Subcommand)]
enum DaemonCommand {
    /// 检查 daemon 是否运行（不会自动启动）
    Status,
    /// 停止 daemon 及其管理的服务
    Stop,
}

#[derive(Subcommand)]
enum SkillCommand {
    /// 安装或更新由 asvc 托管的 skill
    Install(SkillTargetArgs),
    /// 查看内嵌版本和各安装目标状态
    Status,
    /// 卸载由 asvc 托管的 skill
    Uninstall(SkillTargetArgs),
}

#[derive(Args)]
struct SkillTargetArgs {
    /// 操作目标；可重复指定
    #[arg(long, value_enum, default_value = "codex")]
    target: Vec<SkillTarget>,
    /// 跳过确认；用于非交互环境
    #[arg(short = 'y', long)]
    yes: bool,
}

#[derive(Subcommand)]
enum CompleteCommand {
    Services,
}

#[derive(Clone, ValueEnum)]
enum Shell {
    Zsh,
    Bash,
}

#[derive(Deserialize)]
struct AttachResult {
    info: ServiceInfo,
    backlog: Vec<LogLine>,
}

pub async fn run() -> ExitCode {
    match execute(Cli::parse(), Paths::discover()).await {
        Ok(code) => ExitCode::from(code as u8),
        Err(error) => {
            eprintln!("错误: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn execute(cli: Cli, paths: Paths) -> Result<i32> {
    if !matches!(
        &cli.command,
        Commands::Skill { .. } | Commands::Complete { .. } | Commands::Completion { .. }
    ) {
        report_skill_sync(&paths);
    }
    match cli.command {
        Commands::List => {
            let mut client = Client::connect(&paths, true).await?;
            let services: Vec<ServiceInfo> = client.request(json!({ "type": "list" })).await?;
            print_list(&services);
            Ok(0)
        }
        Commands::Info { name } => {
            let mut client = Client::connect(&paths, true).await?;
            let info: ServiceInfo = client
                .request(json!({ "type": "info", "name": name }))
                .await?;
            print_info(&info);
            Ok(0)
        }
        Commands::Logs(args) => logs(args, &paths).await,
        Commands::Restart { name } => {
            let mut client = Client::connect(&paths, true).await?;
            let info: ServiceInfo = client
                .request(json!({ "type": "restart", "name": name }))
                .await?;
            report_start(&mut client, info, "重启").await
        }
        Commands::Stop(args) => stop(args, &paths).await,
        Commands::Start(args) => start(args, &paths).await,
        Commands::Rm(args) => remove(args, &paths).await,
        Commands::Completion { shell } => {
            println!("{}", completion(shell));
            Ok(0)
        }
        Commands::Complete {
            command: CompleteCommand::Services,
        } => {
            if let Ok(mut client) = Client::connect(&paths, false).await {
                let services: Vec<ServiceInfo> = client.request(json!({ "type": "list" })).await?;
                for service in services {
                    println!("{}", service.spec.name);
                }
            }
            Ok(0)
        }
        Commands::Daemon { command } => daemon(command, &paths).await,
        Commands::Skill { command } => manage_skill(command, &paths),
    }
}

fn manage_skill(command: SkillCommand, paths: &Paths) -> Result<i32> {
    match command {
        SkillCommand::Install(args) => {
            let installed = match skill::install(paths, &args.target, args.yes) {
                Ok(installed) => installed,
                Err(error) if error.downcast_ref::<skill::ModifiedSkill>().is_some() => {
                    let modified = error
                        .downcast_ref::<skill::ModifiedSkill>()
                        .expect("checked above");
                    let question = format!(
                        "{} 已被修改。继续安装将覆盖这些修改，是否继续？",
                        modified.path.display()
                    );
                    if !confirm(&question, false)? {
                        println!("已取消");
                        return Ok(0);
                    }
                    skill::install(paths, &args.target, true)?
                }
                Err(error) => return Err(error),
            };
            for target in installed {
                println!(
                    "{} skill {}: {}",
                    target.target.display_name(),
                    target.state,
                    target.path.display()
                );
            }
        }
        SkillCommand::Status => {
            let status = skill::status(paths)?;
            println!("内嵌 skill 版本: {}", status.bundled_version);
            println!("内嵌 SHA-256: {}", status.bundled_sha256);
            let Some(managed_version) = status.managed_version else {
                println!("托管状态: 未安装");
                return Ok(0);
            };
            println!("托管版本: {managed_version}");
            if status.targets.is_empty() {
                println!("安装目标: 无");
            } else {
                for target in status.targets {
                    println!(
                        "{}: {} ({})",
                        target.target.display_name(),
                        target.state,
                        target.path.display()
                    );
                }
            }
        }
        SkillCommand::Uninstall(args) => {
            if skill::status(paths)?.managed_version.is_none() {
                bail!("尚未安装由 asvc 托管的 skill");
            }
            let targets = args
                .target
                .iter()
                .map(|target| target.display_name())
                .collect::<Vec<_>>()
                .join(", ");
            let question =
                format!("将卸载 {targets} skill。请先备份需要保留的本地修改，是否继续？");
            if !confirm(&question, args.yes)? {
                println!("已取消");
                return Ok(0);
            }
            for target in skill::uninstall(paths, &args.target)? {
                println!(
                    "{} skill {}: {}",
                    target.target.display_name(),
                    target.state,
                    target.path.display()
                );
            }
        }
    }
    Ok(0)
}

fn confirm(question: &str, assume_yes: bool) -> Result<bool> {
    if assume_yes {
        return Ok(true);
    }
    if !io::stdin().is_terminal() || !io::stderr().is_terminal() {
        bail!("当前环境无法交互确认；确认操作后请重新执行并添加 --yes");
    }
    eprint!("{question} [y/N] ");
    io::stderr().flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    Ok(matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

fn report_skill_sync(paths: &Paths) {
    match skill::sync_if_installed(paths) {
        Ok(Some(report)) => {
            if !report.updated.is_empty() {
                let targets = report
                    .updated
                    .iter()
                    .map(|target| target.display_name())
                    .collect::<Vec<_>>()
                    .join(", ");
                eprintln!("skill 已随 asvc 自动同步: {targets}");
            }
            if let Some(path) = report.managed_modified {
                eprintln!(
                    "警告: {} 已被修改，asvc 已跳过 skill 自动同步",
                    path.display()
                );
            }
            for (target, path) in report.modified {
                eprintln!(
                    "警告: {} skill 路径 {} 不再由 asvc 安全托管，已跳过 skill 自动同步",
                    target.display_name(),
                    path.display()
                );
            }
        }
        Ok(None) => {}
        Err(error) => eprintln!("警告: skill 自动同步失败: {error}"),
    }
}

async fn start(args: StartArgs, paths: &Paths) -> Result<i32> {
    if args.all {
        if args.name.is_some()
            || args.command.is_some()
            || args.cwd.is_some()
            || args.port.is_some()
            || !args.env.is_empty()
            || args.autorestart
            || args.detach
        {
            bail!("--all 不能与服务名、-c/-w/-p/-e/--autorestart/-d 同时使用");
        }
        let mut client = Client::connect(paths, true).await?;
        let result: BatchResult = client.request(json!({ "type": "startAll" })).await?;
        return Ok(print_batch(&result));
    }
    let name = args
        .name
        .clone()
        .ok_or_else(|| anyhow!("请提供服务名，或使用 --all 启动全部服务"))?;
    let spec = args
        .command
        .as_ref()
        .map(|command| build_spec(&name, command, &args))
        .transpose()?;

    if args.detach {
        let mut client = Client::connect(paths, true).await?;
        let info: Result<ServiceInfo> = if let Some(spec) = spec {
            client
                .request(json!({ "type": "register", "spec": spec, "start": true }))
                .await
        } else {
            client
                .request(json!({ "type": "start", "name": name }))
                .await
        };
        let info = info.map_err(|error| improve_unknown_service(error, &name))?;
        return report_start(&mut client, info, "启动").await;
    }
    start_foreground(paths, &name, spec).await
}

fn build_spec(name: &str, command: &str, args: &StartArgs) -> Result<ServiceSpec> {
    let mut env = BTreeMap::new();
    for item in &args.env {
        let Some((key, value)) = item.split_once('=') else {
            bail!("环境变量格式应为 KEY=VAL: {item}");
        };
        env.insert(key.to_string(), value.to_string());
    }
    // Capture the registering shell's PATH once. The daemon may have been
    // started earlier by a different shell (for example a Codex non-login
    // shell), so inheriting the daemon's PATH would not represent this user.
    // An explicit --env PATH=... remains authoritative.
    if !env.contains_key("PATH")
        && let Ok(path) = std::env::var("PATH")
    {
        env.insert("PATH".into(), path);
    }
    let cwd = args
        .cwd
        .clone()
        .unwrap_or(std::env::current_dir()?)
        .to_string_lossy()
        .into_owned();
    Ok(ServiceSpec {
        name: name.into(),
        command: command.into(),
        cwd,
        env: (!env.is_empty()).then_some(env),
        port: args.port,
        autorestart: args.autorestart,
    })
}

fn improve_unknown_service(error: anyhow::Error, name: &str) -> anyhow::Error {
    if error.to_string().contains("未知服务") {
        anyhow!(
            "{error}\n该服务尚未注册。首次启动请用 -c 指定命令，如：asvc start {name} -c \"npm run dev\""
        )
    } else {
        error
    }
}

async fn start_foreground(paths: &Paths, name: &str, spec: Option<ServiceSpec>) -> Result<i32> {
    let mut client = Client::connect(paths, true).await?;
    if let Some(spec) = spec {
        let _: ServiceInfo = client
            .request(json!({ "type": "register", "spec": spec, "start": false }))
            .await?;
    }
    let attached: AttachResult = client
        .request(json!({ "type": "attach", "name": name, "backlog": 500 }))
        .await
        .map_err(|error| improve_unknown_service(error, name))?;
    for line in attached.backlog {
        print_log(&line);
    }
    let started: ServiceInfo = client
        .request(json!({ "type": "start", "name": name }))
        .await?;
    for event in client.drain_events() {
        print_event(&event);
    }
    if matches!(
        started.status,
        ServiceStatus::Exited | ServiceStatus::Errored
    ) {
        return Ok(started.last_exit_code.unwrap_or(1));
    }
    eprintln!("— {name} 前台运行中（Ctrl-C 停止服务）—");
    loop {
        tokio::select! {
            event = client.next_event() => {
                let event = event?;
                print_event(&event);
                if let Event::Status { info } = &event {
                    if info.spec.name != name || matches!(info.status, ServiceStatus::Running | ServiceStatus::Starting | ServiceStatus::Stopping) {
                        continue;
                    }
                    if info.restarting || (info.status == ServiceStatus::Exited && info.spec.autorestart) {
                        continue;
                    }
                    return Ok(if info.status == ServiceStatus::Exited {
                        info.last_exit_code.unwrap_or(1)
                    } else { 0 });
                }
            }
            _ = tokio::signal::ctrl_c() => {
                eprintln!("\n正在停止服务…");
                let _: ServiceInfo = client.request(json!({ "type": "stop", "name": name })).await?;
                return Ok(0);
            }
        }
    }
}

async fn logs(args: LogsArgs, paths: &Paths) -> Result<i32> {
    let mut client = Client::connect(paths, true).await?;
    if !args.follow {
        let lines: Vec<LogLine> = client
            .request(json!({ "type": "logs", "name": args.name, "lines": args.lines }))
            .await?;
        for line in lines {
            print_log(&line);
        }
        return Ok(0);
    }
    let attached: AttachResult = client
        .request(json!({ "type": "attach", "name": args.name, "backlog": args.lines }))
        .await?;
    let _ = attached.info;
    for line in attached.backlog {
        print_log(&line);
    }
    eprintln!("— 跟随中（Ctrl-C 退出，不影响服务）—");
    loop {
        tokio::select! {
            event = client.next_event() => print_event(&event?),
            _ = tokio::signal::ctrl_c() => return Ok(0),
        }
    }
}

async fn stop(args: TargetArgs, paths: &Paths) -> Result<i32> {
    if args.all && args.name.is_some() {
        bail!("服务名与 --all 不能同时使用");
    }
    let mut client = Client::connect(paths, true).await?;
    if args.all {
        let result: BatchResult = client.request(json!({ "type": "stopAll" })).await?;
        return Ok(print_batch(&result));
    }
    let name = args
        .name
        .ok_or_else(|| anyhow!("请提供服务名，或使用 --all 停止全部服务"))?;
    let info: ServiceInfo = client
        .request(json!({ "type": "stop", "name": name }))
        .await?;
    println!("{} 已停止 → {}", info.spec.name, info.status.as_str());
    Ok(0)
}

async fn remove(args: RemoveArgs, paths: &Paths) -> Result<i32> {
    if args.all && args.name.is_some() {
        bail!("服务名与 --all 不能同时使用");
    }
    if args.yes && !args.all {
        bail!("--yes 只能与 --all 一起使用");
    }
    let mut client = Client::connect(paths, true).await?;
    if args.all {
        if !args.yes {
            let services: Vec<ServiceInfo> = client.request(json!({ "type": "list" })).await?;
            if services.is_empty() {
                println!("暂无已注册服务，无需操作。");
                return Ok(0);
            }
            eprintln!("即将停止并删除 {} 个已注册服务。", services.len());
            eprintln!("服务日志不会删除。");
            eprintln!("确认执行请使用：asvc rm --all --yes");
            return Ok(2);
        }
        let result: BatchResult = client.request(json!({ "type": "removeAll" })).await?;
        return Ok(print_batch(&result));
    }
    let name = args
        .name
        .ok_or_else(|| anyhow!("请提供服务名，或使用 --all 删除全部注册"))?;
    let _: serde_json::Value = client
        .request(json!({ "type": "remove", "name": name }))
        .await?;
    println!("{name} 已移除");
    Ok(0)
}

async fn daemon(command: DaemonCommand, paths: &Paths) -> Result<i32> {
    match command {
        DaemonCommand::Status => match Client::connect(paths, false).await {
            Ok(mut client) => {
                let _: serde_json::Value = client.request(json!({ "type": "ping" })).await?;
                println!("daemon 运行中");
                Ok(0)
            }
            Err(_) => {
                println!("daemon 未运行");
                Ok(0)
            }
        },
        DaemonCommand::Stop => match Client::connect(paths, false).await {
            Ok(mut client) => {
                let _: serde_json::Value = client.request(json!({ "type": "shutdown" })).await?;
                println!("daemon 已停止（所有服务已关闭）");
                Ok(0)
            }
            Err(_) => {
                println!("daemon 未运行");
                Ok(0)
            }
        },
    }
}

async fn report_start(client: &mut Client, info: ServiceInfo, verb: &str) -> Result<i32> {
    if matches!(
        info.status,
        ServiceStatus::Running | ServiceStatus::Starting
    ) {
        if let Some(pid) = info.pid {
            println!(
                "{} {} → {} (pid {pid})",
                info.spec.name,
                verb,
                info.status.as_str()
            );
        } else {
            println!("{} {} → {}", info.spec.name, verb, info.status.as_str());
        }
        return Ok(0);
    }
    eprintln!(
        "{} {}失败 → {}{}",
        info.spec.name,
        verb,
        info.status.as_str(),
        info.last_exit_code
            .map(|code| format!(" (exit {code})"))
            .unwrap_or_default()
    );
    eprintln!("最近日志：");
    let lines: Vec<LogLine> = client
        .request(json!({ "type": "logs", "name": info.spec.name, "lines": 20 }))
        .await
        .unwrap_or_default();
    for line in &lines {
        eprintln!("  {}", line.line);
    }
    if lines.iter().any(|line| {
        let lower = line.line.to_ascii_lowercase();
        lower.contains("eaddrinuse") || lower.contains("address already in use")
    }) {
        eprintln!("端口疑似被占用（EADDRINUSE）：请换端口，或先停掉占用该端口的进程后重试。");
    }
    Ok(1)
}

fn print_list(services: &[ServiceInfo]) {
    if services.is_empty() {
        println!("（暂无服务。用 asvc start <name> -c \"<命令>\" 启动一个）");
        return;
    }
    println!(
        "NAME              STATUS     PID      CPU     MEMORY   UPTIME   PORT    AUTO_RESTARTS  CWD  COMMAND"
    );
    for service in services {
        println!(
            "{:<17} {:<10} {:<8} {:<7} {:<8} {:<8} {:<7} {:<14} {}  {}",
            service.spec.name,
            service.status.as_str(),
            service
                .pid
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".into()),
            service
                .cpu_percent
                .map(|value| format!("{value:.1}%"))
                .unwrap_or_else(|| "-".into()),
            service
                .memory_bytes
                .map(format_memory)
                .unwrap_or_else(|| "-".into()),
            format_uptime(
                service
                    .started_at
                    .filter(|_| service.status == ServiceStatus::Running)
            ),
            service
                .spec
                .port
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".into()),
            service.restarts,
            shorten(&service.spec.cwd, 36),
            truncate(&service.spec.command, 40),
        );
    }
}

fn print_info(service: &ServiceInfo) {
    println!("名称: {}", service.spec.name);
    println!("状态: {}", service.status.as_str());
    println!(
        "PID: {}",
        service
            .pid
            .map(|value| value.to_string())
            .unwrap_or_else(|| "-".into())
    );
    println!(
        "CPU: {}",
        service
            .cpu_percent
            .map(|value| format!("{value:.1}%"))
            .unwrap_or_else(|| "-".into())
    );
    println!(
        "内存: {}",
        service
            .memory_bytes
            .map(format_memory)
            .unwrap_or_else(|| "-".into())
    );
    println!(
        "启动时间: {}",
        service
            .started_at
            .map(format_time)
            .unwrap_or_else(|| "-".into())
    );
    println!(
        "运行时长: {}",
        format_uptime(
            service
                .started_at
                .filter(|_| service.status == ServiceStatus::Running)
        )
    );
    println!(
        "最后退出码: {}",
        service
            .last_exit_code
            .map(|value| value.to_string())
            .unwrap_or_else(|| "-".into())
    );
    println!(
        "最后退出信号: {}",
        service.last_exit_signal.as_deref().unwrap_or("-")
    );
    println!("重启次数: {}", service.restarts);
    println!(
        "自动重启: {}",
        if service.spec.autorestart {
            "是"
        } else {
            "否"
        }
    );
    println!("正在重启: {}", if service.restarting { "是" } else { "否" });
    println!(
        "端口: {}",
        service
            .spec
            .port
            .map(|value| value.to_string())
            .unwrap_or_else(|| "-".into())
    );
    println!("工作目录: {}", service.spec.cwd);
    println!("命令: {}", service.spec.command);
    println!("环境变量:");
    if let Some(env) = &service.spec.env {
        for (key, value) in env {
            println!("  {key}={value}");
        }
    } else {
        println!("  （无）");
    }
}

fn print_batch(result: &BatchResult) -> i32 {
    if result.items.is_empty() {
        println!("暂无已注册服务，无需操作。");
        return 0;
    }
    println!("{} 全部服务（{}）", result.action, result.items.len());
    let mut failed = 0;
    for item in &result.items {
        let detail = item
            .error
            .clone()
            .or_else(|| item.reason.clone())
            .unwrap_or_else(|| {
                item.info
                    .as_ref()
                    .map(|info| info.status.as_str().to_string())
                    .unwrap_or_default()
            });
        println!(
            "{} {:<17} {:<10} {}",
            if item.outcome == "failed" {
                "✗"
            } else {
                "✓"
            },
            item.name,
            item.outcome,
            detail
        );
        if item.outcome == "failed" {
            failed += 1;
        }
    }
    if failed == 0 { 0 } else { 1 }
}

fn print_event(event: &Event) {
    match event {
        Event::Log {
            name,
            stream,
            line,
            ts,
        } => print_log(&LogLine {
            name: name.clone(),
            stream: stream.clone(),
            line: line.clone(),
            ts: *ts,
        }),
        Event::Status { info } => println!(
            "{} [{}] {}",
            format_time(now_ms()),
            info.spec.name,
            info.status.as_str()
        ),
    }
}

fn print_log(line: &LogLine) {
    let marker = if matches!(line.stream, LogStream::System) {
        "» "
    } else {
        ""
    };
    println!("{} {marker}{}", format_time(line.ts), line.line);
}

fn format_time(timestamp: i64) -> String {
    Local
        .timestamp_millis_opt(timestamp)
        .single()
        .map(|time| time.format("%Y-%m-%d %H:%M:%S%.3f").to_string())
        .unwrap_or_else(|| timestamp.to_string())
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn format_uptime(started_at: Option<i64>) -> String {
    let Some(started_at) = started_at else {
        return "-".into();
    };
    let seconds = ((now_ms() - started_at).max(0) / 1_000) as u64;
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 3_600 {
        format!("{}m{}s", seconds / 60, seconds % 60)
    } else {
        format!("{}h{}m", seconds / 3_600, (seconds / 60) % 60)
    }
}

fn format_memory(bytes: u64) -> String {
    let mib = bytes as f64 / 1_048_576.0;
    if mib < 1_024.0 {
        format!("{mib:.0}M")
    } else {
        format!("{:.1}G", mib / 1_024.0)
    }
}

fn shorten(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        return value.into();
    }
    let keep = (max.saturating_sub(1)) / 2;
    let start: String = value.chars().take(max - keep - 1).collect();
    let end: String = value
        .chars()
        .rev()
        .take(keep)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    format!("{start}…{end}")
}

fn truncate(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        value.into()
    } else {
        format!("{}…", value.chars().take(max - 1).collect::<String>())
    }
}

fn completion(shell: Shell) -> &'static str {
    match shell {
        Shell::Zsh => ZSH_COMPLETION,
        Shell::Bash => BASH_COMPLETION,
    }
}

const ZSH_COMPLETION: &str = r#"_asvc() {
  local -a commands
  commands=('start:启动服务' 'stop:停止服务' 'restart:重启服务' 'logs:查看日志' 'list:列出服务' 'info:查看服务详情' 'rm:移除服务' 'daemon:管理 daemon' 'skill:管理 agent skill')
  if (( CURRENT == 2 )); then _describe 'command' commands; return; fi
  local cmd=${words[2]}
  if (( CURRENT == 3 )) && [[ $cmd == skill ]]; then
    local -a actions
    actions=('install:安装 skill' 'status:查看 skill 状态' 'uninstall:卸载 skill')
    _describe 'action' actions
    return
  fi
  if (( CURRENT == 3 )) && [[ $cmd == start || $cmd == stop || $cmd == restart || $cmd == logs || $cmd == info || $cmd == show || $cmd == rm || $cmd == remove ]]; then
    local -a services
    services=( ${(f)"$(command asvc __complete services 2>/dev/null)"} )
    [[ $cmd == start || $cmd == stop || $cmd == rm || $cmd == remove ]] && services+=(--all)
    _describe 'service' services
  fi
}
compdef _asvc asvc"#;

const BASH_COMPLETION: &str = r#"_asvc() {
  local cur="${COMP_WORDS[COMP_CWORD]}"
  if [ "$COMP_CWORD" -eq 1 ]; then
    COMPREPLY=( $(compgen -W "start stop restart logs list ls info show rm daemon skill completion" -- "$cur") )
    return
  fi
  local cmd="${COMP_WORDS[1]}"
  if [ "$COMP_CWORD" -eq 2 ]; then
    case "$cmd" in
      start|stop|rm|remove)
        COMPREPLY=( $(compgen -W "$(asvc __complete services 2>/dev/null) --all" -- "$cur") ) ;;
      restart|logs|info|show)
        COMPREPLY=( $(compgen -W "$(asvc __complete services 2>/dev/null)" -- "$cur") ) ;;
      daemon) COMPREPLY=( $(compgen -W "status stop" -- "$cur") ) ;;
      skill) COMPREPLY=( $(compgen -W "install status uninstall" -- "$cur") ) ;;
    esac
  fi
}
complete -F _asvc asvc"#;
