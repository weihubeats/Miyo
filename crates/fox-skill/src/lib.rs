//! # fox-skill：Agent Skill 管理核心库
//!
//! 管理「真源 + 多工具链接」模式的 skill 资产：一个 skill 只写一份内容
//! （`~/.agents/skills/<name>/SKILL.md`），各 agent 工具目录通过目录链接引用它。
//!
//! ## 模型
//!
//! | 概念 | 说明 |
//! | --- | --- |
//! | 真源 | [`Scope::Global`] → `~/.agents/skills`；[`Scope::Project`] → `<项目>/.agents/skills` |
//! | 目标 | Claude Code / OpenCode / Codex / Cursor / Gemini CLI 的 skill 目录 |
//! | 状态 | `Linked` / `Missing` / `Drifted` / `Broken`，见 [`LinkState`] |
//!
//! ## 设计约束
//!
//! - **文件系统即真相**：不落库，`scan()` 每次推导；
//! - **启停只动链接**：真源内容永不因启停移动；
//! - **不静默覆盖**：目标位置是实体目录（`Drifted`）时返回 `CONFLICT`，交人处理；
//! - **删除可回退**：真源移入 `skill-trash/`，不是硬删；
//! - **Windows 用 junction**：不需要开发者模式，`symlink_dir` 不作为默认路径。
//!
//! ## 用法
//!
//! ```rust,no_run
//! use fox_skill::{Scope, SkillInput, SkillManager};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let manager = SkillManager::for_scope(Scope::Global)?;
//!
//! // 新建（只写真源，不自动启用）
//! manager.create(SkillInput {
//!     name: "pr-review".into(),
//!     description: "按团队规范审查 PR".into(),
//!     body: "# PR Review\n\n…".into(),
//!     ..Default::default()
//! })?;
//!
//! // 在 Claude Code 下启用
//! manager.set_enabled("pr-review", "claude-code", true)?;
//!
//! // 扫描全部状态
//! let index = manager.scan();
//! assert!(index.skills.iter().any(|skill| skill.name == "pr-review"));
//! # Ok(())
//! # }
//! ```

pub mod error;
pub mod frontmatter;
pub mod link;
pub mod manager;
pub mod model;
pub mod tools;

pub use error::SkillError;
pub use manager::{validate_name, SkillManager, MAX_SKILL_BYTES, SKILL_FILE};
pub use model::{
    ForeignSkill, LinkKind, LinkState, Scope, SkillDoc, SkillFrontmatter, SkillIndex, SkillInput,
    SkillSummary, SkillWarning, TargetState, ToolInfo,
};
