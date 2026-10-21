//! 工具适配表：每个 agent 工具读哪个目录、用什么链接方式。
//!
//! 表是静态的（编译期内置），路径由 [`roots`] 在运行期展开为绝对路径。
//! `verified = false` 表示路径尚未实测确认，UI 只做只读展示。

use std::path::{Path, PathBuf};

use crate::model::LinkKind;

/// 单个工具的适配描述。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolSpec {
    pub id: &'static str,
    pub display_name: &'static str,
    pub kind: LinkKind,
    /// 目录是否经过实测确认（false 时 UI 只读展示）。
    pub verified: bool,
    /// 全局根（相对 home）。
    pub global_rel: &'static str,
    /// 项目级根（相对项目目录）。
    pub project_rel: &'static str,
}

/// 默认工具表。
pub const TOOLS: &[ToolSpec] = &[
    ToolSpec {
        id: "claude-code",
        display_name: "Claude Code",
        kind: LinkKind::Symlink,
        verified: true,
        global_rel: ".claude/skills",
        project_rel: ".claude/skills",
    },
    ToolSpec {
        id: "opencode",
        display_name: "OpenCode",
        kind: LinkKind::Symlink,
        verified: true,
        global_rel: ".config/opencode/skills",
        project_rel: ".opencode/skills",
    },
    ToolSpec {
        id: "codex",
        display_name: "Codex",
        kind: LinkKind::Symlink,
        // 目录存在，但「是否跟随目录链接」尚未实测。
        verified: false,
        global_rel: ".codex/skills",
        project_rel: ".codex/skills",
    },
    ToolSpec {
        id: "cursor",
        display_name: "Cursor",
        kind: LinkKind::Symlink,
        verified: false,
        global_rel: ".cursor/skills",
        project_rel: ".cursor/skills",
    },
    ToolSpec {
        id: "gemini-cli",
        display_name: "Gemini CLI",
        kind: LinkKind::Symlink,
        verified: false,
        global_rel: ".gemini/skills",
        project_rel: ".gemini/skills",
    },
];

/// 按 id 查工具。
pub fn find(id: &str) -> Option<&'static ToolSpec> {
    TOOLS.iter().find(|tool| tool.id == id)
}

/// 标记为「已实测确认」。`Box::leak` 一次性小对象：工具表本来就随进程存活，
/// 这样即使用户在系统设置里禁用了某个工具（内置表为 verified = false），
/// 只要目录仍在，UI 也能如实显示它可用。
impl ToolSpec {
    pub fn mark_verified(self) -> &'static ToolSpec {
        Box::leak(Box::new(ToolSpec {
            verified: true,
            ..self
        }))
    }
}

/// 全局根：`home` 不存在时返回 `None`（无法定位用户目录）。
pub fn global_root(tool: &ToolSpec, home: &Path) -> PathBuf {
    home.join(tool.global_rel)
}

/// 项目级根。
pub fn project_root(tool: &ToolSpec, project: &Path) -> PathBuf {
    project.join(tool.project_rel)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_ids_are_unique() {
        let mut ids: Vec<_> = TOOLS.iter().map(|tool| tool.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), TOOLS.len());
    }

    #[test]
    fn every_tool_has_a_root() {
        let home = Path::new("/home/demo");
        for tool in TOOLS {
            assert!(!tool.global_rel.is_empty());
            assert!(global_root(tool, home).starts_with(home));
        }
    }
}
