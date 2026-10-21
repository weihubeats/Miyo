//! 错误类型：错误码与 `fox-tauri::CommandError` 的 `code` 一一对应。
//!
//! 本 crate 不依赖 tauri，命令层只需 `err.code()` → `CommandError::with_code`。

use thiserror::Error;

/// Skill 管理错误。
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SkillError {
    /// 名称不合法（含路径分隔符 / 大小写 / 长度越界）。
    #[error("skill 名称不合法：{0}")]
    InvalidName(String),

    /// 真源或目标条目不存在。
    #[error("skill 不存在：{0}")]
    NotFound(String),

    /// 真源已存在同名目录。
    #[error("skill 已存在：{0}")]
    AlreadyExists(String),

    /// 与现有文件系统状态冲突（外部分叉 / 并发修改）。
    #[error("状态冲突：{0}")]
    Conflict(String),

    /// frontmatter 缺失或字段不合法。
    #[error("frontmatter 无效：{0}")]
    Frontmatter(String),

    /// 内容超出大小上限。
    #[error("文件过大：{0} 字节（上限 {1} 字节）")]
    TooLarge(u64, u64),

    /// 建链 / 断链失败（权限、文件系统不支持）。
    #[error("链接操作失败：{0}")]
    LinkFailed(String),

    /// 当前版本不支持的操作（如 CopyOnly 工具）。
    #[error("暂不支持：{0}")]
    Unsupported(String),

    /// 路径逃逸（realpath 不在真源根内）。
    #[error("路径越界：{0}")]
    OutsideRoot(String),

    /// 底层 IO 错误。
    #[error("IO 错误：{0}")]
    Io(String),
}

impl SkillError {
    /// 程序化错误码，映射到前端 `{ code }`。
    pub fn code(&self) -> &'static str {
        match self {
            SkillError::InvalidName(_)
            | SkillError::Frontmatter(_)
            | SkillError::TooLarge(_, _) => "VALIDATION",
            SkillError::NotFound(_) => "NOT_FOUND",
            SkillError::AlreadyExists(_) | SkillError::Conflict(_) => "CONFLICT",
            SkillError::LinkFailed(_) => "LINK_FAILED",
            SkillError::Unsupported(_) => "UNSUPPORTED",
            SkillError::OutsideRoot(_) => "FORBIDDEN",
            SkillError::Io(_) => "IO",
        }
    }
}

impl From<std::io::Error> for SkillError {
    fn from(err: std::io::Error) -> Self {
        SkillError::Io(err.to_string())
    }
}
