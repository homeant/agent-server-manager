use std::{
    collections::BTreeSet,
    error::Error,
    fmt,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use anyhow::{Context, Result, anyhow, bail};
use clap::ValueEnum;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    i18n::{Locale, locale, text},
    paths::{Paths, user_home},
};

pub const BUNDLED_SKILL: &str = include_str!("../skill/asvc-service-manager/SKILL.md");

const SCHEMA_VERSION: u32 = 1;
const MANAGED_SKILL_ID: &str = "asvc";
pub const DEFAULT_SKILL_NAME: &str = "asvc-service-manager";
const MANIFEST_NAME: &str = "install.json";
const SKILL_FILE: &str = "SKILL.md";
static TEMP_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum SkillTarget {
    Codex,
    Claude,
}

impl SkillTarget {
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::Claude => "Claude Code",
        }
    }

    fn install_dir(self, skill_name: &str) -> Result<PathBuf> {
        let home = user_home();
        if home == Path::new(".") {
            bail!(
                "{}",
                text(
                    "could not determine the user home; set HOME or USERPROFILE",
                    "无法确定用户主目录，请设置 HOME 或 USERPROFILE"
                )
            );
        }
        Ok(match self {
            Self::Codex => home.join(".agents").join("skills").join(skill_name),
            Self::Claude => home.join(".claude").join("skills").join(skill_name),
        })
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InstallManifest {
    schema_version: u32,
    skill_name: String,
    cli_version: String,
    bundled_sha256: String,
    targets: Vec<TargetRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TargetRecord {
    target: SkillTarget,
    path: PathBuf,
    installed_sha256: String,
}

#[derive(Debug, Default)]
pub struct SyncReport {
    pub updated: Vec<SkillTarget>,
    pub modified: Vec<(SkillTarget, PathBuf)>,
    pub managed_modified: Option<PathBuf>,
}

pub struct TargetStatus {
    pub target: SkillTarget,
    pub path: PathBuf,
    pub state: &'static str,
}

pub struct SkillStatus {
    pub bundled_version: &'static str,
    pub bundled_sha256: String,
    pub skill_name: String,
    pub managed_version: Option<String>,
    pub targets: Vec<TargetStatus>,
}

#[derive(Debug)]
pub struct ModifiedSkill {
    pub path: PathBuf,
}

impl fmt::Display for ModifiedSkill {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} {}",
            self.path.display(),
            text("was modified", "已被修改")
        )
    }
}

impl Error for ModifiedSkill {}

pub fn install(
    paths: &Paths,
    targets: &[SkillTarget],
    requested_name: Option<&str>,
    overwrite_modified: bool,
) -> Result<Vec<TargetStatus>> {
    let targets = unique_targets(targets);
    if targets.is_empty() {
        bail!(
            "{}",
            text(
                "specify at least one skill installation target",
                "请至少指定一个 skill 安装目标"
            )
        );
    }
    if let Some(name) = requested_name {
        validate_skill_name(name)?;
    }
    with_lock(paths, || {
        install_locked(paths, &targets, requested_name, overwrite_modified)
    })
}

pub fn uninstall(paths: &Paths, targets: &[SkillTarget]) -> Result<Vec<TargetStatus>> {
    let targets = unique_targets(targets);
    if targets.is_empty() {
        bail!(
            "{}",
            text(
                "specify at least one skill uninstall target",
                "请至少指定一个 skill 卸载目标"
            )
        );
    }
    if !manifest_path(paths).exists() {
        bail!(
            "{}",
            text(
                "no asvc-managed skill is installed",
                "尚未安装由 asvc 托管的 skill"
            )
        );
    }
    with_lock(paths, || uninstall_locked(paths, &targets))
}

pub fn status(paths: &Paths) -> Result<SkillStatus> {
    let managed_root = absolute_path(&managed_dir(paths))?;
    let path = manifest_path(paths);
    if !path.exists() {
        let bundled_skill = render_skill(DEFAULT_SKILL_NAME)?;
        return Ok(SkillStatus {
            bundled_version: env!("CARGO_PKG_VERSION"),
            bundled_sha256: sha256(bundled_skill.as_bytes()),
            skill_name: DEFAULT_SKILL_NAME.into(),
            managed_version: None,
            targets: Vec::new(),
        });
    }
    let manifest = load_manifest(&path)?;
    validate_manifest(&manifest)?;
    let bundled_skill = render_skill(&manifest.skill_name)?;
    let bundled_sha256 = sha256(bundled_skill.as_bytes());
    let targets = manifest
        .targets
        .iter()
        .map(|record| TargetStatus {
            target: record.target,
            path: record.path.clone(),
            state: target_state(record, &manifest, &managed_root, &bundled_sha256),
        })
        .collect();
    Ok(SkillStatus {
        bundled_version: env!("CARGO_PKG_VERSION"),
        bundled_sha256,
        skill_name: manifest.skill_name,
        managed_version: Some(manifest.cli_version),
        targets,
    })
}

pub fn sync_if_installed(paths: &Paths) -> Result<Option<SyncReport>> {
    if !manifest_path(paths).exists() {
        return Ok(None);
    }
    with_lock(paths, || sync_locked(paths)).map(Some)
}

fn install_locked(
    paths: &Paths,
    targets: &[SkillTarget],
    requested_name: Option<&str>,
    overwrite_modified: bool,
) -> Result<Vec<TargetStatus>> {
    let manifest_file = manifest_path(paths);
    let mut manifest = if manifest_file.exists() {
        let manifest = load_manifest(&manifest_file)?;
        validate_manifest(&manifest)?;
        if let Some(requested_name) = requested_name
            && requested_name != manifest.skill_name
        {
            if locale() == Locale::English {
                bail!(
                    "the skill is installed as {}; uninstall it before changing its name",
                    manifest.skill_name
                );
            } else {
                bail!(
                    "skill 已按名称 {} 安装；如需改名，请先卸载后重新安装",
                    manifest.skill_name
                );
            }
        }
        if managed_file_modified(paths, &manifest)? && !overwrite_modified {
            return Err(ModifiedSkill {
                path: managed_skill_path(paths),
            }
            .into());
        }
        manifest
    } else {
        if path_exists(&managed_dir(paths)) {
            bail!(
                "{} {}",
                managed_dir(paths).display(),
                text(
                    "exists without an asvc manifest; back it up and move it before retrying",
                    "已存在但缺少 asvc 托管清单；请先备份并移走该目录"
                )
            );
        }
        InstallManifest {
            schema_version: SCHEMA_VERSION,
            skill_name: requested_name.unwrap_or(DEFAULT_SKILL_NAME).into(),
            cli_version: env!("CARGO_PKG_VERSION").into(),
            bundled_sha256: String::new(),
            targets: Vec::new(),
        }
    };
    let bundled_skill = render_skill(&manifest.skill_name)?;
    let bundled_sha256 = sha256(bundled_skill.as_bytes());
    let managed_root = absolute_path(&managed_dir(paths))?;

    // Validate all targets first so a collision cannot leave a partial
    // installation that is absent from the manifest.
    for target in targets {
        let install_dir = target.install_dir(&manifest.skill_name)?;
        let record = manifest
            .targets
            .iter()
            .find(|record| record.target == *target);
        if let Some(record) = record
            && record.path != install_dir
        {
            if locale() == Locale::English {
                bail!(
                    "{} skill is managed at {}, but the current home resolves to {}",
                    target.display_name(),
                    record.path.display(),
                    install_dir.display()
                );
            } else {
                bail!(
                    "{} skill 已由 asvc 托管在 {}，当前主目录解析为 {}",
                    target.display_name(),
                    record.path.display(),
                    install_dir.display()
                );
            }
        }
        preflight_install_target(
            record,
            &install_dir,
            &managed_root,
            &bundled_sha256,
            overwrite_modified,
        )?;
    }

    write_if_changed(&managed_skill_path(paths), bundled_skill.as_bytes())?;
    let mut installed = Vec::new();
    for target in targets {
        let install_dir = target.install_dir(&manifest.skill_name)?;
        install_target(
            &install_dir,
            &managed_root,
            bundled_skill.as_bytes(),
            &bundled_sha256,
        )?;
        if let Some(record) = manifest
            .targets
            .iter_mut()
            .find(|record| record.target == *target)
        {
            record.installed_sha256.clone_from(&bundled_sha256);
        } else {
            manifest.targets.push(TargetRecord {
                target: *target,
                path: install_dir.clone(),
                installed_sha256: bundled_sha256.clone(),
            });
        }
        installed.push(TargetStatus {
            target: *target,
            path: install_dir,
            state: text("installed", "已安装"),
        });
    }
    manifest.targets.sort_by_key(|record| record.target);
    manifest.cli_version = env!("CARGO_PKG_VERSION").into();
    manifest.bundled_sha256 = bundled_sha256;
    save_manifest(&manifest_file, &manifest)?;
    Ok(installed)
}

fn uninstall_locked(paths: &Paths, targets: &[SkillTarget]) -> Result<Vec<TargetStatus>> {
    let manifest_file = manifest_path(paths);
    let mut manifest = load_manifest(&manifest_file)?;
    validate_manifest(&manifest)?;
    let managed_root = absolute_path(&managed_dir(paths))?;

    for target in targets {
        if let Some(record) = manifest
            .targets
            .iter()
            .find(|record| record.target == *target)
        {
            preflight_uninstall_target(record, &managed_root)?;
        }
    }

    let mut removed = Vec::new();
    for target in targets {
        let Some(index) = manifest
            .targets
            .iter()
            .position(|record| record.target == *target)
        else {
            removed.push(TargetStatus {
                target: *target,
                path: target.install_dir(&manifest.skill_name)?,
                state: text("not installed", "未安装"),
            });
            continue;
        };
        let record = manifest.targets.remove(index);
        remove_target(&record.path)?;
        removed.push(TargetStatus {
            target: *target,
            path: record.path,
            state: text("uninstalled", "已卸载"),
        });
    }

    if manifest.targets.is_empty() {
        if path_exists(&managed_skill_path(paths)) {
            fs::remove_file(managed_skill_path(paths))?;
        }
        fs::remove_file(&manifest_file)?;
        let _ = fs::remove_dir(managed_dir(paths));
        let _ = fs::remove_dir(paths.home.join("skills"));
    } else {
        save_manifest(&manifest_file, &manifest)?;
    }
    Ok(removed)
}

fn sync_locked(paths: &Paths) -> Result<SyncReport> {
    let manifest_file = manifest_path(paths);
    let mut manifest = load_manifest(&manifest_file)?;
    validate_manifest(&manifest)?;
    let bundled_skill = render_skill(&manifest.skill_name)?;
    let bundled_sha256 = sha256(bundled_skill.as_bytes());
    let managed_dir = absolute_path(&managed_dir(paths))?;
    let mut report = SyncReport::default();

    if ensure_managed_unmodified(paths, &manifest).is_err() {
        report.managed_modified = Some(managed_skill_path(paths));
        return Ok(report);
    }
    let version_changed = manifest.bundled_sha256 != bundled_sha256;
    write_if_changed(&managed_skill_path(paths), bundled_skill.as_bytes())?;

    for record in &mut manifest.targets {
        sync_target(
            record,
            &managed_dir,
            bundled_skill.as_bytes(),
            &bundled_sha256,
            version_changed,
            &mut report,
        )?;
    }
    manifest.cli_version = env!("CARGO_PKG_VERSION").into();
    manifest.bundled_sha256 = bundled_sha256;
    save_manifest(&manifest_file, &manifest)?;
    Ok(report)
}

fn ensure_managed_unmodified(paths: &Paths, manifest: &InstallManifest) -> Result<()> {
    let skill_file = managed_skill_path(paths);
    if managed_file_modified(paths, manifest)? {
        bail!(
            "{} {}",
            skill_file.display(),
            text(
                "was modified; asvc will not overwrite or remove it. Back it up and move it first",
                "已被修改，asvc 不会覆盖或删除；请先备份并移走该文件"
            )
        );
    }
    Ok(())
}

fn managed_file_modified(paths: &Paths, manifest: &InstallManifest) -> Result<bool> {
    let skill_file = managed_skill_path(paths);
    if !path_exists(&skill_file) {
        return Ok(false);
    }
    let current_sha256 = sha256(&fs::read(&skill_file)?);
    let bundled_sha256 = sha256(render_skill(&manifest.skill_name)?.as_bytes());
    Ok(current_sha256 != manifest.bundled_sha256 && current_sha256 != bundled_sha256)
}

#[cfg(unix)]
fn preflight_install_target(
    record: Option<&TargetRecord>,
    install_dir: &Path,
    managed_dir: &Path,
    _bundled_sha256: &str,
    _overwrite_modified: bool,
) -> Result<()> {
    match record {
        Some(_) => validate_managed_link(install_dir, managed_dir, true),
        None if path_exists(install_dir) => bail!(
            "{} {}",
            install_dir.display(),
            text(
                "already exists and is not managed by asvc; move or remove it before retrying",
                "已存在且不归 asvc 托管；请先移动或删除后重试"
            )
        ),
        None => Ok(()),
    }
}

#[cfg(windows)]
fn preflight_install_target(
    record: Option<&TargetRecord>,
    install_dir: &Path,
    _managed_dir: &Path,
    bundled_sha256: &str,
    overwrite_modified: bool,
) -> Result<()> {
    match record {
        Some(record) if windows_copy_modified(record, bundled_sha256)? && !overwrite_modified => {
            Err(ModifiedSkill {
                path: record.path.join(SKILL_FILE),
            }
            .into())
        }
        Some(_) => Ok(()),
        None if path_exists(install_dir) => bail!(
            "{} {}",
            install_dir.display(),
            text(
                "already exists and is not managed by asvc; move or remove it before retrying",
                "已存在且不归 asvc 托管；请先移动或删除后重试"
            )
        ),
        None => Ok(()),
    }
}

#[cfg(unix)]
fn install_target(
    install_dir: &Path,
    managed_dir: &Path,
    _bundled_skill: &[u8],
    _bundled_sha256: &str,
) -> Result<()> {
    if path_exists(install_dir) {
        return validate_managed_link(install_dir, managed_dir, false);
    }
    let parent = install_dir.parent().ok_or_else(|| {
        anyhow!(
            "{} {}",
            install_dir.display(),
            text("has no parent directory", "没有父目录")
        )
    })?;
    fs::create_dir_all(parent)?;
    std::os::unix::fs::symlink(managed_dir, install_dir).with_context(|| {
        format!(
            "{} {} → {}",
            text("failed to create skill symlink", "无法创建 skill 软连接"),
            install_dir.display(),
            managed_dir.display()
        )
    })
}

#[cfg(windows)]
fn install_target(
    install_dir: &Path,
    _managed_dir: &Path,
    bundled_skill: &[u8],
    _bundled_sha256: &str,
) -> Result<()> {
    write_if_changed(&install_dir.join(SKILL_FILE), bundled_skill)?;
    Ok(())
}

#[cfg(unix)]
fn preflight_uninstall_target(record: &TargetRecord, managed_dir: &Path) -> Result<()> {
    validate_managed_link(&record.path, managed_dir, true)
}

#[cfg(windows)]
fn preflight_uninstall_target(record: &TargetRecord, _managed_dir: &Path) -> Result<()> {
    let _ = record;
    Ok(())
}

#[cfg(unix)]
fn remove_target(path: &Path) -> Result<()> {
    if path_exists(path) {
        fs::remove_file(path).with_context(|| {
            format!(
                "{} {}",
                text("failed to remove symlink", "无法删除软连接"),
                path.display()
            )
        })?;
    }
    Ok(())
}

#[cfg(windows)]
fn remove_target(path: &Path) -> Result<()> {
    let skill_file = path.join(SKILL_FILE);
    if path_exists(&skill_file) {
        fs::remove_file(&skill_file).with_context(|| {
            format!(
                "{} {}",
                text("failed to remove", "无法删除"),
                skill_file.display()
            )
        })?;
    }
    // Preserve the directory if the user placed any other resources in it.
    let _ = fs::remove_dir(path);
    Ok(())
}

#[cfg(unix)]
fn sync_target(
    record: &mut TargetRecord,
    managed_dir: &Path,
    bundled_skill: &[u8],
    bundled_sha256: &str,
    version_changed: bool,
    report: &mut SyncReport,
) -> Result<()> {
    if !path_exists(&record.path) {
        install_target(&record.path, managed_dir, bundled_skill, bundled_sha256)?;
        report.updated.push(record.target);
    } else if validate_managed_link(&record.path, managed_dir, false).is_err() {
        report.modified.push((record.target, record.path.clone()));
        return Ok(());
    } else if version_changed {
        report.updated.push(record.target);
    }
    record.installed_sha256 = bundled_sha256.into();
    Ok(())
}

#[cfg(windows)]
fn sync_target(
    record: &mut TargetRecord,
    _managed_dir: &Path,
    bundled_skill: &[u8],
    bundled_sha256: &str,
    _version_changed: bool,
    report: &mut SyncReport,
) -> Result<()> {
    let skill_file = record.path.join(SKILL_FILE);
    if !path_exists(&skill_file) {
        write_if_changed(&skill_file, bundled_skill)?;
        record.installed_sha256 = bundled_sha256.into();
        report.updated.push(record.target);
        return Ok(());
    }
    let current_sha256 = sha256(&fs::read(&skill_file)?);
    if current_sha256 == bundled_sha256 {
        record.installed_sha256 = bundled_sha256.into();
    } else if current_sha256 == record.installed_sha256 {
        write_if_changed(&skill_file, bundled_skill)?;
        record.installed_sha256 = bundled_sha256.into();
        report.updated.push(record.target);
    } else {
        report.modified.push((record.target, skill_file));
    }
    Ok(())
}

#[cfg(unix)]
fn target_state(
    record: &TargetRecord,
    manifest: &InstallManifest,
    managed_dir: &Path,
    bundled_sha256: &str,
) -> &'static str {
    let Ok(()) = validate_managed_link(&record.path, managed_dir, false) else {
        return if path_exists(&record.path) {
            text("unmanaged", "非托管")
        } else {
            text("missing", "缺失")
        };
    };
    let Ok(contents) = fs::read(record.path.join(SKILL_FILE)) else {
        return text("missing", "缺失");
    };
    let current_sha256 = sha256(&contents);
    if current_sha256 == bundled_sha256 {
        text("current", "最新")
    } else if current_sha256 == manifest.bundled_sha256 {
        text("pending sync", "待同步")
    } else {
        text("modified", "已修改")
    }
}

#[cfg(windows)]
fn target_state(
    record: &TargetRecord,
    _manifest: &InstallManifest,
    _managed_dir: &Path,
    bundled_sha256: &str,
) -> &'static str {
    let skill_file = record.path.join(SKILL_FILE);
    let Ok(contents) = fs::read(skill_file) else {
        return text("missing", "缺失");
    };
    let current_sha256 = sha256(&contents);
    if current_sha256 == bundled_sha256 {
        text("current", "最新")
    } else if current_sha256 == record.installed_sha256 {
        text("pending sync", "待同步")
    } else {
        text("modified", "已修改")
    }
}

#[cfg(unix)]
fn validate_managed_link(path: &Path, managed_dir: &Path, missing_ok: bool) -> Result<()> {
    if !path_exists(path) {
        if missing_ok {
            return Ok(());
        }
        bail!("{} {}", path.display(), text("does not exist", "不存在"));
    }
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_symlink() {
        bail!(
            "{} {}",
            path.display(),
            text(
                "is no longer a symlink created by asvc",
                "不再是 asvc 创建的软连接"
            )
        );
    }
    let target = fs::read_link(path)?;
    let target = if target.is_absolute() {
        target
    } else {
        path.parent().unwrap_or_else(|| Path::new(".")).join(target)
    };
    if target != managed_dir {
        if locale() == Locale::English {
            bail!(
                "{} points to {}, not the asvc-managed directory {}",
                path.display(),
                target.display(),
                managed_dir.display()
            );
        } else {
            bail!(
                "{} 指向 {}，而不是 asvc 托管目录 {}",
                path.display(),
                target.display(),
                managed_dir.display()
            );
        }
    }
    Ok(())
}

#[cfg(windows)]
fn windows_copy_modified(record: &TargetRecord, bundled_sha256: &str) -> Result<bool> {
    let skill_file = record.path.join(SKILL_FILE);
    if !path_exists(&skill_file) {
        return Ok(false);
    }
    let current_sha256 = sha256(&fs::read(&skill_file)?);
    Ok(current_sha256 != record.installed_sha256 && current_sha256 != bundled_sha256)
}

fn validate_skill_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > 64 {
        bail!(
            "{}",
            text(
                "skill name must contain 1–64 characters",
                "skill 名称长度必须为 1–64 个字符"
            )
        );
    }
    if name.starts_with('-') || name.ends_with('-') {
        bail!(
            "{}",
            text(
                "skill name cannot start or end with a hyphen",
                "skill 名称不能以连字符开头或结尾"
            )
        );
    }
    if !name.bytes().all(|character| {
        character.is_ascii_lowercase() || character.is_ascii_digit() || character == b'-'
    }) {
        bail!(
            "{}",
            text(
                "skill name may contain only lowercase letters, digits, and hyphens",
                "skill 名称只能包含小写字母、数字和连字符"
            )
        );
    }
    Ok(())
}

fn render_skill(skill_name: &str) -> Result<String> {
    validate_skill_name(skill_name)?;
    let default_name = format!("name: {DEFAULT_SKILL_NAME}");
    if !BUNDLED_SKILL.contains(&default_name) {
        bail!(
            "{} {DEFAULT_SKILL_NAME}",
            text(
                "bundled skill is missing its default name",
                "内嵌 skill 缺少默认名称"
            )
        );
    }
    Ok(BUNDLED_SKILL.replacen(&default_name, &format!("name: {skill_name}"), 1))
}

fn validate_manifest(manifest: &InstallManifest) -> Result<()> {
    if manifest.schema_version != SCHEMA_VERSION {
        bail!(
            "{}: {}",
            text(
                "unsupported skill manifest version",
                "不支持的 skill 安装清单版本"
            ),
            manifest.schema_version
        );
    }
    validate_skill_name(&manifest.skill_name).with_context(|| {
        format!(
            "{}: {}",
            text("invalid skill manifest name", "skill 安装清单名称无效"),
            manifest.skill_name
        )
    })?;
    let mut targets = BTreeSet::new();
    for record in &manifest.targets {
        if !targets.insert(record.target) {
            bail!(
                "{} {}",
                text(
                    "skill manifest contains a duplicate target:",
                    "skill 安装清单包含重复的目标："
                ),
                record.target.display_name()
            );
        }
        let expected = record.target.install_dir(&manifest.skill_name)?;
        if record.path != expected {
            if locale() == Locale::English {
                bail!(
                    "{} skill should be managed at {}, but the manifest contains {}",
                    record.target.display_name(),
                    expected.display(),
                    record.path.display()
                );
            } else {
                bail!(
                    "{} skill 的托管路径应为 {}，清单中却是 {}",
                    record.target.display_name(),
                    expected.display(),
                    record.path.display()
                );
            }
        }
    }
    Ok(())
}

fn load_manifest(path: &Path) -> Result<InstallManifest> {
    serde_json::from_slice(
        &fs::read(path).with_context(|| {
            format!("{} {}", text("failed to read", "无法读取"), path.display())
        })?,
    )
    .with_context(|| format!("{} {}", text("failed to parse", "无法解析"), path.display()))
}

fn save_manifest(path: &Path, manifest: &InstallManifest) -> Result<()> {
    let mut contents = serde_json::to_vec_pretty(manifest)?;
    contents.push(b'\n');
    write_atomic(path, &contents)
}

fn unique_targets(targets: &[SkillTarget]) -> Vec<SkillTarget> {
    targets
        .iter()
        .copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn managed_dir(paths: &Paths) -> PathBuf {
    paths.home.join("skills").join(MANAGED_SKILL_ID)
}

fn managed_skill_path(paths: &Paths) -> PathBuf {
    managed_dir(paths).join(SKILL_FILE)
}

fn manifest_path(paths: &Paths) -> PathBuf {
    managed_dir(paths).join(MANIFEST_NAME)
}

fn lock_path(paths: &Paths) -> PathBuf {
    paths.home.join("skills").join(".install.lock")
}

fn with_lock<T>(paths: &Paths, action: impl FnOnce() -> Result<T>) -> Result<T> {
    let lock_path = lock_path(paths);
    let parent = lock_path.parent().ok_or_else(|| {
        anyhow!(
            "{}",
            text(
                "skill lock path has no parent directory",
                "skill lock 路径无父目录"
            )
        )
    })?;
    fs::create_dir_all(parent)?;
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)?;
    lock.lock_exclusive()?;
    let result = action();
    let _ = FileExt::unlock(&lock);
    result
}

fn write_if_changed(path: &Path, contents: &[u8]) -> Result<bool> {
    if fs::read(path).is_ok_and(|current| current == contents) {
        return Ok(false);
    }
    write_atomic(path, contents)?;
    Ok(true)
}

fn write_atomic(path: &Path, contents: &[u8]) -> Result<()> {
    let parent = path.parent().ok_or_else(|| {
        anyhow!(
            "{} {}",
            path.display(),
            text("has no parent directory", "没有父目录")
        )
    })?;
    fs::create_dir_all(parent)?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("skill");
    let temporary = parent.join(format!(
        ".{file_name}.tmp.{}.{}",
        std::process::id(),
        TEMP_ID.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(contents)?;
        file.sync_all()?;
        replace_file(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.with_context(|| {
        format!(
            "{} {}",
            text("failed to atomically write", "无法原子写入"),
            path.display()
        )
    })
}

#[cfg(unix)]
fn replace_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    fs::rename(source, destination)
}

#[cfg(windows)]
fn replace_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let moved = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn absolute_path(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

fn sha256(contents: &[u8]) -> String {
    format!("{:x}", Sha256::digest(contents))
}

fn path_exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}
