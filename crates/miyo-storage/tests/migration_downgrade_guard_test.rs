//! 迁移降级回归：用户从新版回退旧版打开「被新版本升级过」的库（issue #153）。
//!
//! 覆盖：
//! 1. 报错信息点名版本不匹配，而不是笼统的「数据库操作失败」；
//! 2. 启动兜底 `redirect_newer_data_dir` 能把数据目录改指向新目录，老库原样留存；
//! 3. 三条不抢占护栏：隔离目录已有库 / bootstrap 已被用户配置 / 写配置失败；
//! 4. 改指向落点跟随覆盖来源：env 覆盖生效时不动 bootstrap（写了也无效，必须改 env）。

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};

use uuid::Uuid;

use miyo_storage::db;

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("miyo-downgrade-{tag}-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn temp_db(tag: &str) -> (PathBuf, PathBuf) {
    let dir = temp_dir(tag);
    let path = dir.join("miyo.db");
    (dir, path)
}

/// 正常建库后再塞一条「未来版本」的迁移记录：模拟库被更高版本应用升级过。
async fn build_newer_db(path: &Path) {
    let pool = db::init_db(path).await.unwrap();
    sqlx::query(
        "INSERT INTO _sqlx_migrations (version, description, installed_on, success, checksum, execution_time)
         VALUES (999999, 'future_migration', datetime('now'), TRUE, x'00', 1)",
    )
    .execute(&pool)
    .await
    .unwrap();
    pool.close().await;
}

/// env 是进程全局的：涉及它的用例串行。
/// 用 std 锁而不是 tokio 锁：guard 要活到 `await` 之后，tokio 的 guard 不能跨 await 持有。
fn env_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// 一次性的 env 覆盖 + 退出时还原（作用域结束时执行，panic 也还原）。
struct EnvGuard {
    prev: Vec<(&'static str, Option<std::ffi::OsString>)>,
    _lock: MutexGuard<'static, ()>,
}

impl EnvGuard {
    /// set: 需要设置的环境变量；remove: 需要清空的环境变量。
    fn new(set: &[(&'static str, &Path)], remove: &[&'static str]) -> Self {
        let lock = env_lock();
        let mut prev = Vec::new();
        for (key, value) in set {
            prev.push((*key, std::env::var_os(key)));
            std::env::set_var(key, value);
        }
        for key in remove {
            prev.push((*key, std::env::var_os(key)));
            std::env::remove_var(key);
        }
        Self { prev, _lock: lock }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (key, value) in &self.prev {
            match value {
                Some(v) => std::env::set_var(key, v),
                None => std::env::remove_var(key),
            }
        }
    }
}

/// 旧版本程序启动：必须报「版本」相关提示（不是「数据库操作失败」）。
#[tokio::test]
async fn newer_schema_reports_version_mismatch() {
    let (dir, path) = temp_db("newer");
    build_newer_db(&path).await;

    let err = db::init_db(&path)
        .await
        .expect_err("应拒绝打开更高版本的库");
    let msg = err.user_message();
    assert!(
        msg.contains("更新版本") && msg.contains("999999"),
        "降级错误必须提示版本不匹配，实际：{msg}"
    );
    assert_ne!(msg, "数据库操作失败");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 库版本正常时不做任何改指向。
#[tokio::test]
async fn current_schema_not_redirected() {
    let (dir, path) = temp_db("current");
    {
        let pool = db::init_db(&path).await.unwrap();
        pool.close().await;
    }
    assert_eq!(
        db::max_applied_version(&path).await,
        Some(db::schema_version())
    );
    assert!(
        db::redirect_newer_data_dir(&path).await.is_none(),
        "版本一致不应改指向"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 库比程序新、数据目录来自默认目录（无 env / 无用户 bootstrap）：
/// 写 bootstrap 改指向隔离目录，老库不动。
///
/// bootstrap 位置用 `BOOTSTRAP_DIR_ENV` 固定在临时目录（否则会写用户的真实配置），
/// 覆盖「默认目录 → 写 bootstrap」这条主路径；用户已配置 bootstrap 的情况见
/// `user_bootstrap_override_is_respected`，两者靠 bootstrap 文件内容区分。
#[tokio::test]
async fn newer_schema_redirects_to_isolated_dir() {
    let root = temp_dir("redirect");
    let current = root.join("Miyo");
    std::fs::create_dir_all(&current).unwrap();
    let path = current.join("miyo.db");
    build_newer_db(&path).await;

    // 不设 MIYO_DATA_DIR：数据目录来源只有 bootstrap（测试里指向临时 root），
    // 覆盖「默认目录 → 写 bootstrap」这条主路径。
    let _env = EnvGuard::new(
        &[(miyo_core::paths::BOOTSTRAP_DIR_ENV, &root)],
        &[
            miyo_core::paths::DATA_DIR_ENV,
            miyo_core::paths::LEGACY_DATA_DIR_ENV,
        ],
    );
    let target = db::redirect_newer_data_dir(&path).await;

    let target = target.expect("库更新时应改指向新目录");
    assert!(target.is_dir(), "新数据目录已创建：{target:?}");
    assert_ne!(target, current);
    assert_eq!(
        target.parent(),
        Some(root.as_path()),
        "隔离目录与库同父目录"
    );
    let name = target.file_name().unwrap().to_string_lossy().to_string();
    assert!(
        name.starts_with("Miyo-v999999-p"),
        "隔离目录名带库版本与程序版本：{name}"
    );
    assert!(path.exists(), "老库必须原样留存");
    let written = std::fs::read_to_string(root.join("data_dir.txt")).unwrap();
    assert_eq!(written, target.to_string_lossy());
    let _ = std::fs::remove_dir_all(&root);
}

/// 护栏 1：隔离目录里已经有数据库 → 不抢占（可能是另一份还在用的数据），返回 None。
#[tokio::test]
async fn existing_isolated_db_is_not_overwritten() {
    let root = temp_dir("guard-occupied");
    let current = root.join("Miyo");
    std::fs::create_dir_all(&current).unwrap();
    let path = current.join("miyo.db");
    build_newer_db(&path).await;

    let _env = EnvGuard::new(
        &[(miyo_core::paths::BOOTSTRAP_DIR_ENV, &root)],
        &[
            miyo_core::paths::DATA_DIR_ENV,
            miyo_core::paths::LEGACY_DATA_DIR_ENV,
        ],
    );
    // 先跑一次拿到目标目录，再往里面放一个库文件模拟「已被占用」。
    let first = db::redirect_newer_data_dir(&path)
        .await
        .expect("首次应改指向");
    std::fs::write(first.join("miyo.db"), b"existing").unwrap();
    // 把 bootstrap 清回「未配置」，只留被占用的隔离目录这一个变量。
    std::fs::remove_file(root.join("data_dir.txt")).unwrap();

    let second = db::redirect_newer_data_dir(&path).await;
    assert!(second.is_none(), "隔离目录已有库时不得覆盖：{second:?}");
    assert!(
        !root.join("data_dir.txt").exists(),
        "未改指向就不该写 bootstrap"
    );
    assert_eq!(std::fs::read(first.join("miyo.db")).unwrap(), b"existing");
    let _ = std::fs::remove_dir_all(&root);
}

/// 护栏 2：bootstrap 已被用户显式配置 → 保留用户配置，不擅自改。
#[tokio::test]
async fn user_bootstrap_override_is_respected() {
    let root = temp_dir("guard-bootstrap");
    let current = root.join("Miyo");
    std::fs::create_dir_all(&current).unwrap();
    let path = current.join("miyo.db");
    build_newer_db(&path).await;
    let user_dir = root.join("my-old-data");
    std::fs::write(
        root.join("data_dir.txt"),
        user_dir.to_string_lossy().as_bytes(),
    )
    .unwrap();

    let _env = EnvGuard::new(
        &[(miyo_core::paths::BOOTSTRAP_DIR_ENV, &root)],
        &[
            miyo_core::paths::DATA_DIR_ENV,
            miyo_core::paths::LEGACY_DATA_DIR_ENV,
        ],
    );
    // 用户配置生效时 data_dir() 是 my-old-data，调用方实际探查的是那里的库；
    // 这里直接探查一个「版本更新」的库，模拟旧程序面对的场景。
    let target = db::redirect_newer_data_dir(&path).await;

    assert!(target.is_none(), "用户配置过目录时不得改指向：{target:?}");
    let written = std::fs::read_to_string(root.join("data_dir.txt")).unwrap();
    assert_eq!(written, user_dir.to_string_lossy(), "用户配置原样保留");
    assert!(!user_dir.exists(), "不得替用户建那个目录");
    let _ = std::fs::remove_dir_all(&root);
}

/// 护栏 3：写 bootstrap 失败（父目录不存在 / 只读）→ 返回 None，不谎报已切换。
#[tokio::test]
async fn bootstrap_write_failure_is_reported_as_noop() {
    let root = temp_dir("guard-write");
    let current = root.join("Miyo");
    std::fs::create_dir_all(&current).unwrap();
    let path = current.join("miyo.db");
    build_newer_db(&path).await;
    // 指向一个不存在的目录：写 data_dir.txt 必然失败。
    let ghost = root.join("nv").join("missing").join("bootstrap");

    let _env = EnvGuard::new(
        &[(miyo_core::paths::BOOTSTRAP_DIR_ENV, &ghost)],
        &[
            miyo_core::paths::DATA_DIR_ENV,
            miyo_core::paths::LEGACY_DATA_DIR_ENV,
        ],
    );
    let target = db::redirect_newer_data_dir(&path).await;

    assert!(target.is_none(), "写配置失败不得返回新目录：{target:?}");
    assert!(!ghost.join("data_dir.txt").exists());
    let _ = std::fs::remove_dir_all(&root);
}

/// 覆盖来源是 env：不写 bootstrap（写了也无效，env 优先级更高），
/// 返回的隔离目录基于环境变量指向的目录，由调用方提示用户改 env。
#[tokio::test]
async fn env_override_reports_target_without_touching_bootstrap() {
    let root = temp_dir("guard-env");
    let from_env = root.join("FromEnv");
    std::fs::create_dir_all(&from_env).unwrap();
    let path = from_env.join("miyo.db");
    build_newer_db(&path).await;

    let _env = EnvGuard::new(
        &[
            (miyo_core::paths::DATA_DIR_ENV, &from_env),
            (miyo_core::paths::BOOTSTRAP_DIR_ENV, &root),
        ],
        &[miyo_core::paths::LEGACY_DATA_DIR_ENV],
    );
    let target = db::redirect_newer_data_dir(&path).await;

    let target = target.expect("env 来源也应给出目标目录（供提示改 env）");
    assert_eq!(
        target.parent(),
        Some(root.as_path()),
        "隔离目录基于 env 指向的目录，而不是默认目录"
    );
    let name = target.file_name().unwrap().to_string_lossy().to_string();
    assert!(name.starts_with("FromEnv-v999999-p"), "实际：{name}");
    assert!(
        !root.join("data_dir.txt").exists(),
        "env 覆盖生效时不得写 bootstrap（优先于它，写了无效）"
    );
    assert!(path.exists(), "老库原样留存");
    let _ = std::fs::remove_dir_all(&root);
}
