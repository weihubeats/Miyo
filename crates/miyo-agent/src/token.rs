//! Agent 令牌文件：`{data_dir}/agent-token`。
//!
//! 首次调用生成随机 UUID 并以 0600 权限写入（Windows 无 POSIX 权限，仅写文件）；
//! 已存在则直接读回，保证重启后 token 稳定、Agent 配置无需变更。

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

/// 令牌文件名（位于数据目录下）。
pub const TOKEN_FILE: &str = "agent-token";

/// 令牌文件完整路径。
pub fn token_path(data_dir: &Path) -> PathBuf {
    data_dir.join(TOKEN_FILE)
}

/// 读取或创建令牌。创建时生成 UUID v4 并尽力设置为仅所有者可读写。
pub fn load_or_create_token(data_dir: &Path) -> std::io::Result<String> {
    let path = token_path(data_dir);
    if let Ok(existing) = std::fs::read_to_string(&path) {
        let trimmed = existing.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }

    let token = uuid::Uuid::new_v4().to_string();
    // 原子写：先写临时文件再 rename。直写 truncate(true) 在 write_all 中途
    // 崩溃会留下半截非空 token 文件——下次启动把它当有效 token 复用，与
    // 控制面/端口文件失配且用户无从排查（rename 在 unix 上原子替换）。
    let tmp = data_dir.join(format!("{TOKEN_FILE}.tmp"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)?;
        file.write_all(token.as_bytes())?;
        file.sync_all().ok();
        std::fs::rename(&tmp, &path)?;
    }
    #[cfg(not(unix))]
    {
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&tmp)?;
        file.write_all(token.as_bytes())?;
        std::fs::rename(&tmp, &path).or_else(|_| {
            // rename 跨设备失败的极端情况：回退直写（尽力而为）。
            std::fs::write(&path, &token)
        })?;
    }
    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_roundtrip_is_stable() {
        let dir = std::env::temp_dir().join(format!("miyo-agent-token-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let first = load_or_create_token(&dir).expect("首次创建");
        assert!(!first.is_empty());
        // 第二次读取必须返回同一 token（Agent 侧配置不失效）。
        let second = load_or_create_token(&dir).expect("读回");
        assert_eq!(first, second);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.join(TOKEN_FILE))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600, "令牌文件应为 0600");
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_file_is_regenerated() {
        let dir =
            std::env::temp_dir().join(format!("miyo-agent-token-empty-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(TOKEN_FILE), "  \n").unwrap();
        let token = load_or_create_token(&dir).expect("空文件应重新生成");
        assert!(!token.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
