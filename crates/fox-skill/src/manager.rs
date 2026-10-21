//! SkillManager：扫描真源、推导各工具状态、建链/断链、SKILL.md 读写。
//!
//! 设计要点：
//! - **文件系统即真相**：不落库、不缓存；每次 `scan()` 直接推导状态；
//! - **真源只读语义**：启停只建/删链接，绝不移动真源内容；
//! - **破坏性操作可回退**：删除走 trash 目录，`Drifted` 目录一律不覆盖、不删除；
//! - **单写入者**：写操作由调用方串行化（命令层用 `Mutex`），本层不做锁。

use std::collections::BTreeSet;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::SkillError;
use crate::frontmatter::{self, Parsed};
use crate::link;
use crate::model::{
    ForeignSkill, LinkState, Scope, SkillDoc, SkillFrontmatter, SkillIndex, SkillInput,
    SkillSummary, SkillWarning, TargetState, ToolInfo,
};
use crate::tools::{self, ToolSpec, TOOLS};

/// skill 定义文件名。
pub const SKILL_FILE: &str = "SKILL.md";

/// SKILL.md 大小上限（1 MiB）。
pub const MAX_SKILL_BYTES: u64 = 1024 * 1024;

/// 名称长度上限。
const MAX_NAME_LEN: usize = 64;

/// 一个作用域下的 Skill 管理器。
#[derive(Debug, Clone)]
pub struct SkillManager {
    scope: Scope,
    source_root: PathBuf,
    trash_root: PathBuf,
    targets: Vec<TargetEntry>,
}

#[derive(Debug, Clone)]
struct TargetEntry {
    spec: &'static ToolSpec,
    root: PathBuf,
}

impl SkillManager {
    /// 按作用域构建：全局用 `~/.agents/skills`，项目级用 `<项目>/.agents/skills`。
    pub fn for_scope(scope: Scope) -> Result<Self, SkillError> {
        let (source_root, trash_root, targets): (
            PathBuf,
            PathBuf,
            Vec<(&'static ToolSpec, PathBuf)>,
        ) = match &scope {
            Scope::Global => {
                let home = dirs::home_dir().ok_or_else(|| {
                    SkillError::Unsupported("无法定位用户主目录，无法管理全局 skill".into())
                })?;
                (
                    home.join(".agents/skills"),
                    home.join(".agents/skill-trash"),
                    TOOLS
                        .iter()
                        .map(|spec| (spec, tools::global_root(spec, &home)))
                        .collect(),
                )
            }
            Scope::Project(project) => (
                project.join(".agents/skills"),
                project.join(".agents/skill-trash"),
                TOOLS
                    .iter()
                    .map(|spec| (spec, tools::project_root(spec, project)))
                    .collect(),
            ),
        };

        Ok(Self::with_layout(scope, source_root, trash_root, targets))
    }

    /// 全局作用域快捷构造。
    pub fn global() -> Result<Self, SkillError> {
        Self::for_scope(Scope::Global)
    }

    /// 重新探测：链接失效（被删）的工具，如果它的根目录又重新出现了，就放宽 `verified`。
    ///
    /// 只放宽、不放严——探测失败时保留内置结论，避免把已验证的工具
    /// 因为一次临时读不到目录而从 UI 上「摘掉」。
    pub fn with_probe(mut self) -> Self {
        for target in &mut self.targets {
            if !target.root.is_dir() {
                continue;
            }
            let spec = target.spec;
            let confirmed = TOOLS
                .iter()
                .find(|candidate| candidate.id == spec.id)
                .map(|candidate| candidate.verified)
                .unwrap_or(false);
            if !confirmed {
                target.spec = spec.mark_verified();
            }
        }
        self
    }

    /// 显式布局（测试 / 自定义目录用）。
    pub fn with_layout(
        scope: Scope,
        source_root: PathBuf,
        trash_root: PathBuf,
        targets: Vec<(&'static ToolSpec, PathBuf)>,
    ) -> Self {
        SkillManager {
            scope,
            source_root,
            trash_root,
            targets: targets
                .into_iter()
                .map(|(spec, root)| TargetEntry { spec, root })
                .collect(),
        }
    }

    /// 当前作用域。
    pub fn scope(&self) -> &Scope {
        &self.scope
    }

    /// 真源根目录。
    pub fn source_root(&self) -> &Path {
        &self.source_root
    }

    /// 工具列表（含目录是否存在）。
    pub fn tools(&self) -> Vec<ToolInfo> {
        self.targets
            .iter()
            .map(|target| ToolInfo {
                id: target.spec.id,
                display_name: target.spec.display_name,
                kind: target.spec.kind,
                verified: target.spec.verified,
                root: target.root.clone(),
                present: target.root.exists(),
            })
            .collect()
    }

    /// 全量扫描。
    pub fn scan(&self) -> SkillIndex {
        let mut index = SkillIndex {
            source_root: self.source_root.clone(),
            tools: self.tools(),
            ..Default::default()
        };

        let entries = self.source_entries(&mut index.warnings);
        let known: BTreeSet<String> = entries.iter().map(|(name, _)| name.clone()).collect();
        index.skills = entries
            .into_iter()
            .map(|(name, dir)| self.summary(&name, &dir))
            .collect();
        index.skills.sort_by(|a, b| a.name.cmp(&b.name));
        index.foreign = self.foreign(&known);
        index
    }

    /// 读取单个 skill 详情。
    pub fn read(&self, name: &str) -> Result<SkillDoc, SkillError> {
        let name = validate_name(name)?;
        let dir = self.existing_dir(&name)?;
        let raw = match fs::read_to_string(dir.join(SKILL_FILE)) {
            Ok(raw) => raw,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Err(SkillError::NotFound(format!("{name} 缺少 {SKILL_FILE}")))
            }
            Err(err) => return Err(err.into()),
        };
        let parsed = frontmatter::parse_lenient(&raw, &name);
        Ok(SkillDoc {
            summary: self.summary(&name, &dir),
            frontmatter: parsed.frontmatter,
            body: parsed.body,
            raw,
        })
    }

    /// 新建 skill（只写真源，不自动启用）。
    pub fn create(&self, input: SkillInput) -> Result<SkillSummary, SkillError> {
        let name = validate_name(&input.name)?;
        fs::create_dir_all(&self.source_root)?;

        let dir = self.source_root.join(&name);
        if fs::symlink_metadata(&dir).is_ok() {
            return Err(SkillError::AlreadyExists(name));
        }

        let frontmatter = self.build_frontmatter(&name, &input)?;
        let content = frontmatter::render(&frontmatter, &input.body)?;
        ensure_fits(content.len() as u64)?;
        write_atomic(&dir, &content)?;

        Ok(self.summary(&name, &dir))
    }

    /// 保存 skill 内容。
    ///
    /// `input.raw_expected` 与磁盘不一致时返回 [`SkillError::Conflict`]（外部改动保护）。
    pub fn save(&self, name: &str, input: SkillInput) -> Result<SkillSummary, SkillError> {
        let name = validate_name(name)?;
        let dir = self.existing_dir(&name)?;

        if !input.name.trim().is_empty() && input.name.trim() != name {
            return Err(SkillError::InvalidName(format!(
                "保存时不允许改名（{name} → {}）",
                input.name.trim()
            )));
        }

        if let Some(expected) = &input.raw_expected {
            let current = fs::read_to_string(dir.join(SKILL_FILE)).unwrap_or_default();
            if &current != expected {
                return Err(SkillError::Conflict(
                    "文件已被外部修改，请重新加载后再保存".into(),
                ));
            }
        }

        let frontmatter = self.build_frontmatter(&name, &input)?;
        let content = frontmatter::render(&frontmatter, &input.body)?;
        ensure_fits(content.len() as u64)?;
        write_atomic(&dir, &content)?;

        Ok(self.summary(&name, &dir))
    }

    /// 删除 skill：断链 → 真实源目录移入 trash（不硬删）。
    ///
    /// 真源条目是符号链接时，链接本身只是入口，真正的内容在链接目标里：
    /// 因此移动的是**真实目录**，然后删除入口链接，避免只剩一个悬空链接。
    /// `Drifted` 目标里的实体目录属于用户自己的文件，一律不碰。
    pub fn delete(&self, name: &str) -> Result<(), SkillError> {
        let name = validate_name(name)?;
        let entry = self.existing_dir(&name)?;
        let is_link = link::is_link(&entry);
        let real = fs::canonicalize(&entry)?;
        let root = fs::canonicalize(&self.source_root).unwrap_or_else(|_| self.source_root.clone());
        // 真身是真源（或主目录）的上层路径时拒绝：否则会把整棵树搬进 trash。
        if root.starts_with(&real) {
            return Err(SkillError::Conflict(format!(
                "{} 是上层目录，拒绝删除",
                real.display()
            )));
        }

        for target in &self.targets {
            let target_entry = target.root.join(&name);
            if link::is_link(&target_entry) {
                link::remove(&target_entry)?;
            }
        }

        fs::create_dir_all(&self.trash_root)?;
        let dest = unique_trash_path(&self.trash_root, &name);
        let victim = if is_link { real } else { entry.clone() };
        move_to_trash(&victim, dest)?;

        if is_link {
            link::remove(&entry)?;
        }
        Ok(())
    }

    /// 启停某个工具下的 skill。
    pub fn set_enabled(
        &self,
        name: &str,
        tool_id: &str,
        enabled: bool,
    ) -> Result<SkillSummary, SkillError> {
        let name = validate_name(name)?;
        let dir = self.existing_dir(&name)?;
        let source = fs::canonicalize(&dir)?;
        let target = self
            .targets
            .iter()
            .find(|target| target.spec.id == tool_id)
            .ok_or_else(|| SkillError::NotFound(format!("未知工具：{tool_id}")))?;

        if target.spec.kind == crate::model::LinkKind::CopyOnly {
            return Err(SkillError::Unsupported(format!(
                "{} 不跟随目录链接，v1 暂不支持",
                target.spec.display_name
            )));
        }

        let entry = target.root.join(&name);
        let state = inspect_entry(&entry, &source);

        if enabled {
            match state.state {
                LinkState::Linked => {}
                LinkState::Missing | LinkState::Broken => {
                    fs::create_dir_all(&target.root)?;
                    link::create(&entry, &source, target.spec.kind)?;
                }
                LinkState::Drifted => {
                    return Err(SkillError::Conflict(format!(
                        "{} 已存在实体目录：{}（不会自动覆盖，请手动处理）",
                        target.spec.display_name,
                        entry.display()
                    )))
                }
            }
        } else {
            match state.state {
                LinkState::Linked | LinkState::Broken => link::remove(&entry)?,
                LinkState::Missing => {}
                LinkState::Drifted => {
                    return Err(SkillError::Conflict(format!(
                        "{} 下是实体目录，启停不会删除你的文件：{}",
                        target.spec.display_name,
                        entry.display()
                    )))
                }
            }
        }

        Ok(self.summary(&name, &dir))
    }

    // ---------- 内部实现 ----------

    fn build_frontmatter(
        &self,
        name: &str,
        input: &SkillInput,
    ) -> Result<SkillFrontmatter, SkillError> {
        let description = input.description.trim();
        if description.is_empty() {
            return Err(SkillError::Frontmatter(
                "description 不能为空（各工具靠它判断何时加载）".into(),
            ));
        }
        Ok(SkillFrontmatter {
            name: name.to_string(),
            description: description.to_string(),
            extra: input.extra.clone(),
        })
    }

    fn source_entries(&self, warnings: &mut Vec<SkillWarning>) -> Vec<(String, PathBuf)> {
        let reader = match fs::read_dir(&self.source_root) {
            Ok(reader) => reader,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                warnings.push(warning(
                    "sourceMissing",
                    format!("真源目录尚不存在：{}", self.source_root.display()),
                ));
                return Vec::new();
            }
            Err(err) => {
                warnings.push(warning(
                    "sourceUnreadable",
                    format!("真源目录无法读取：{err}"),
                ));
                return Vec::new();
            }
        };

        let mut entries = Vec::new();
        for entry in reader.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            let path = entry.path();
            // 跟随链接判断是否目录（真源里偶尔会有指向别处的链接）。
            let Ok(meta) = fs::metadata(&path) else {
                continue;
            };
            if !meta.is_dir() {
                continue;
            }
            entries.push((name, path));
        }
        entries
    }

    fn foreign(&self, known: &BTreeSet<String>) -> Vec<ForeignSkill> {
        let mut seen = BTreeSet::new();
        let mut out = Vec::new();
        for target in &self.targets {
            let Ok(reader) = fs::read_dir(&target.root) else {
                continue;
            };
            for entry in reader.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with('.') || known.contains(&name) {
                    continue;
                }
                if !entry.path().is_dir() {
                    continue;
                }
                let key = (name.clone(), target.spec.id);
                if seen.insert(key) {
                    out.push(ForeignSkill {
                        name,
                        tool_id: target.spec.id.to_string(),
                        root: target.root.clone(),
                    });
                }
            }
        }
        out.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.tool_id.cmp(&b.tool_id)));
        out
    }

    fn summary(&self, name: &str, dir: &Path) -> SkillSummary {
        let mut warnings = Vec::new();
        let skill_md = dir.join(SKILL_FILE);
        let mut updated_at = None;

        let frontmatter = match fs::read_to_string(&skill_md) {
            Ok(raw) => {
                updated_at = fs::metadata(&skill_md)
                    .and_then(|meta| meta.modified())
                    .ok()
                    .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                    .map(|duration| duration.as_millis() as u64);
                let Parsed {
                    frontmatter,
                    warning: warn_message,
                    ..
                } = frontmatter::parse_lenient(&raw, name);
                if let Some(message) = warn_message {
                    warnings.push(warning("frontmatter", message));
                }
                frontmatter
            }
            Err(err) => {
                let message = if err.kind() == std::io::ErrorKind::NotFound {
                    format!("缺少 {SKILL_FILE}")
                } else {
                    format!("{SKILL_FILE} 无法读取：{err}")
                };
                warnings.push(warning("missingSkillFile", message));
                SkillFrontmatter {
                    name: name.to_string(),
                    ..Default::default()
                }
            }
        };

        if !frontmatter.name.is_empty() && frontmatter.name != name {
            warnings.push(warning(
                "nameMismatch",
                format!(
                    "frontmatter.name（{}）与目录名（{name}）不一致，部分工具会忽略该 skill",
                    frontmatter.name
                ),
            ));
        }

        let source = fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
        let states = self
            .targets
            .iter()
            .map(|target| {
                let mut state = inspect_entry(&target.root.join(name), &source);
                state.tool_id = target.spec.id.to_string();
                state.tool_name = target.spec.display_name.to_string();
                state.root = target.root.clone();
                state
            })
            .collect();

        SkillSummary {
            name: name.to_string(),
            description: frontmatter.description,
            source_root: self.source_root.clone(),
            // 真实源目录：真源条目是链接时沿链接解析，UI「打开目录」直接落到内容所在处。
            dir: source,
            updated_at,
            extra_entries: list_extra_entries(dir),
            states,
            warnings,
        }
    }

    /// 校验名称并确认真源条目存在。
    ///
    /// 真源条目允许是指向别处的符号链接（例如
    /// `~/.agents/skills/<name>` -> `~/.agents/skill-hub/skills/<name>`）：
    /// 读写都会由内核跟随链接落到**真实 skill** 上，所以链接条目不做「越界」拒绝；
    /// 只有实体目录才校验 realpath 未逃逸，防止链接父目录把写操作带到真源之外。
    fn existing_dir(&self, name: &str) -> Result<PathBuf, SkillError> {
        let name = validate_name(name)?;
        let dir = self.source_root.join(&name);
        let meta = fs::symlink_metadata(&dir)
            .map_err(|_| SkillError::NotFound(format!("{name} 不在真源目录中")))?;
        let is_link = link::is_link(&dir);
        if !meta.is_dir() && !is_link {
            return Err(SkillError::NotFound(format!("{name} 不是目录")));
        }
        let real =
            fs::canonicalize(&dir).map_err(|_| SkillError::NotFound(format!("{name} 已失效")))?;
        if !real.is_dir() {
            return Err(SkillError::NotFound(format!("{name} 链接指向的不是目录")));
        }
        if !is_link {
            let root = fs::canonicalize(&self.source_root)
                .map_err(|_| SkillError::NotFound("真源目录不存在".into()))?;
            if !real.starts_with(&root) {
                return Err(SkillError::OutsideRoot(format!(
                    "{} 不在真源目录内",
                    real.display()
                )));
            }
        }
        Ok(dir)
    }
}

/// 校验 skill 名称：仅 ASCII 字母数字、`-`、`_`、`.`，不以 `.` 开头/结尾，不含 `..`。
pub fn validate_name(raw: &str) -> Result<String, SkillError> {
    let name = raw.trim();
    if name.is_empty() {
        return Err(SkillError::InvalidName("名称不能为空".into()));
    }
    if name.len() > MAX_NAME_LEN {
        return Err(SkillError::InvalidName(format!(
            "名称过长（上限 {MAX_NAME_LEN} 字节）：{name}"
        )));
    }
    if name.starts_with('.') || name.ends_with('.') {
        return Err(SkillError::InvalidName(format!(
            "名称不能以 . 开头或结尾：{name}"
        )));
    }
    if name.contains("..") {
        return Err(SkillError::InvalidName(format!("名称不能包含 ..：{name}")));
    }
    let valid = name
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'));
    if !valid {
        return Err(SkillError::InvalidName(format!(
            "名称只能包含字母、数字、-、_、.：{name}"
        )));
    }
    Ok(name.to_string())
}

fn ensure_fits(bytes: u64) -> Result<(), SkillError> {
    if bytes > MAX_SKILL_BYTES {
        return Err(SkillError::TooLarge(bytes, MAX_SKILL_BYTES));
    }
    Ok(())
}

fn warning(code: &'static str, message: impl Into<String>) -> SkillWarning {
    SkillWarning {
        code,
        message: message.into(),
    }
}

fn inspect_entry(entry: &Path, source: &Path) -> TargetState {
    let meta = match fs::symlink_metadata(entry) {
        Ok(meta) => meta,
        Err(_) => {
            return TargetState {
                tool_id: String::new(),
                tool_name: String::new(),
                root: PathBuf::new(),
                state: LinkState::Missing,
                detail: None,
            }
        }
    };

    let (state, detail) = if link::is_link(entry) {
        match fs::canonicalize(entry) {
            Ok(real) if real == source => (LinkState::Linked, None),
            Ok(real) => (
                LinkState::Drifted,
                Some(format!("链接指向其他位置：{}", real.display())),
            ),
            Err(_) => (
                LinkState::Broken,
                Some("链接已悬空（真源被删除或移动）".to_string()),
            ),
        }
    } else if meta.is_dir() {
        (
            LinkState::Drifted,
            Some("目标位置是实体目录（已分叉，不会自动覆盖）".to_string()),
        )
    } else {
        (LinkState::Drifted, Some("目标位置是文件".to_string()))
    };

    TargetState {
        tool_id: String::new(),
        tool_name: String::new(),
        root: PathBuf::new(),
        state,
        detail,
    }
}

fn list_extra_entries(dir: &Path) -> Vec<String> {
    let Ok(reader) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut entries: Vec<String> = reader
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .filter(|name| name != SKILL_FILE && !name.starts_with('.'))
        .collect();
    entries.sort();
    entries
}

fn write_atomic(dir: &Path, content: &str) -> Result<(), SkillError> {
    fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(".{SKILL_FILE}.tmp-{}", std::process::id()));
    let result = (|| -> Result<(), SkillError> {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(content.as_bytes())?;
        file.sync_all()?;
        fs::rename(&tmp, dir.join(SKILL_FILE))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

fn unique_trash_path(trash_root: &Path, name: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    let mut candidate = trash_root.join(format!("{name}-{stamp}"));
    let mut suffix = 1;
    while candidate.exists() {
        candidate = trash_root.join(format!("{name}-{stamp}-{suffix}"));
        suffix += 1;
    }
    candidate
}

/// 移入 trash：同盘直接 rename，跨盘退化为拷贝后删除。
fn move_to_trash(source: &Path, dest: PathBuf) -> Result<(), SkillError> {
    match fs::rename(source, &dest) {
        Ok(()) => Ok(()),
        Err(_) => {
            copy_dir_all(source, &dest)?;
            fs::remove_dir_all(source)?;
            Ok(())
        }
    }
}

fn copy_dir_all(from: &Path, to: &Path) -> Result<(), SkillError> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let path = entry.path();
        let dest = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_all(&path, &dest)?;
        } else {
            fs::copy(&path, &dest)?;
        }
    }
    Ok(())
}
