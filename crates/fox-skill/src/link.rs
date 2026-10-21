//! 目录链接的跨平台封装。
//!
//! - Unix：`std::os::unix::fs::symlink`；
//! - Windows：一律 junction（`junction` crate，不需要开发者模式/管理员权限）；
//!   `symlink_dir` 需要开发者模式，不作为默认路径；
//! - `CopyOnly`：v1 不支持，返回 [`SkillError::Unsupported`]。

use std::path::Path;

use crate::error::SkillError;
use crate::model::LinkKind;

/// 建立目录链接。
pub fn create(link: &Path, target: &Path, kind: LinkKind) -> Result<(), SkillError> {
    if kind == LinkKind::CopyOnly {
        return Err(SkillError::Unsupported(
            "该工具不跟随目录链接，v1 暂不支持拷贝模式".into(),
        ));
    }
    platform::create(link, target, kind)
}

/// 删除链接本身（绝不删除链接指向的内容）。
pub fn remove(link: &Path) -> Result<(), SkillError> {
    match std::fs::symlink_metadata(link) {
        Ok(_) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err.into()),
    }
    if !is_link(link) {
        return Err(SkillError::Conflict(format!(
            "{} 不是链接，拒绝删除",
            link.display()
        )));
    }
    platform::remove(link).map_err(|err| SkillError::LinkFailed(err.to_string()))
}

/// 判断路径是否代表链接（Unix symlink / Windows symlink 或 junction）。
///
/// Windows 的 junction 在部分 std 版本里 `file_type().is_symlink()` 为 true，
/// 但为稳妥起见额外用 `junction::exists` 判一次，避免把分叉目录误判成实体目录。
pub fn is_link(path: &Path) -> bool {
    let is_symlink = std::fs::symlink_metadata(path)
        .map(|meta| meta.file_type().is_symlink())
        .unwrap_or(false);
    if is_symlink {
        return true;
    }
    #[cfg(windows)]
    {
        junction::exists(path).unwrap_or(false)
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// 读取链接指向。
pub fn read_target(link: &Path) -> Result<std::path::PathBuf, SkillError> {
    std::fs::read_link(link)
        .map_err(|err| SkillError::LinkFailed(format!("{}：{err}", link.display())))
}

#[cfg(unix)]
mod platform {
    use std::path::Path;

    pub fn create(
        link: &Path,
        target: &Path,
        _kind: crate::model::LinkKind,
    ) -> Result<(), crate::error::SkillError> {
        // 已存在链接则先摘除，保证幂等（悬空链重建路径）。
        if super::is_link(link) {
            remove(link).map_err(|err| crate::error::SkillError::LinkFailed(err.to_string()))?;
        }
        std::os::unix::fs::symlink(target, link).map_err(|err| {
            crate::error::SkillError::LinkFailed(format!("{}：{err}", link.display()))
        })
    }

    pub fn remove(link: &Path) -> std::io::Result<()> {
        std::fs::remove_file(link)
    }
}

#[cfg(windows)]
mod platform {
    use std::path::Path;

    use crate::error::SkillError;
    use crate::model::LinkKind;

    pub fn create(link: &Path, target: &Path, kind: LinkKind) -> Result<(), SkillError> {
        // 已存在链接（含悬空 junction）先摘除，否则 junction::create 会因目录已存在失败。
        if super::is_link(link) {
            remove(link).map_err(|err| SkillError::LinkFailed(err.to_string()))?;
        }
        match kind {
            // 两个变体都走 junction：Windows 上 symlink_dir 需要开发者模式。
            LinkKind::Junction | LinkKind::Symlink => {
                junction::create(target, link).map_err(|err| {
                    SkillError::LinkFailed(format!("junction {}：{err}", link.display()))
                })
            }
            LinkKind::CopyOnly => Err(SkillError::Unsupported(
                "该工具不跟随目录链接，v1 暂不支持拷贝模式".into(),
            )),
        }
    }

    pub fn remove(link: &Path) -> std::io::Result<()> {
        // junction / 目录符号链接都用 remove_dir 摘除（只删链接，不碰内容）。
        std::fs::remove_dir(link)
    }
}

#[cfg(not(any(unix, windows)))]
mod platform {
    use std::path::Path;

    use crate::error::SkillError;
    use crate::model::LinkKind;

    pub fn create(_: &Path, _: &Path, _: LinkKind) -> Result<(), SkillError> {
        Err(SkillError::Unsupported("当前平台不支持目录链接".into()))
    }

    pub fn remove(_: &Path) -> std::io::Result<()> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "当前平台不支持目录链接",
        ))
    }
}
