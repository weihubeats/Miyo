//! 应用数据目录解析：默认位置 + 用户覆盖。
//!
//! - 默认：`{系统数据目录}/Miyo`（开发构建为 `Miyo-dev`，与正式数据隔离）；
//! - 覆盖优先级：`MIYO_DATA_DIR` 环境变量 > 默认目录下的 `data_dir.txt`（设置页写入）> 默认。
//! - 只有绝对路径生效（相对/空内容视为未配置并警告）；调用方负责建目录与可写校验。
//!
//! 注意：miyo-secret 的密钥 historically 固定在 `Miyo`（无 -dev 后缀），
//! 因此覆盖解析与默认后缀分开暴露，各调用方保持自己的历史默认行为。
//!
//! 品牌更名（RustFox → Miyo）兼容：[`migrate_legacy_default_dirs`] 在进程启动
//! 最早阶段把旧默认目录整目录改名（库 / 密钥 / 日志 / bootstrap 覆盖文件
//! 一次带入）；旧环境变量名 `RUSTFOX_DATA_DIR` 继续兜底读取，但文档只宣传新名。

use std::path::PathBuf;

/// 用户覆盖环境变量（绝对路径）。
pub const DATA_DIR_ENV: &str = "MIYO_DATA_DIR";

/// 旧版覆盖环境变量（品牌更名前）。仅在 [`DATA_DIR_ENV`] 未设置时兜底读取。
pub const LEGACY_DATA_DIR_ENV: &str = "RUSTFOX_DATA_DIR";

/// 存覆盖路径的 bootstrap 文件名（位于**默认**数据目录，位置固定才能自举）。
pub const DATA_DIR_BOOTSTRAP_FILE: &str = "data_dir.txt";

/// 测试钩子：覆盖 bootstrap 文件所在目录（生产代码永不设置）。
pub const BOOTSTRAP_DIR_ENV: &str = "MIYO_BOOTSTRAP_DIR";

/// 旧版默认数据目录名 → 新名（品牌更名前 → 更名后），含 dev 后缀变体。
pub const LEGACY_DEFAULT_SUBDIRS: [(&str, &str); 2] =
    [("RustFox", "Miyo"), ("RustFox-dev", "Miyo-dev")];

/// 开发构建的数据目录后缀（与正式版隔离，避免迁移版本污染）。
pub fn default_data_subdir() -> &'static str {
    if cfg!(debug_assertions) {
        "Miyo-dev"
    } else {
        "Miyo"
    }
}

/// 系统数据目录（`dirs::data_dir`，不可用时回退当前目录）。
pub fn system_data_dir() -> PathBuf {
    dirs::data_dir().unwrap_or_else(|| PathBuf::from("."))
}

/// 品牌更名迁移：旧默认数据目录整目录改名（`RustFox` → `Miyo`、
/// `RustFox-dev` → `Miyo-dev`），库 / master.key / 日志 / 备份 / bootstrap
/// 覆盖文件一次带入，必须在任何 `default_data_dir()` / bootstrap 读取之前调用。
///
/// 幂等且不覆盖：新目录已存在（全新安装 / 已迁移）或旧目录不存在时为空操作；
/// 新旧并存（极少见，如本机跑过更名后的 dev 构建）时保留新目录、不合并，
/// 旧目录原样留存由用户手动处理。失败仅告警不阻断（失败时密钥层面还有
/// miyo-secret 的旧路径兜底读取）。
pub fn migrate_legacy_default_dirs() {
    migrate_legacy_default_dirs_in(&system_data_dir());
}

/// [`migrate_legacy_default_dirs`] 的可注入实现（测试传临时目录）。
fn migrate_legacy_default_dirs_in(system_dir: &std::path::Path) {
    for (legacy, target) in LEGACY_DEFAULT_SUBDIRS {
        let legacy_dir = system_dir.join(legacy);
        let target_dir = system_dir.join(target);
        if !legacy_dir.is_dir() || target_dir.exists() {
            continue;
        }
        if let Err(e) = std::fs::rename(&legacy_dir, &target_dir) {
            tracing::warn!("旧数据目录 {legacy} 迁移失败（不影响本次启动）：{e}");
        } else {
            tracing::info!("旧数据目录已迁移：{legacy} → {target}");
        }
    }
}

/// 默认数据目录（含 dev 后缀）。
pub fn default_data_dir() -> PathBuf {
    system_data_dir().join(default_data_subdir())
}

/// bootstrap 文件路径（覆盖目录配置本身必须放在固定位置）。
pub fn bootstrap_path() -> PathBuf {
    if let Some(dir) = std::env::var_os(BOOTSTRAP_DIR_ENV) {
        let dir = PathBuf::from(dir);
        if dir.is_absolute() {
            return dir.join(DATA_DIR_BOOTSTRAP_FILE);
        }
        tracing::warn!("{BOOTSTRAP_DIR_ENV} 非绝对路径，已忽略（测试钩子仅接受绝对路径）");
    }
    default_data_dir().join(DATA_DIR_BOOTSTRAP_FILE)
}

/// 用户覆盖的数据目录（环境变量优先，其次 bootstrap 文件；均无效时 None）。
///
/// 环境变量读取新名 [`DATA_DIR_ENV`] 在前；旧名 [`LEGACY_DATA_DIR_ENV`]
/// 兜底（更名前已配置覆盖的用户升级后覆盖继续生效）。
pub fn data_dir_override() -> Option<PathBuf> {
    data_dir_from_env().or_else(read_bootstrap_override)
}

/// 仅环境变量来源的覆盖（不含 bootstrap）：`MIYO_DATA_DIR` 优先，`RUSTFOX_DATA_DIR` 兜底。
///
/// 与 [`data_dir_override`] 分开是为了让「写 bootstrap 有没有用」这件事可判断——
/// env 优先级更高，库版本降级兜底在 env 生效时写 bootstrap 是无效动作。
pub fn data_dir_from_env() -> Option<PathBuf> {
    for (key, is_legacy) in [(DATA_DIR_ENV, false), (LEGACY_DATA_DIR_ENV, true)] {
        if let Some(raw) = std::env::var_os(key) {
            let p = PathBuf::from(raw);
            if p.is_absolute() {
                if is_legacy {
                    tracing::info!("检测到旧环境变量 {key}，请改用 {DATA_DIR_ENV}");
                }
                return Some(p);
            }
            tracing::warn!("{key} 非绝对路径，已忽略");
        }
    }
    None
}

/// 读取 bootstrap 文件中的覆盖路径（空文件/相对路径视为未配置）。
fn read_bootstrap_override() -> Option<PathBuf> {
    let raw = std::fs::read_to_string(bootstrap_path()).ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let p = PathBuf::from(trimmed);
    if !p.is_absolute() {
        tracing::warn!("bootstrap 数据目录非绝对路径，已忽略");
        return None;
    }
    Some(p)
}

/// 解析生效的数据目录（覆盖 > 默认）。
pub fn resolve_data_dir_with_default(default: PathBuf) -> PathBuf {
    data_dir_override().unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    /// 环境变量是进程全局的：涉及它的用例串行执行。
    fn env_serial() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    struct EnvGuard {
        key: &'static str,
        prev: Option<std::ffi::OsString>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let prev = std::env::var_os(key);
            // 测试串行持有 env_serial 锁（edition 2021 下 set_var 本就是安全函数）。
            std::env::set_var(key, value);
            Self { key, prev }
        }

        fn remove(key: &'static str) -> Self {
            let prev = std::env::var_os(key);
            std::env::remove_var(key);
            Self { key, prev }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.prev {
                Some(v) => std::env::set_var(self.key, v),
                None => std::env::remove_var(self.key),
            }
        }
    }

    fn temp_home(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("miyo-paths-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建临时目录");
        dir
    }

    #[test]
    fn env_override_wins_over_bootstrap_and_default() {
        let _serial = env_serial();
        let home = temp_home("env-wins");
        let _env = EnvGuard::set(DATA_DIR_ENV, home.join("custom").to_str().unwrap());
        let _boot = EnvGuard::set(BOOTSTRAP_DIR_ENV, home.join("bootstrap").to_str().unwrap());
        std::fs::create_dir_all(home.join("bootstrap")).expect("建 bootstrap 目录");
        std::fs::write(
            home.join("bootstrap").join(DATA_DIR_BOOTSTRAP_FILE),
            home.join("from-file").to_str().unwrap(),
        )
        .expect("写 bootstrap");
        assert_eq!(
            data_dir_override(),
            Some(home.join("custom")),
            "环境变量优先于 bootstrap 文件"
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn bootstrap_used_when_no_env() {
        let _serial = env_serial();
        let home = temp_home("bootstrap");
        let _env = EnvGuard::remove(DATA_DIR_ENV);
        let _boot = EnvGuard::set(BOOTSTRAP_DIR_ENV, home.join("bootstrap").to_str().unwrap());
        std::fs::create_dir_all(home.join("bootstrap")).expect("建 bootstrap 目录");
        std::fs::write(
            home.join("bootstrap").join(DATA_DIR_BOOTSTRAP_FILE),
            home.join("from-file").to_str().unwrap(),
        )
        .expect("写 bootstrap");
        assert_eq!(
            data_dir_override(),
            Some(home.join("from-file")),
            "无环境变量时采用 bootstrap 文件"
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn relative_and_empty_configs_ignored() {
        let _serial = env_serial();
        let home = temp_home("invalid");
        // 相对路径的环境变量被忽略
        let _env = EnvGuard::set(DATA_DIR_ENV, "relative/path");
        assert_eq!(data_dir_override(), None);
        drop(_env);
        // 空 bootstrap 文件被忽略
        let _env = EnvGuard::remove(DATA_DIR_ENV);
        let _boot = EnvGuard::set(BOOTSTRAP_DIR_ENV, home.join("bootstrap").to_str().unwrap());
        std::fs::create_dir_all(home.join("bootstrap")).expect("建 bootstrap 目录");
        std::fs::write(home.join("bootstrap").join(DATA_DIR_BOOTSTRAP_FILE), "  \n")
            .expect("写空 bootstrap");
        assert_eq!(data_dir_override(), None);
        // 相对路径的 bootstrap 被忽略
        std::fs::write(
            home.join("bootstrap").join(DATA_DIR_BOOTSTRAP_FILE),
            "relative/path",
        )
        .expect("写相对 bootstrap");
        assert_eq!(data_dir_override(), None);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn resolve_falls_back_to_default() {
        let _serial = env_serial();
        let home = temp_home("fallback");
        let _env = EnvGuard::remove(DATA_DIR_ENV);
        let _boot = EnvGuard::set(BOOTSTRAP_DIR_ENV, home.join("missing").to_str().unwrap());
        let def = home.join("default");
        assert_eq!(resolve_data_dir_with_default(def.clone()), def);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn legacy_env_var_still_respected() {
        let _serial = env_serial();
        let home = temp_home("legacy-env");
        let _new = EnvGuard::set(DATA_DIR_ENV, home.join("new").to_str().unwrap());
        let _legacy = EnvGuard::set(LEGACY_DATA_DIR_ENV, home.join("old").to_str().unwrap());
        assert_eq!(
            data_dir_override(),
            Some(home.join("new")),
            "新环境变量优先于旧环境变量"
        );
        drop(_new);
        assert_eq!(
            data_dir_override(),
            Some(home.join("old")),
            "仅旧环境变量时兜底生效"
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn data_dir_from_env_ignores_bootstrap() {
        let _serial = env_serial();
        let home = temp_home("from-env");
        std::fs::create_dir_all(home.join("bootstrap")).expect("建 bootstrap 目录");
        std::fs::write(
            home.join("bootstrap").join(DATA_DIR_BOOTSTRAP_FILE),
            home.join("from-file").to_str().unwrap(),
        )
        .expect("写 bootstrap");
        // 只有 bootstrap：env 来源判定必须为 None（降级兜底据此决定写不写 bootstrap）
        let _env = EnvGuard::remove(DATA_DIR_ENV);
        let _legacy = EnvGuard::remove(LEGACY_DATA_DIR_ENV);
        let _boot = EnvGuard::set(BOOTSTRAP_DIR_ENV, home.join("bootstrap").to_str().unwrap());
        assert_eq!(data_dir_from_env(), None);
        assert_eq!(data_dir_override(), Some(home.join("from-file")));
        // 加上 env：env 来源命中，且仍优先于 bootstrap
        let _env = EnvGuard::set(DATA_DIR_ENV, home.join("custom").to_str().unwrap());
        assert_eq!(data_dir_from_env(), Some(home.join("custom")));
        assert_eq!(data_dir_override(), Some(home.join("custom")));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn legacy_default_dirs_migrated_and_idempotent() {
        let home = temp_home("migrate");
        // 旧目录带内容：bootstrap 覆盖文件必须随目录一起迁入
        std::fs::create_dir_all(home.join("RustFox")).expect("建旧目录");
        std::fs::write(home.join("RustFox").join(DATA_DIR_BOOTSTRAP_FILE), "/tmp/x")
            .expect("写覆盖文件");
        std::fs::create_dir_all(home.join("RustFox-dev")).expect("建旧 dev 目录");
        migrate_legacy_default_dirs_in(&home);
        assert!(home.join("Miyo").is_dir(), "正式目录已迁入");
        assert!(home.join("Miyo").join(DATA_DIR_BOOTSTRAP_FILE).is_file());
        assert!(home.join("Miyo-dev").is_dir(), "dev 目录已迁入");
        assert!(!home.join("RustFox").exists());
        // 幂等：重复调用无副作用
        migrate_legacy_default_dirs_in(&home);
        assert!(home.join("Miyo").is_dir());
        // 新旧并存：保留新目录，不合并、不覆盖
        std::fs::create_dir_all(home.join("RustFox")).expect("重建旧目录");
        std::fs::write(home.join("RustFox").join("marker"), "old").expect("写标记");
        migrate_legacy_default_dirs_in(&home);
        assert!(
            home.join("RustFox").is_dir(),
            "新目录已存在时旧目录原样保留"
        );
        assert!(
            !home.join("Miyo").join("marker").exists(),
            "不把旧文件并入新目录"
        );
        let _ = std::fs::remove_dir_all(&home);
    }
}
