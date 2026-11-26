//! 渲染活性与无界面兜底（issue #192）。
//!
//! Windows Server + 虚拟机上出现过「页面加载完成、`frontend ready` 已上报、
//! 窗口却一个像素都不刷」的案例：WebView2 渲染进程活着（CPU 近 0），
//! 页面输出写到一块永不呈现的表面上。此时：
//!
//! - 8 秒首帧看门狗**不会**触发（就绪事件准时到了），旧兜底完全失效；
//! - 用户连「设置 → 日志」都点不到，只能干瞪眼。
//!
//! 本模块提供两个能力：
//! 1. [`render_heartbeat`]：前端在 `requestAnimationFrame` 里周期性调用，
//!    壳侧据此判断「画面是否还在刷新」（计数停滞 = 画面冻结）；
//! 2. [`export_database_without_ui`]：进程内直接把当前库复制/导出到指定路径，
//!    供 `--miyo-export-db=<路径>` 使用——窗口点不动时仍能把数据救出来。

use crate::error::{CommandError, CommandErrorCode, CommandResult};

/// 前端渲染心跳计数（单调递增，主线程 rAF 驱动）。
///
/// 只在壳侧（`frontend/src-tauri`）与前端之间有意义：壳侧定时读取并与上次比较，
/// 数值不变即说明 `requestAnimationFrame` 停了——典型的「合成挂死」。
static RENDER_HEARTBEAT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// 前端调用一次心跳（`requestAnimationFrame` 回调里调用）。
#[tauri::command(rename_all = "camelCase")]
pub async fn render_heartbeat() -> CommandResult<()> {
    RENDER_HEARTBEAT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    Ok(())
}

/// 读取当前心跳计数（壳侧轮询用，非 Command）。
pub fn render_heartbeat_count() -> u64 {
    RENDER_HEARTBEAT.load(std::sync::atomic::Ordering::Relaxed)
}

/// 无界面导出数据库：把当前 sqlite 库文件复制到 `dest`。
///
/// 供 `--miyo-export-db=<路径>` 启动参数使用：窗口画不出来时，用户仍可
/// 通过命令行把数据捞出来（避免「黑屏 = 数据变砖」）。
///
/// 走 `VACUUM INTO` 而非直接复制文件：WAL 中未 checkpoint 的已提交事务
/// 也会被带上，得到的是首尾一致的单文件快照。目标已存在时覆盖。
pub fn export_database_without_ui(
    db: &sqlx::SqlitePool,
    dest: &std::path::Path,
) -> CommandResult<()> {
    if dest.as_os_str().is_empty() {
        return Err(CommandError::validation("导出路径不能为空"));
    }
    if let Some(parent) = dest.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| {
                CommandError::with_code(
                    CommandErrorCode::Io,
                    format!("创建导出目录失败（{}）：{e}", parent.display()),
                )
            })?;
        }
    }
    let dest = dest.to_string_lossy().to_string();
    // VACUUM INTO 不接受已存在的目标文件，先删再导。
    if std::path::Path::new(&dest).exists() {
        std::fs::remove_file(&dest).map_err(|e| {
            CommandError::with_code(
                CommandErrorCode::Io,
                format!("覆盖已有导出文件失败（{dest}）：{e}"),
            )
        })?;
    }
    tauri::async_runtime::block_on(async {
        // 路径作为绑定参数传入，避免拼接 SQL 时的引号转义问题。
        sqlx::query("VACUUM INTO ?")
            .bind(&dest)
            .execute(db)
            .await
            .map_err(|e| {
                CommandError::with_code(
                    CommandErrorCode::Io,
                    format!("导出数据库失败（{dest}）：{e}"),
                )
            })
            .map(|_| ())
    })
}

/// 把一段诊断摘要追加写进日志目录（黑屏时用户拿不到 UI，只能靠文件）。
///
/// 文件名固定 `blank-screen-<本地时间>.txt`，不参与日志清理（不是 `miyo.log.*` 前缀），
/// 用户可直接把整个文件发出来。
pub fn write_blank_screen_report(body: &str) -> Option<std::path::PathBuf> {
    let dir = miyo_storage::db::log_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return None;
    }
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let path = dir.join(format!("blank-screen-{stamp}.txt"));
    std::fs::write(&path, body).ok()?;
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heartbeat_is_monotonic() {
        let before = render_heartbeat_count();
        tauri::async_runtime::block_on(render_heartbeat()).unwrap();
        assert!(render_heartbeat_count() > before);
    }

    #[test]
    fn export_rejects_empty_path() {
        let pool = tauri::async_runtime::block_on(sqlx::SqlitePool::connect("sqlite::memory:"))
            .expect("memory db");
        let err = export_database_without_ui(&pool, std::path::Path::new("")).unwrap_err();
        assert!(err.to_string().contains("导出路径不能为空"));
    }

    /// `VACUUM INTO` 对 `:memory:` 库是空操作（不会落盘），因此用临时文件库验证。
    #[test]
    fn export_writes_consistent_snapshot_and_overwrites() {
        let dir = std::env::temp_dir().join(format!("miyo-export-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("source.db");
        let dest = dir.join("out").join("snapshot.db");

        let pool = tauri::async_runtime::block_on(async {
            let opts = sqlx::sqlite::SqliteConnectOptions::new()
                .filename(&source)
                .create_if_missing(true);
            let pool = sqlx::SqlitePool::connect_with(opts).await.expect("file db");
            sqlx::query("CREATE TABLE t (v TEXT)")
                .execute(&pool)
                .await
                .unwrap();
            sqlx::query("INSERT INTO t VALUES ('a')")
                .execute(&pool)
                .await
                .unwrap();
            pool
        });

        export_database_without_ui(&pool, &dest).unwrap();
        assert!(dest.exists(), "首次导出应生成文件");
        // 导出产物本身是完整的 sqlite 库：能独立打开并读出数据。
        let rows: (i64,) = tauri::async_runtime::block_on(async {
            let snap = sqlx::SqlitePool::connect(&format!("sqlite://{}", dest.display()))
                .await
                .expect("open snapshot");
            sqlx::query_as("SELECT COUNT(*) FROM t")
                .fetch_one(&snap)
                .await
                .expect("count rows")
        });
        assert_eq!(rows.0, 1, "导出的快照应包含数据行");

        // 二次导出覆盖同一路径（VACUUM INTO 自身不接受已存在目标）。
        export_database_without_ui(&pool, &dest).unwrap();
        assert!(dest.exists());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
