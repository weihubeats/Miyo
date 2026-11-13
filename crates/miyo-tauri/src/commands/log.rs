//! 日志查看 Command：日志文件列表 / 尾部读取 / 目录路径 / 过期清理。
//!
//! 日志由 `init_tracing` 按天滚动（`{log_dir}/miyo.log.YYYY-MM-DD`）。
//! 原来只能靠顶栏「反馈」生成一次性诊断摘要；本模块给设置页"日志" Tab
//! 提供查看面，排障不再需要用户翻文件系统。
//!
//! 过期清理：按 settings 中的保留天数（默认 [`DEFAULT_RETENTION_DAYS`]），
//! 应用启动时与修改保留天数时各扫一遍，删除超过保留期的历史滚动文件；
//! 当日活动文件 `miyo.log`（无日期后缀）永不删除。

use serde::Serialize;
use tauri::State;

use miyo_storage::repository as repo;

use crate::error::{CommandError, CommandErrorCode, CommandResult};
use crate::state::AppState;

/// settings 表中的日志保留天数键；值为 JSON 数字。缺失时用默认值。
const LOG_RETENTION_KEY: &str = "log_retention_days";

/// 默认保留天数（「保留最近 14 天」：含今天在内共 14 个日历日）。
pub const DEFAULT_RETENTION_DAYS: u32 = 14;

/// 允许的保留天数范围。
const MIN_RETENTION_DAYS: u32 = 1;
const MAX_RETENTION_DAYS: u32 = 365;

/// 日志文件元信息（按修改时间倒序）。
#[derive(Debug, Clone, Serialize)]
pub struct LogFile {
    pub name: String,
    pub size_bytes: u64,
    pub modified_at: String,
}

/// 列出日志文件（最新在前；目录缺失返回空）。
#[tauri::command(rename_all = "camelCase")]
pub async fn log_files() -> CommandResult<Vec<LogFile>> {
    // 目录扫描 + 元数据读取为同步文件 IO：挪到阻塞线程池。
    tokio::task::spawn_blocking(list_log_files)
        .await
        .map_err(|e| CommandError::with_code(CommandErrorCode::Internal, format!("日志扫描任务失败：{e}")))?
}

/// 同步实现（阻塞线程池内执行）。
fn list_log_files() -> CommandResult<Vec<LogFile>> {
    let dir = miyo_storage::db::log_dir();
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return Ok(out);
    };
    for entry in rd.filter_map(|e| e.ok()) {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        // 当前与更名前的前缀都列出（旧滚动文件在 UI 里同样可见、可清理）
        if !(name.starts_with("miyo.log") || name.starts_with("rustfox.log")) {
            continue;
        }
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if !meta.is_file() || path.extension().is_some_and(|e| e == "gz") {
            continue;
        }
        let modified_at = meta
            .modified()
            .ok()
            .and_then(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339().into())
            .unwrap_or_default();
        out.push(LogFile {
            name,
            size_bytes: meta.len(),
            modified_at,
        });
    }
    out.sort_by(|a, b| b.name.cmp(&a.name));
    Ok(out)
}

const MAX_TAIL_BYTES: u64 = 512 * 1024;
const MAX_LINES: u64 = 2000;

/// 读取日志尾部（默认 300 行，上限 2000 行；只读末尾 512KB，大文件不爆内存）。
#[tauri::command(rename_all = "camelCase")]
pub async fn log_tail(file: Option<String>, lines: Option<u64>) -> CommandResult<String> {
    let dir = miyo_storage::db::log_dir();
    let name = file.unwrap_or_else(|| "miyo.log".to_string());
    // 文件名限定：防目录穿越。
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err(CommandError::validation("非法的文件名"));
    }
    let want = lines.unwrap_or(300).clamp(1, MAX_LINES) as usize;
    // tail 读取（seek + read_to_end）为同步 IO：挪进 blocking 线程池，
    // 与同文件 list_log_files 的口径一致。
    tokio::task::spawn_blocking(move || read_log_tail(dir.join(&name), want))
        .await
        .map_err(|e| CommandError::with_code(CommandErrorCode::Internal, format!("日志读取任务失败：{e}")))?
}

/// 同步实现：只读文件末尾 [`MAX_TAIL_BYTES`]，返回最后 `want` 行。
fn read_log_tail(path: std::path::PathBuf, want: usize) -> CommandResult<String> {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("miyo.log")
        .to_string();
    let mut file = std::fs::File::open(&path)
        .map_err(|e| CommandError::with_code(CommandErrorCode::Io, format!("读取日志失败（{name}）：{e}")))?;
    use std::io::{Read, Seek, SeekFrom};
    let len = file
        .metadata()
        .map_err(|e| CommandError::with_code(CommandErrorCode::Io, format!("读取日志失败（{name}）：{e}")))?
        .len();
    let offset = len.saturating_sub(MAX_TAIL_BYTES);
    file.seek(SeekFrom::Start(offset))
        .map_err(|e| CommandError::with_code(CommandErrorCode::Io, format!("读取日志失败（{name}）：{e}")))?;
    let mut content = Vec::with_capacity((len - offset) as usize);
    file.read_to_end(&mut content)
        .map_err(|e| CommandError::with_code(CommandErrorCode::Io, format!("读取日志失败（{name}）：{e}")))?;
    // 非文件起点时跳过被截断的 UTF-8 首字符（字符边界安全）。
    let mut start = 0usize;
    if offset > 0 {
        while start < content.len() && !is_utf8_boundary(&content, start) {
            start += 1;
        }
    }
    let text = String::from_utf8_lossy(&content[start..]);
    let all: Vec<&str> = text.lines().collect();
    let from = all.len().saturating_sub(want);
    Ok(all[from..].join("\n"))
}

fn is_utf8_boundary(bytes: &[u8], index: usize) -> bool {
    if index >= bytes.len() {
        return true;
    }
    (bytes[index] as i8) >= -0x40
}

/// 日志目录绝对路径（供「打开目录」）。
#[tauri::command(rename_all = "camelCase")]
pub async fn log_dir_path() -> CommandResult<String> {
    Ok(miyo_storage::db::log_dir().to_string_lossy().to_string())
}

/// 读取日志保留天数（缺键 / 越界 / 解析失败时回退默认值）。
pub async fn read_log_retention_days(db: &sqlx::SqlitePool) -> u32 {
    match repo::get_setting(db, LOG_RETENTION_KEY).await {
        Ok(Some(json)) => serde_json::from_str::<u32>(&json)
            .ok()
            .filter(|d| (MIN_RETENTION_DAYS..=MAX_RETENTION_DAYS).contains(d))
            .unwrap_or(DEFAULT_RETENTION_DAYS),
        _ => DEFAULT_RETENTION_DAYS,
    }
}

/// 读取日志保留天数（设置页展示）。
#[tauri::command(rename_all = "camelCase")]
pub async fn get_log_retention_days(state: State<'_, AppState>) -> CommandResult<u32> {
    Ok(read_log_retention_days(&state.db).await)
}

/// 设置日志保留天数（1~365）并立即按新规则清理一次。
#[tauri::command(rename_all = "camelCase")]
pub async fn set_log_retention_days(state: State<'_, AppState>, days: u32) -> CommandResult<()> {
    if !(MIN_RETENTION_DAYS..=MAX_RETENTION_DAYS).contains(&days) {
        return Err(CommandError::validation("保留天数需在 1 ~ 365 之间"));
    }
    let json = serde_json::to_string(&days)
        .map_err(|e| CommandError::with_code(CommandErrorCode::Internal, format!("序列化失败：{e}")))?;
    repo::set_setting(&state.db, LOG_RETENTION_KEY, &json).await?;
    // 目录扫描 + 删除为同步文件 IO：挪到阻塞线程池，不占 IPC 线程。
    let dir = miyo_storage::db::log_dir();
    let removed = tokio::task::spawn_blocking(move || cleanup_expired_logs(&dir, days))
        .await
        .map_err(|e| CommandError::with_code(CommandErrorCode::Internal, format!("日志清理任务失败：{e}")))?;
    if removed > 0 {
        tracing::info!("[log] 按保留 {days} 天清理，删除 {removed} 个过期日志文件");
    }
    Ok(())
}

/// 滚动日志文件名前缀：当前 `miyo.log.` + 品牌更名前的历史前缀
/// `rustfox.log.`（旧滚动文件同样参与保留期清理，避免更名前的日志永久残留）。
const LOG_ROLLED_PREFIXES: [&str; 2] = ["miyo.log.", "rustfox.log."];

/// 从滚动文件名解析日期：`miyo.log.YYYY-MM-DD` → `YYYY-MM-DD`。
/// 活动文件（无日期后缀）返回 None。
fn rolled_date_of(name: &str) -> Option<&str> {
    let rest = LOG_ROLLED_PREFIXES
        .iter()
        .find_map(|p| name.strip_prefix(p))?;
    let b = rest.as_bytes();
    // 严格 YYYY-MM-DD：10 字节，第 5/8 位为 '-'，其余为数字。
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    if !b
        .iter()
        .enumerate()
        .all(|(i, &c)| i == 4 || i == 7 || c.is_ascii_digit())
    {
        return None;
    }
    chrono::NaiveDate::parse_from_str(rest, "%Y-%m-%d").ok()?;
    Some(rest)
}

/// 判断历史滚动文件是否应删除：日期早于 cutoff（cutoff = today - (retention - 1)
/// 之前的那一天，即保留含今天在内的 `retention` 个日历日）。
fn is_expired_rolled(name: &str, today: chrono::NaiveDate, retention_days: u32) -> bool {
    let Some(date_str) = rolled_date_of(name) else {
        return false; // 活动文件 / 非日期后缀：不删
    };
    let Ok(date) = chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d") else {
        return false;
    };
    // 保留 age ∈ [0, retention)：age = today - date
    match today.signed_duration_since(date).num_days() {
        age if age < 0 => false, // 文件日期在未来（时钟回拨等）：不删
        age => age >= i64::from(retention_days),
    }
}

/// 扫描日志目录，删除超过保留期的滚动文件。返回删除个数。
///
/// 只处理 `miyo.log.YYYY-MM-DD`；活动文件 `miyo.log` 与无法解析
/// 日期的文件保留。目录不存在时返回 0。
pub fn cleanup_expired_logs(dir: &std::path::Path, retention_days: u32) -> usize {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return 0;
    };
    let today = chrono::Local::now().date_naive();
    let mut removed = 0usize;
    for entry in rd.filter_map(|e| e.ok()) {
        let name = entry.file_name().to_string_lossy().to_string();
        if !LOG_ROLLED_PREFIXES.iter().any(|p| name.starts_with(p)) {
            continue;
        }
        if is_expired_rolled(&name, today, retention_days) && entry.path().is_file() {
            match std::fs::remove_file(entry.path()) {
                Ok(()) => removed += 1,
                Err(e) => tracing::warn!("[log] 删除过期日志失败 {name}：{e}"),
            }
        }
    }
    removed
}

/// 启动时按保留天数清理一次（失败仅记日志，不阻断启动）。
pub fn cleanup_logs_on_startup(db: &sqlx::SqlitePool) {
    let days = tauri::async_runtime::block_on(read_log_retention_days(db));
    let removed = cleanup_expired_logs(&miyo_storage::db::log_dir(), days);
    if removed > 0 {
        tracing::info!("[log] 启动清理：保留 {days} 天，删除 {removed} 个过期日志文件");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 纯日期规则：today=2026-09-23，保留 7 天 → 只删 age≥7。
    #[test]
    fn expired_roll_files_follow_retention_window() {
        let today = chrono::NaiveDate::from_ymd_opt(2026, 9, 23).unwrap();
        // age 0（今天）~ age 6：保留
        for age in 0..7 {
            let d = today - chrono::Duration::days(age);
            let name = format!("miyo.log.{}", d.format("%Y-%m-%d"));
            assert!(!is_expired_rolled(&name, today, 7), "应保留 {name}");
        }
        // age 7、30：删除
        for age in [7, 30] {
            let d = today - chrono::Duration::days(age);
            let name = format!("miyo.log.{}", d.format("%Y-%m-%d"));
            assert!(is_expired_rolled(&name, today, 7), "应删除 {name}");
        }
    }

    #[test]
    fn active_and_malformed_names_never_expire() {
        let today = chrono::NaiveDate::from_ymd_opt(2026, 9, 23).unwrap();
        assert!(!is_expired_rolled("miyo.log", today, 1));
        assert!(!is_expired_rolled("miyo.log.not-a-date", today, 1));
        assert!(!is_expired_rolled("miyo.log.2020-01-01.bak", today, 1));
        // 未来日期（时钟回拨）不删
        assert!(!is_expired_rolled("miyo.log.2099-01-01", today, 1));
        // 活动文件的常见变体前缀匹配：日期段非法仍不删
        assert!(!is_expired_rolled("miyo.log.99-99-99", today, 1));
    }

    /// 品牌更名前的旧前缀日志与当前前缀同一套规则：
    /// 过期滚动文件参与清理，旧活动文件与畸形名永不删除。
    #[test]
    fn legacy_prefix_logs_follow_same_rules() {
        let today = chrono::NaiveDate::from_ymd_opt(2026, 9, 23).unwrap();
        assert!(
            is_expired_rolled("rustfox.log.2026-09-01", today, 7),
            "更名前的过期滚动日志应被清理"
        );
        assert!(
            !is_expired_rolled("rustfox.log", today, 1),
            "旧活动文件永不删除"
        );
        assert!(!is_expired_rolled("rustfox.log.2099-01-01", today, 1));
        assert!(!is_expired_rolled("rustfox.log.not-a-date", today, 1));
    }

    #[test]
    fn rolled_date_parses_only_iso_suffix() {
        assert_eq!(rolled_date_of("miyo.log.2026-09-22"), Some("2026-09-22"));
        assert_eq!(rolled_date_of("miyo.log"), None);
        assert_eq!(rolled_date_of("miyo.log.2026-9-2"), None);
        assert_eq!(rolled_date_of("other.log.2026-09-22"), None);
    }

    #[test]
    fn cleanup_removes_only_expired_in_temp_dir() {
        let dir = std::env::temp_dir().join(format!("miyo-log-clean-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let today = chrono::Local::now().date_naive();
        let keep = today - chrono::Duration::days(1);
        let drop = today - chrono::Duration::days(30);
        std::fs::write(dir.join("miyo.log"), b"active").unwrap();
        std::fs::write(
            dir.join(format!("miyo.log.{}", keep.format("%Y-%m-%d"))),
            b"keep",
        )
        .unwrap();
        std::fs::write(
            dir.join(format!("miyo.log.{}", drop.format("%Y-%m-%d"))),
            b"old",
        )
        .unwrap();

        let removed = cleanup_expired_logs(&dir, 7);
        assert_eq!(removed, 1, "只删 30 天前那一份");
        assert!(dir.join("miyo.log").exists(), "活动文件保留");
        assert!(
            dir.join(format!("miyo.log.{}", keep.format("%Y-%m-%d")))
                .exists(),
            "1 天前保留"
        );
        assert!(
            !dir.join(format!("miyo.log.{}", drop.format("%Y-%m-%d")))
                .exists(),
            "30 天前删除"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
