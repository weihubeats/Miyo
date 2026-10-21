//! 对外数据模型（serde camelCase，前端直接消费，无需二次转换）。

use std::collections::BTreeMap;
use std::path::PathBuf;

/// 工具目录的落盘方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LinkKind {
    /// Unix 符号链接 / Windows 目录符号链接（需开发者模式）。
    Symlink,
    /// Windows NTFS junction（不需要管理员权限）。
    Junction,
    /// 只能拷贝（工具不跟随链接）；v1 未实现，`set_enabled` 返回 `UNSUPPORTED`。
    CopyOnly,
}

/// 单个 (skill, tool) 的生效状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LinkState {
    /// 链接存在且指向真源。
    Linked,
    /// 目标目录里没有该条目（未启用）。
    Missing,
    /// 目标里是实体目录或指向别处（已分叉）。
    Drifted,
    /// 链接悬空（真源被删 / 移动）。
    Broken,
}

impl LinkState {
    /// 是否算「已启用」。
    pub fn enabled(self) -> bool {
        matches!(self, LinkState::Linked)
    }
}

/// 某个 skill 在某个工具下的状态。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetState {
    pub tool_id: String,
    pub tool_name: String,
    /// 该工具的 skill 根目录。
    pub root: PathBuf,
    pub state: LinkState,
    /// 补充说明（指向哪里 / 为什么冲突）。
    pub detail: Option<String>,
}

/// 非阻断告警（名称不一致、缺 description 等）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillWarning {
    pub code: &'static str,
    pub message: String,
}

/// 列表项：一个真源 skill + 它在所有工具下的状态。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillSummary {
    pub name: String,
    pub description: String,
    pub source_root: PathBuf,
    /// 真实源目录：真源条目是链接时已沿链接解析到内容所在处。
    pub dir: PathBuf,
    /// SKILL.md 修改时间（Unix 毫秒）。
    pub updated_at: Option<u64>,
    /// 真源目录内除 SKILL.md 外的条目（scripts/、reference/ …）。
    pub extra_entries: Vec<String>,
    pub states: Vec<TargetState>,
    pub warnings: Vec<SkillWarning>,
}

/// frontmatter 的强类型视图；`extra` 原样保留未知键。
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillFrontmatter {
    pub name: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, serde_json::Value>,
}

/// 详情：frontmatter + 正文 + 原文（乐观锁用）。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillDoc {
    pub summary: SkillSummary,
    pub frontmatter: SkillFrontmatter,
    /// 正文（不含 frontmatter 与分隔符）。
    pub body: String,
    /// 文件原文，保存时作为 `raw_expected` 回传。
    pub raw: String,
}

/// 新建 / 保存入参。
#[derive(Debug, Clone, Default, PartialEq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillInput {
    pub name: String,
    pub description: String,
    pub body: String,
    #[serde(default)]
    pub extra: BTreeMap<String, serde_json::Value>,
    /// 编辑态传入原文；与磁盘不一致则视为外部改动 → `CONFLICT`。
    #[serde(default)]
    pub raw_expected: Option<String>,
}

/// 目标目录里有、真源里没有的 skill。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForeignSkill {
    pub name: String,
    pub tool_id: String,
    pub root: PathBuf,
}

/// 工具描述（含目录是否已存在）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolInfo {
    pub id: &'static str,
    pub display_name: &'static str,
    pub kind: LinkKind,
    /// 路径是否经过实测确认；false 时 UI 只读展示。
    pub verified: bool,
    pub root: PathBuf,
    /// 根目录或父目录是否存在（判断工具是否装过）。
    pub present: bool,
}

/// 一次扫描的完整结果。
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillIndex {
    pub source_root: PathBuf,
    pub skills: Vec<SkillSummary>,
    pub foreign: Vec<ForeignSkill>,
    pub tools: Vec<ToolInfo>,
    /// 扫描级告警（真源根不存在、条目是链接等）。
    pub warnings: Vec<SkillWarning>,
}

/// 作用域：全局（用户目录）或项目级（仓库内 `.agents/skills`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    Global,
    Project(PathBuf),
}

impl Scope {
    /// 人类可读标签（UI / 日志用）。
    pub fn label(&self) -> String {
        match self {
            Scope::Global => "global".to_string(),
            Scope::Project(p) => format!("project:{}", p.display()),
        }
    }
}
