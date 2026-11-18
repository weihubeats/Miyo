//! 插件清单（manifest.json）：类型定义 + 严格校验（真源，前端展示层只做镜像）。
//!
//! 安全校则：
//! - `deny_unknown_fields`：未知字段一律拒绝（防止未来字段被当成绕过口子）；
//! - 权限声明必须逐条命中 [`crate::permission`] 目录，未知名直接拒绝——
//!   不存在「声明了未知权限就能拿到某种能力」的通道；
//! - id / entry / 路由等所有路径形字段做穿越与字符白名单校验；
//! - 字符串一律长度封顶（清单本身不允许超过 64 KiB，命令层另有检查）。

use serde::{Deserialize, Serialize};

use miyo_core::AppError;

/// 支持的插件 SDK 协议版本（破坏性变更才递增）。
pub const SUPPORTED_SDK_VERSION: u32 = 1;

/// 清单总大小上限（命令层读文件后先行检查，校验层兜底）。
pub const MAX_MANIFEST_BYTES: usize = 64 * 1024;

/// 插件形态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum PluginKind {
    /// 纯前端页面插件。
    Ui,
    /// 页面 + 独立进程（sidecar 协议预留，v1 未启用；校验层直接拒绝）。
    UiSidecar,
}

/// 导航项声明（activate 时经 SDK 落地）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PluginNavItem {
    /// 导航项 ID（插件内唯一）。
    pub id: String,
    /// 文案 i18n 键（内置插件用；展示优先于 label）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_key: Option<String>,
    /// 文案字面量（第三方插件）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// 图标：主程序 IconName 或 data:image/* URI。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// 目标路由（以 / 开头）。
    pub route: String,
    /// 排序权重（核心项 0-99，插件默认 100 起）。
    #[serde(default)]
    pub order: i32,
}

/// 运行时依赖声明（v1 仅声明 + UI 提示；连通性由插件自身连接时探测）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PluginRuntimeDep {
    /// 依赖种类：java / node / python / binary …
    pub kind: String,
    /// 最低版本。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_version: Option<String>,
    /// 用户可读说明。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// 配置字段（JSON Schema 严格子集；插件管理中心按此渲染设置表单）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PluginConfigField {
    /// 字段类型。
    #[serde(rename = "type")]
    pub kind: PluginConfigFieldType,
    /// 表单标题。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// 表单说明。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// 默认值。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<serde_json::Value>,
    /// kind = enum 时的候选值。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enum_values: Option<Vec<String>>,
    /// number 范围（可选）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
}

/// 配置字段类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum PluginConfigFieldType {
    String,
    Number,
    Boolean,
    Enum,
}

/// 配置 Schema（JSON Schema 严格子集）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PluginConfigSchema {
    /// 属性表（≤ 32 键）。
    pub properties: std::collections::BTreeMap<String, PluginConfigField>,
    /// 必填键。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required: Option<Vec<String>>,
}

/// 文件完整性清单条目（manifest 2.0：覆盖包内全部文件的 sha256）。
///
/// 签名 payload 含 files 行清单（见 [`crate::package::signing_payload`]）——
/// manifest 内容与全部文件摘要都在签名覆盖范围内；安装 / 启用前逐文件复核。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PluginFileEntry {
    /// 包内相对路径（校验同 entry：禁绝对路径 / 穿越 / 反斜杠）。
    pub path: String,
    /// 文件内容 sha256（64 位 hex 小写）。
    pub sha256: String,
    /// 文件字节数。
    pub size: u64,
}

/// Sidecar 配置（manifest 2.0；kind = ui-sidecar 时必填）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PluginSidecarConfig {
    /// 可执行文件（包内相对路径；必须在 files 清单内；unix 安装后置 +x）。
    pub executable: String,
    /// sidecar 协议版本（宿主侧按协议号协商，破坏性变更递增）。
    pub protocol: u32,
    /// 内存上限 MB（进程级；None = 宿主默认上限）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_memory_mb: Option<u32>,
    /// 崩溃重启策略：off（默认）/ on-crash（指数退避，宿主内置防循环上限）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restart_policy: Option<String>,
}

/// 插件清单（manifest.json）。字段集与前端 `PluginManifest` 镜像（ts-rs 生成）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(deny_unknown_fields)]
pub struct PluginManifest {
    /// 全局唯一 ID：reverse-DNS。
    pub id: String,
    /// 展示名（第三方字面量 / 内置插件建议配 name_key 走 i18n）。
    pub name: String,
    /// 名称 i18n 键（内置插件）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name_key: Option<String>,
    /// 描述。
    pub description: String,
    /// 描述 i18n 键（内置插件）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description_key: Option<String>,
    /// 插件语义版本（x.y.z）。
    pub version: String,
    /// 作者。
    #[serde(default)]
    pub authors: Vec<String>,
    /// 图标：IconName 或 data:image/* URI。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// 入口（第三方 UI 插件 = 包内相对路径；内置插件省略）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry: Option<String>,
    /// 插件形态。
    pub kind: PluginKind,
    /// 最低兼容的 Miyo 版本。
    pub min_app_version: String,
    /// 插件 SDK 协议版本。
    pub sdk_version: u32,
    /// 导航注册声明。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nav: Option<Vec<PluginNavItem>>,
    /// 权限声明（必须逐条命中目录，未知名拒绝）。
    #[serde(default)]
    pub permissions: Vec<String>,
    /// 运行时依赖声明。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_deps: Option<Vec<PluginRuntimeDep>>,
    /// 插件间依赖（id → 最低版本）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_deps: Option<std::collections::BTreeMap<String, String>>,
    /// 配置 Schema。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_schema: Option<PluginConfigSchema>,
    /// 完整性摘要（sha256 hex of manifest bytes；安装时由后端计算）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub integrity: Option<String>,
    /// ed25519 签名（hex；覆盖 manifest 原文 + files 行清单，见 package.rs）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    /// 文件完整性清单（manifest 2.0 可选；声明即强制——安装 / 启用 / 检视
    /// 逐文件验 sha256，包内出现清单外文件即拒绝）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub files: Option<Vec<PluginFileEntry>>,
    /// Sidecar 配置（kind = ui-sidecar 时必填；ui 插件声明即拒绝）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sidecar: Option<PluginSidecarConfig>,
    /// 支持平台（manifest 2.0 可选；空 = 全平台。值白名单：
    /// windows / macos / linux，可带 -arm64 / -x64 后缀）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platforms: Option<Vec<String>>,
}

impl PluginManifest {
    /// 从 JSON 字节解析并严格校验。
    pub fn parse(bytes: &[u8]) -> Result<Self, AppError> {
        if bytes.len() > MAX_MANIFEST_BYTES {
            return Err(AppError::Validation(format!(
                "manifest.json 超过大小上限（{} > {} 字节）",
                bytes.len(),
                MAX_MANIFEST_BYTES
            )));
        }
        let manifest: PluginManifest = serde_json::from_slice(bytes)
            .map_err(|e| AppError::Validation(format!("manifest.json 解析失败：{e}")))?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// 全量校验（解析后仍可单独调用，用于更新场景的再校验）。
    pub fn validate(&self) -> Result<(), AppError> {
        Self::validate_id(&self.id)?;
        validate_bounded(&self.name, 1, 96, "name")?;
        validate_bounded(&self.description, 1, 512, "description")?;
        for (i, a) in self.authors.iter().enumerate() {
            validate_bounded(a, 1, 64, &format!("authors[{i}]"))?;
        }
        if self.authors.len() > 10 {
            return Err(AppError::Validation("authors 数量超过 10".into()));
        }
        if let Some(icon) = &self.icon {
            validate_icon(icon)?;
        }
        // 版本与兼容性：两处都必须是严格 x.y.z。
        validate_version(&self.version, "version")?;
        validate_version(&self.min_app_version, "min_app_version")?;
        // SDK 版本：高于宿主支持版本即拒绝（避免新插件跑在旧宿主上）。
        if self.sdk_version != SUPPORTED_SDK_VERSION {
            return Err(AppError::Validation(format!(
                "不支持的插件 SDK 版本：{}（当前支持 {SUPPORTED_SDK_VERSION}）",
                self.sdk_version
            )));
        }
        // kind：sidecar 形态 v1 预留未启用，明确拒绝而不是静默降级。
        if self.kind == PluginKind::UiSidecar {
            return Err(AppError::Validation(
                "ui-sidecar 插件形态尚未开放（协议预留）".into(),
            ));
        }
        if let Some(entry) = &self.entry {
            validate_entry(entry)?;
        }
        if let Some(nav) = &self.nav {
            if nav.is_empty() {
                return Err(AppError::Validation(
                    "nav 不能为空数组（省略字段即可）".into(),
                ));
            }
            if nav.len() > 8 {
                return Err(AppError::Validation("nav 项数超过 8".into()));
            }
            let mut seen_routes = std::collections::BTreeSet::new();
            let mut seen_ids = std::collections::BTreeSet::new();
            for item in nav {
                validate_bounded(&item.id, 1, 48, "nav.id")?;
                if !item
                    .id
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
                {
                    return Err(AppError::Validation(format!(
                        "nav.id 只允许小写字母 / 数字 / 连字符：{}",
                        item.id
                    )));
                }
                if !seen_ids.insert(item.id.clone()) {
                    return Err(AppError::Validation(format!("nav.id 重复：{}", item.id)));
                }
                validate_route(&item.route)?;
                if !seen_routes.insert(item.route.clone()) {
                    return Err(AppError::Validation(format!(
                        "nav.route 重复：{}",
                        item.route
                    )));
                }
                if item.label_key.is_none() && item.label.is_none() {
                    return Err(AppError::Validation(format!(
                        "nav[{}] 必须提供 label_key 或 label",
                        item.id
                    )));
                }
                if let Some(k) = &item.label_key {
                    validate_bounded(k, 1, 96, "nav.label_key")?;
                }
                if let Some(l) = &item.label {
                    validate_bounded(l, 1, 64, "nav.label")?;
                }
                if let Some(icon) = &item.icon {
                    validate_icon(icon)?;
                }
            }
        }
        // 权限：逐条命中目录，未知名拒绝（防声明绕过）。
        crate::permission::validate_permissions(&self.permissions)?;
        if let Some(deps) = &self.runtime_deps {
            if deps.len() > 8 {
                return Err(AppError::Validation("runtime_deps 数量超过 8".into()));
            }
            for dep in deps {
                validate_bounded(&dep.kind, 1, 32, "runtime_deps.kind")?;
                if let Some(v) = &dep.min_version {
                    // 外部运行时版本（如 Java "11"）不是插件 semver，做宽松字符校验。
                    validate_bounded(v, 1, 32, "runtime_deps.min_version")?;
                    if !v
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'))
                    {
                        return Err(AppError::Validation(format!(
                            "runtime_deps.min_version 含非法字符：{v}"
                        )));
                    }
                }
                if let Some(d) = &dep.description {
                    validate_bounded(d, 0, 256, "runtime_deps.description")?;
                }
            }
        }
        if let Some(deps) = &self.plugin_deps {
            if deps.len() > 8 {
                return Err(AppError::Validation("plugin_deps 数量超过 8".into()));
            }
            for (id, ver) in deps {
                Self::validate_id(id)?;
                validate_version(ver, "plugin_deps 版本")?;
                if id == &self.id {
                    return Err(AppError::Validation("plugin_deps 不能依赖自身".into()));
                }
            }
        }
        if let Some(schema) = &self.config_schema {
            validate_config_schema(schema)?;
        }
        if let Some(h) = &self.integrity {
            validate_sha256_hex(h)?;
        }
        if let Some(s) = &self.signature {
            if s.len() != 128 || !s.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(AppError::Validation(
                    "signature 必须是 128 位 hex（ed25519 签名 64 字节）".into(),
                ));
            }
        }
        // manifest 2.0：文件完整性清单（声明即强制；entry 必须在清单内）。
        if let Some(files) = &self.files {
            if files.is_empty() {
                return Err(AppError::Validation(
                    "files 不能为空数组（省略字段即可）".into(),
                ));
            }
            if files.len() > 200 {
                return Err(AppError::Validation("files 数量超过 200".into()));
            }
            let mut seen = std::collections::BTreeSet::new();
            for f in files {
                validate_package_path(&f.path, "files.path")?;
                validate_sha256_hex(&f.sha256)?;
                if f.sha256.chars().any(|c| c.is_ascii_uppercase()) {
                    return Err(AppError::Validation(format!(
                        "files.sha256 必须小写 hex：{}",
                        f.path
                    )));
                }
                if f.size == 0 {
                    return Err(AppError::Validation(format!(
                        "files.size 必须大于 0：{}",
                        f.path
                    )));
                }
                if !seen.insert(f.path.clone()) {
                    return Err(AppError::Validation(format!("files.path 重复：{}", f.path)));
                }
            }
            if let Some(entry) = &self.entry {
                if !seen.contains(entry) {
                    return Err(AppError::Validation(format!(
                        "entry（{entry}）必须包含在 files 清单内"
                    )));
                }
            }
        }
        // manifest 2.0：sidecar 配置（ui 形态声明即拒绝；ui-sidecar 在开放时
        // 要求 sidecar 必填——见 kind 分支）。
        if let Some(sidecar) = &self.sidecar {
            if self.kind != PluginKind::UiSidecar {
                return Err(AppError::Validation(
                    "sidecar 配置仅支持 ui-sidecar 插件形态".into(),
                ));
            }
            validate_package_path(&sidecar.executable, "sidecar.executable")?;
            if sidecar.protocol == 0 {
                return Err(AppError::Validation("sidecar.protocol 必须 ≥ 1".into()));
            }
            if let Some(mem) = sidecar.max_memory_mb {
                if mem == 0 || mem > 4096 {
                    return Err(AppError::Validation(
                        "sidecar.max_memory_mb 必须在 1-4096".into(),
                    ));
                }
            }
            if let Some(p) = &sidecar.restart_policy {
                if p != "off" && p != "on-crash" {
                    return Err(AppError::Validation(
                        "sidecar.restart_policy 只允许 off / on-crash".into(),
                    ));
                }
            }
            if let Some(files) = &self.files {
                if !files.iter().any(|f| f.path == sidecar.executable) {
                    return Err(AppError::Validation(format!(
                        "sidecar.executable（{}）必须包含在 files 清单内",
                        sidecar.executable
                    )));
                }
            }
        }
        if let Some(platforms) = &self.platforms {
            if platforms.is_empty() {
                return Err(AppError::Validation(
                    "platforms 不能为空数组（省略字段即可）".into(),
                ));
            }
            if platforms.len() > 6 {
                return Err(AppError::Validation("platforms 数量超过 6".into()));
            }
            let mut seen = std::collections::BTreeSet::new();
            for p in platforms {
                let known = ["windows", "macos", "linux"];
                let base = p.split('-').next().unwrap_or(p);
                let suffix = p.strip_prefix(base).unwrap_or("");
                if !known.contains(&base)
                    || !(suffix.is_empty() || suffix == "-arm64" || suffix == "-x64")
                {
                    return Err(AppError::Validation(format!(
                        "platforms 含不支持的平台：{p}（允许 windows/macos/linux，\
                         可带 -arm64 / -x64 后缀）"
                    )));
                }
                if !seen.insert(p.as_str()) {
                    return Err(AppError::Validation(format!("platforms 重复：{p}")));
                }
            }
        }
        Ok(())
    }

    /// 插件 ID：reverse-DNS（1-4 段，每段小写字母开头，含 [a-z0-9-]）。
    pub fn validate_id(id: &str) -> Result<(), AppError> {
        validate_bounded(id, 3, 64, "id")?;
        let segments: Vec<&str> = id.split('.').collect();
        if segments.len() < 2 || segments.len() > 4 {
            return Err(AppError::Validation(format!(
                "插件 ID 必须是 2-4 段的 reverse-DNS：{id}"
            )));
        }
        for seg in segments {
            let mut chars = seg.chars();
            let first = chars.next().unwrap_or_default();
            if !first.is_ascii_lowercase() {
                return Err(AppError::Validation(format!(
                    "插件 ID 段必须以小写字母开头：{seg}"
                )));
            }
            if !chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
                return Err(AppError::Validation(format!(
                    "插件 ID 段只允许小写字母 / 数字 / 连字符：{seg}"
                )));
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// 校验工具
// ---------------------------------------------------------------------------

/// 字符串长度边界校验（字符数）。
fn validate_bounded(s: &str, min: usize, max: usize, field: &str) -> Result<(), AppError> {
    let n = s.chars().count();
    if n < min || n > max {
        return Err(AppError::Validation(format!(
            "{field} 长度必须在 {min}-{max} 之间（当前 {n}）"
        )));
    }
    Ok(())
}

/// 严格语义版本：`x.y.z`，每段 0-999 的无符号整数（拒绝 v 前缀 / 预发布 / 元数据）。
pub fn validate_version(v: &str, field: &str) -> Result<(), AppError> {
    parse_version(v)
        .map(|_| ())
        .map_err(|e| AppError::Validation(format!("{field} 不是合法版本号（x.y.z）：{v}（{e}）")))
}

/// 解析 `x.y.z` 为 (major, minor, patch)。失败返回原因。
pub fn parse_version(v: &str) -> Result<(u64, u64, u64), String> {
    let parts: Vec<&str> = v.split('.').collect();
    if parts.len() != 3 {
        return Err("必须有三段".into());
    }
    let mut out = [0u64; 3];
    for (i, p) in parts.iter().enumerate() {
        if p.is_empty() || p.len() > 3 || !p.chars().all(|c| c.is_ascii_digit()) {
            return Err(format!("第 {} 段非法：{p}", i + 1));
        }
        out[i] = p
            .parse()
            .map_err(|_| format!("第 {} 段超出范围：{p}", i + 1))?;
    }
    Ok((out[0], out[1], out[2]))
}

/// 图标：主程序 IconName（≤ 32，字母数字连字符）或 data:image/* base64 URI（≤ 16 KiB）。
fn validate_icon(icon: &str) -> Result<(), AppError> {
    if icon.starts_with("data:image/") {
        return validate_bounded(icon, 14, 16 * 1024, "icon(data-uri)");
    }
    validate_bounded(icon, 1, 32, "icon")?;
    if !icon
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(AppError::Validation(format!(
            "icon 名只允许小写字母 / 数字 / 连字符：{icon}"
        )));
    }
    Ok(())
}

/// 入口文件：包内相对路径；禁绝对路径 / `..` / 反斜杠 / 非常规扩展。
fn validate_entry(entry: &str) -> Result<(), AppError> {
    validate_bounded(entry, 1, 128, "entry")?;
    if entry.starts_with('/') || entry.starts_with('\\') {
        return Err(AppError::Validation("entry 禁止绝对路径".into()));
    }
    if entry
        .split(['/', '\\'])
        .any(|seg| seg == ".." || seg == ".")
    {
        return Err(AppError::Validation(
            "entry 禁止路径穿越段（.. / .）".into(),
        ));
    }
    if !entry.to_ascii_lowercase().ends_with(".html") {
        return Err(AppError::Validation(
            "entry 当前仅允许 .html（UI 插件）".into(),
        ));
    }
    Ok(())
}

/// 包内相对路径（files 清单 / sidecar 可执行文件通用）：
/// 禁绝对路径、反斜杠、`..` 段、空段；字符白名单
/// `[A-Za-z0-9._-]`（与解包条目白名单一致——声明里合法的路径必须能通过解包）。
pub fn validate_package_path(path: &str, field: &str) -> Result<(), AppError> {
    validate_bounded(path, 1, 200, field)?;
    if path.starts_with('/') || path.starts_with('\\') || path.contains('\\') {
        return Err(AppError::Validation(format!(
            "{field} 禁止绝对路径 / 反斜杠：{path}"
        )));
    }
    let mut segments = path.split('/');
    for seg in segments.by_ref() {
        if seg.is_empty() || seg == "." || seg == ".." {
            return Err(AppError::Validation(format!(
                "{field} 含非法路径段（空 / . / ..）：{path}"
            )));
        }
        if !seg
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        {
            return Err(AppError::Validation(format!(
                "{field} 路径段含非法字符（白名单 [A-Za-z0-9._-]）：{seg}"
            )));
        }
    }
    Ok(())
}

/// 路由：以 / 开头、无空白、无 `..`、长度 ≤ 128、ASCII 可见字符。
fn validate_route(route: &str) -> Result<(), AppError> {
    validate_bounded(route, 2, 128, "nav.route")?;
    if !route.starts_with('/') {
        return Err(AppError::Validation(format!(
            "nav.route 必须以 / 开头：{route}"
        )));
    }
    if route
        .chars()
        .any(|c| c.is_whitespace() || !c.is_ascii_graphic())
    {
        return Err(AppError::Validation(format!(
            "nav.route 含非法字符：{route}"
        )));
    }
    if route.split('/').any(|seg| seg == ".." || seg == ".") {
        return Err(AppError::Validation(format!(
            "nav.route 禁止穿越段：{route}"
        )));
    }
    Ok(())
}

/// sha256 hex（64 位十六进制）。
fn validate_sha256_hex(s: &str) -> Result<(), AppError> {
    if s.len() != 64 || !s.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(AppError::Validation(
            "integrity 必须是 64 位 hex（sha256）".into(),
        ));
    }
    Ok(())
}

/// 配置 Schema 校验：键白名单 + 数量上限 + 字段类型约束。
fn validate_config_schema(schema: &PluginConfigSchema) -> Result<(), AppError> {
    if schema.properties.len() > 32 {
        return Err(AppError::Validation("config_schema 属性数超过 32".into()));
    }
    for (key, field) in &schema.properties {
        validate_bounded(key, 1, 48, "config_schema key")?;
        if !key
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
        {
            return Err(AppError::Validation(format!(
                "config_schema 键只允许小写字母 / 数字 / 下划线 / 连字符：{key}"
            )));
        }
        if let Some(default) = &field.default {
            let ok = match field.kind {
                PluginConfigFieldType::String => default.is_string(),
                PluginConfigFieldType::Number => default.is_number(),
                PluginConfigFieldType::Boolean => default.is_boolean(),
                PluginConfigFieldType::Enum => default.is_string(),
            };
            if !ok {
                return Err(AppError::Validation(format!(
                    "config_schema[{key}] default 类型与字段类型不符"
                )));
            }
        }
        if let Some(values) = &field.enum_values {
            if field.kind != PluginConfigFieldType::Enum {
                return Err(AppError::Validation(format!(
                    "config_schema[{key}] enum_values 只允许 enum 类型字段"
                )));
            }
            if values.is_empty() || values.len() > 24 {
                return Err(AppError::Validation(format!(
                    "config_schema[{key}] enum 候选数必须在 1-24"
                )));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_manifest() -> String {
        r#"{
          "id": "com.example.demo",
          "name": "Demo",
          "description": "示例插件",
          "version": "0.1.0",
          "authors": ["tester"],
          "kind": "ui",
          "min_app_version": "0.0.60",
          "sdk_version": 1,
          "permissions": ["std.storage"]
        }"#
        .to_string()
    }

    #[test]
    fn accepts_minimal_manifest() {
        let m = PluginManifest::parse(base_manifest().as_bytes()).unwrap();
        assert_eq!(m.id, "com.example.demo");
        assert_eq!(m.permissions, vec!["std.storage".to_string()]);
        assert!(m.nav.is_none());
    }

    #[test]
    fn rejects_unknown_fields() {
        let bad = base_manifest().replace(
            r#""permissions": ["std.storage"]"#,
            r#""permissions": ["std.storage"], "evil_hook": true"#,
        );
        let err = PluginManifest::parse(bad.as_bytes()).unwrap_err();
        assert!(err.user_message().contains("解析失败"), "{err}");
    }

    #[test]
    fn rejects_unknown_permission_names() {
        let bad = base_manifest().replace("std.storage", "std.arbitrary.file");
        let err = PluginManifest::parse(bad.as_bytes()).unwrap_err();
        assert!(err.user_message().contains("未知权限"), "{err}");
    }

    #[test]
    fn rejects_bad_ids() {
        // 合法参照：com.demo（2 段）也应通过。
        let ok = base_manifest().replace("com.example.demo", "com.demo");
        assert!(PluginManifest::parse(ok.as_bytes()).is_ok());
        for id in [
            "demo",
            "Com.demo.x",
            "com.demo..x",
            "com.Demo.x",
            "com.demo.x.y.z.w",
        ] {
            let bad = base_manifest().replace("com.example.demo", id);
            assert!(
                PluginManifest::parse(bad.as_bytes()).is_err(),
                "应拒绝 id = {id}"
            );
        }
    }

    #[test]
    fn rejects_path_traversal_in_entry_and_route() {
        let bad = base_manifest().replace(
            r#""permissions": ["std.storage"]"#,
            r#""permissions": ["std.storage"], "entry": "../evil/index.html""#,
        );
        let err = PluginManifest::parse(bad.as_bytes()).unwrap_err();
        assert!(err.user_message().contains("穿越"), "{err}");

        let bad_route = base_manifest().replace(
            r#""permissions": ["std.storage"]"#,
            r#""permissions": ["std.storage"], "nav": [{"id": "m", "label": "x", "route": "/../x"}]"#,
        );
        let err = PluginManifest::parse(bad_route.as_bytes()).unwrap_err();
        assert!(err.user_message().contains("穿越"), "{err}");
    }

    #[test]
    fn rejects_bad_versions_and_sdk() {
        for v in ["1", "0.0", "0.0.x", "v0.0.1", "0.0.1000", "0.0.60-beta"] {
            let bad = base_manifest().replace("\"0.1.0\"", &format!("\"{v}\""));
            assert!(PluginManifest::parse(bad.as_bytes()).is_err(), "应拒绝 {v}");
        }
        let bad_sdk = base_manifest().replace("\"sdk_version\": 1", "\"sdk_version\": 2");
        let err = PluginManifest::parse(bad_sdk.as_bytes()).unwrap_err();
        assert!(err.user_message().contains("SDK 版本"), "{err}");
    }

    #[test]
    fn rejects_sidecar_kind_for_now() {
        let bad = base_manifest().replace("\"kind\": \"ui\"", "\"kind\": \"ui-sidecar\"");
        let err = PluginManifest::parse(bad.as_bytes()).unwrap_err();
        assert!(err.user_message().contains("ui-sidecar"), "{err}");
    }

    #[test]
    fn nav_requires_label_and_unique_routes() {
        let ok = base_manifest().replace(
            r#""permissions": ["std.storage"]"#,
            r#""permissions": ["std.storage"], "nav": [{"id": "main", "label": "面板", "route": "/demo"}]"#,
        );
        assert!(PluginManifest::parse(ok.as_bytes()).is_ok());

        let no_label = ok.replace(r#""label": "面板", "#, "");
        assert!(PluginManifest::parse(no_label.as_bytes()).is_err());

        let dup = ok.replace(
            r#""nav": [{"id": "main", "label": "面板", "route": "/demo"}]"#,
            r#""nav": [{"id": "a", "label": "x", "route": "/d"}, {"id": "b", "label": "y", "route": "/d"}]"#,
        );
        assert!(PluginManifest::parse(dup.as_bytes()).is_err());
    }

    #[test]
    fn version_parse_edge_cases() {
        assert_eq!(parse_version("0.0.60").unwrap(), (0, 0, 60));
        assert_eq!(parse_version("12.345.678").unwrap(), (12, 345, 678));
        assert!(parse_version("1.2").is_err());
        assert!(parse_version("1.2.3.4").is_err());
        assert!(parse_version("1.2.x").is_err());
        assert!(parse_version("").is_err());
    }

    // ---------------- manifest 2.0：files / sidecar / platforms ----------------

    fn sha_a() -> String {
        "aa".repeat(32)
    }
    fn sha_b() -> String {
        "bb".repeat(32)
    }

    fn manifest_with_files(files_json: &str, entry: &str) -> String {
        base_manifest().replace(
            r#""permissions": ["std.storage"]"#,
            &format!(
                r#""permissions": ["std.storage"], "entry": "{entry}", "files": {files_json}"#
            ),
        )
    }

    #[test]
    fn accepts_valid_files_list() {
        let (sha_a, sha_b) = (sha_a(), sha_b());
        let ok = manifest_with_files(
            &format!(
                r#"[{{"path": "index.html", "sha256": "{sha_a}", "size": 1024}},
                    {{"path": "assets/app.js", "sha256": "{sha_b}", "size": 8}}]"#
            ),
            "index.html",
        );
        let m = PluginManifest::parse(ok.as_bytes()).unwrap();
        assert_eq!(m.files.as_ref().unwrap().len(), 2);
    }

    #[test]
    fn files_rejects_traversal_uppercase_sha_dup_and_empty() {
        let sha = sha_a();
        // 穿越
        let bad = manifest_with_files(
            &format!(r#"[{{"path": "../evil.html", "sha256": "{sha}", "size": 1}}]"#),
            "index.html",
        );
        assert!(PluginManifest::parse(bad.as_bytes()).is_err());
        // 大写 hex
        let upper: String = sha.to_uppercase();
        let bad = manifest_with_files(
            &format!(r#"[{{"path": "index.html", "sha256": "{upper}", "size": 1}}]"#),
            "index.html",
        );
        assert!(PluginManifest::parse(bad.as_bytes()).is_err());
        // 重复路径
        let bad = manifest_with_files(
            &format!(
                r#"[{{"path": "index.html", "sha256": "{sha}", "size": 1}},
                    {{"path": "index.html", "sha256": "{sha}", "size": 2}}]"#
            ),
            "index.html",
        );
        assert!(PluginManifest::parse(bad.as_bytes()).is_err());
        // size = 0
        let bad = manifest_with_files(
            &format!(r#"[{{"path": "index.html", "sha256": "{sha}", "size": 0}}]"#),
            "index.html",
        );
        assert!(PluginManifest::parse(bad.as_bytes()).is_err());
        // 空数组
        let bad = manifest_with_files("[]", "index.html");
        assert!(PluginManifest::parse(bad.as_bytes()).is_err());
    }

    #[test]
    fn files_must_cover_entry() {
        let sha = sha_a();
        let bad = manifest_with_files(
            &format!(r#"[{{"path": "other.html", "sha256": "{sha}", "size": 1}}]"#),
            "index.html",
        );
        let err = PluginManifest::parse(bad.as_bytes()).unwrap_err();
        assert!(err.user_message().contains("files 清单"), "{err}");
    }

    #[test]
    fn sidecar_rejected_on_ui_kind() {
        let sha = sha_a();
        let bad = base_manifest().replace(
            r#""permissions": ["std.storage"]"#,
            &format!(
                r#""permissions": ["std.storage"],
                "sidecar": {{"executable": "bin/agent", "protocol": 1, "max_memory_mb": 256}},
                "files": [{{"path": "bin/agent", "sha256": "{sha}", "size": 1}}]"#
            ),
        );
        let err = PluginManifest::parse(bad.as_bytes()).is_err();
        assert!(err);
    }

    #[test]
    fn platforms_validation() {
        let ok = base_manifest().replace(
            r#""permissions": ["std.storage"]"#,
            r#""permissions": ["std.storage"], "platforms": ["macos-arm64", "linux"]"#,
        );
        assert!(PluginManifest::parse(ok.as_bytes()).is_ok());
        let bad = base_manifest().replace(
            r#""permissions": ["std.storage"]"#,
            r#""permissions": ["std.storage"], "platforms": ["android"]"#,
        );
        assert!(PluginManifest::parse(bad.as_bytes()).is_err());
        let dup = base_manifest().replace(
            r#""permissions": ["std.storage"]"#,
            r#""permissions": ["std.storage"], "platforms": ["linux", "linux"]"#,
        );
        assert!(PluginManifest::parse(dup.as_bytes()).is_err());
    }
}
