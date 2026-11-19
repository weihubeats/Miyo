//! 插件注册 / 私有存储 / 统一日志的持久化（0031_plugins）。
//!
//! - `plugins`：生命周期状态机由 [`set_plugin_state`] 强制校验（非法跃迁拒绝）；
//! - `plugin_data`：插件私有 KV，所有读写都以 plugin_id 为主键前缀（行级隔离，
//!   外键 ON DELETE CASCADE 随卸载清理）；
//! - `plugin_logs`：统一日志（debug/info/warn/error/audit），按插件滚动保留。

use sqlx::SqlitePool;

use miyo_core::{AppError, Result};
use miyo_plugin::manifest::PluginManifest;

/// 生命周期状态（plugins.state）。
pub const STATE_INSTALLED: &str = "installed";
pub const STATE_ENABLED: &str = "enabled";
pub const STATE_ERROR: &str = "error";

/// 每插件日志保留上限（插入方滚动清理）。
pub const PLUGIN_LOG_CAP: i64 = 200;

/// 插件注册行（manifest 以 JSON 原文落库，读取时重新解析——保证与校验规则同源）。
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PluginRow {
    pub id: String,
    pub kind: String,
    pub version: String,
    pub state: String,
    pub manifest_json: String,
    pub source: String,
    pub integrity: Option<String>,
    pub signed: bool,
    pub granted_permissions_json: String,
    pub install_path: Option<String>,
    pub error_json: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    /// 全文件清单最近一次复核是否通过（0033；v1 包无清单恒 0）。
    pub files_verified: bool,
    /// 最近一次全文件复核时间（RFC3339；NULL = 从未复核）。
    pub last_verified_at: Option<String>,
}

impl PluginRow {
    /// 解析清单（损坏行返回错误而不是静默丢弃，管理中心可展示并提示重装）。
    pub fn manifest(&self) -> Result<PluginManifest> {
        PluginManifest::parse(self.manifest_json.as_bytes())
    }

    /// 用户已批准的权限快照。
    pub fn granted_permissions(&self) -> Vec<String> {
        serde_json::from_str(&self.granted_permissions_json).unwrap_or_default()
    }
}

/// plugins 表 SELECT 列清单（新增列需同步此处与所有 query_as）。
const PLUGIN_COLUMNS: &str = "id, kind, version, state, manifest_json, source, integrity, signed, \
     granted_permissions_json, install_path, error_json, created_at, updated_at, \
     files_verified, last_verified_at";

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// 列出全部已安装插件（安装时间倒序）。
pub async fn list_plugins(db: &SqlitePool) -> Result<Vec<PluginRow>> {
    let rows: Vec<PluginRow> = sqlx::query_as(&format!(
        "SELECT {PLUGIN_COLUMNS} FROM plugins ORDER BY created_at DESC"
    ))
    .fetch_all(db)
    .await?;
    Ok(rows)
}

/// 按 ID 取插件行。
pub async fn get_plugin(db: &SqlitePool, id: &str) -> Result<Option<PluginRow>> {
    let row: Option<PluginRow> = sqlx::query_as(&format!(
        "SELECT {PLUGIN_COLUMNS} FROM plugins WHERE id = ?"
    ))
    .bind(id)
    .fetch_optional(db)
    .await?;
    Ok(row)
}

/// 取插件行（不存在即报错）。
pub async fn require_plugin(db: &SqlitePool, id: &str) -> Result<PluginRow> {
    get_plugin(db, id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("插件未安装：{id}")))
}

/// 安装（写入注册行）。调用方已完成清单校验 / 兼容性检查 / 权限快照确认。
#[allow(clippy::too_many_arguments)]
pub async fn insert_plugin(
    db: &SqlitePool,
    id: &str,
    manifest: &PluginManifest,
    source: &str,
    integrity: Option<&str>,
    signed: bool,
    granted_permissions: &[String],
    install_path: Option<&str>,
) -> Result<()> {
    let manifest_json = serde_json::to_string(manifest)
        .map_err(|e| AppError::Validation(format!("manifest 序列化失败：{e}")))?;
    let granted = serde_json::to_string(granted_permissions)
        .map_err(|e| AppError::Validation(format!("权限快照序列化失败：{e}")))?;
    let kind = match manifest.kind {
        miyo_plugin::manifest::PluginKind::Ui => "ui",
        miyo_plugin::manifest::PluginKind::UiSidecar => "ui-sidecar",
    };
    sqlx::query(
        "INSERT INTO plugins (id, kind, version, state, manifest_json, source, integrity, signed, \
         granted_permissions_json, install_path, error_json, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, NULL, ?, ?)",
    )
    .bind(id)
    .bind(kind)
    .bind(&manifest.version)
    .bind(STATE_INSTALLED)
    .bind(&manifest_json)
    .bind(source)
    .bind(integrity)
    .bind(signed)
    .bind(&granted)
    .bind(install_path)
    .bind(now())
    .bind(now())
    .execute(db)
    .await?;
    Ok(())
}

/// 更新插件版本 / 清单 / 完整性（builtin 或本地包升级）。状态保持不变。
pub async fn update_plugin(
    db: &SqlitePool,
    id: &str,
    manifest: &PluginManifest,
    integrity: Option<&str>,
    signed: bool,
    granted_permissions: &[String],
) -> Result<()> {
    let manifest_json = serde_json::to_string(manifest)
        .map_err(|e| AppError::Validation(format!("manifest 序列化失败：{e}")))?;
    let granted = serde_json::to_string(granted_permissions)
        .map_err(|e| AppError::Validation(format!("权限快照序列化失败：{e}")))?;
    let n = sqlx::query(
        "UPDATE plugins SET version = ?, manifest_json = ?, integrity = ?, signed = ?, \
         granted_permissions_json = ?, updated_at = ? WHERE id = ?",
    )
    .bind(&manifest.version)
    .bind(&manifest_json)
    .bind(integrity)
    .bind(signed)
    .bind(&granted)
    .bind(now())
    .bind(id)
    .execute(db)
    .await?
    .rows_affected();
    if n == 0 {
        return Err(AppError::NotFound(format!("插件未安装：{id}")));
    }
    Ok(())
}

/// 生命周期状态跃迁（带合法性校验）：
/// - `installed`：启用后停用 / 激活失败恢复落点；
/// - `enabled`：启用；
/// - `error`：激活失败（保留 error_json 供恢复）。
pub async fn set_plugin_state(
    db: &SqlitePool,
    id: &str,
    next: &str,
    error_json: Option<&str>,
) -> Result<()> {
    if !matches!(next, STATE_INSTALLED | STATE_ENABLED | STATE_ERROR) {
        return Err(AppError::Validation(format!("非法插件状态：{next}")));
    }
    let row = require_plugin(db, id).await?;
    let legal = match (row.state.as_str(), next) {
        // 启用：已安装（含激活失败恢复后）→ enabled
        (STATE_INSTALLED, STATE_ENABLED) => true,
        // 停用：enabled → installed
        (STATE_ENABLED, STATE_INSTALLED) => true,
        // 激活失败：enabled|installed → error
        (STATE_ENABLED, STATE_ERROR) | (STATE_INSTALLED, STATE_ERROR) => true,
        // 错误恢复：error → enabled（重试激活）/ error → installed（禁用止损）
        (STATE_ERROR, STATE_ENABLED) | (STATE_ERROR, STATE_INSTALLED) => true,
        // 同状态幂等（enable 幂等 / disable 幂等）
        (STATE_ENABLED, STATE_ENABLED) | (STATE_INSTALLED, STATE_INSTALLED) => true,
        _ => false,
    };
    if !legal {
        return Err(AppError::Validation(format!(
            "插件状态跃迁非法：{} → {next}",
            row.state
        )));
    }
    sqlx::query("UPDATE plugins SET state = ?, error_json = ?, updated_at = ? WHERE id = ?")
        .bind(next)
        .bind(error_json)
        .bind(now())
        .bind(id)
        .execute(db)
        .await?;
    Ok(())
}

/// 卸载（级联清理 plugin_data；日志保留供事后排查，可手动清理）。
pub async fn delete_plugin(db: &SqlitePool, id: &str) -> Result<()> {
    require_plugin(db, id).await?;
    sqlx::query("DELETE FROM plugins WHERE id = ?")
        .bind(id)
        .execute(db)
        .await?;
    Ok(())
}

/// 插件是否处于启用态（命令门禁热路径：单行索引查询）。
pub async fn is_plugin_enabled(db: &SqlitePool, id: &str) -> Result<bool> {
    let row: Option<(String,)> = sqlx::query_as("SELECT state FROM plugins WHERE id = ?")
        .bind(id)
        .fetch_optional(db)
        .await?;
    Ok(row.is_some_and(|(s,)| s == STATE_ENABLED))
}

/// 插件是否已安装（任意状态）。
pub async fn is_plugin_installed(db: &SqlitePool, id: &str) -> Result<bool> {
    let row: Option<(String,)> = sqlx::query_as("SELECT state FROM plugins WHERE id = ?")
        .bind(id)
        .fetch_optional(db)
        .await?;
    Ok(row.is_some())
}

/// 命令门禁：插件必须已安装且启用，否则返回带错误码语义的错误
/// （未安装 → NotFound；已装未启用 → Forbidden）。
pub async fn ensure_plugin_enabled(db: &SqlitePool, id: &str) -> Result<()> {
    let row: Option<(String,)> = sqlx::query_as("SELECT state FROM plugins WHERE id = ?")
        .bind(id)
        .fetch_optional(db)
        .await?;
    match row.map(|(s,)| s) {
        None => Err(AppError::NotFound(format!(
            "插件未安装：{id}（请在插件管理中安装后使用）"
        ))),
        Some(state) if state == STATE_ENABLED => Ok(()),
        Some(_) => Err(AppError::Forbidden(format!(
            "插件已停用：{id}（请在插件管理中启用）"
        ))),
    }
}

// ---------------------------------------------------------------------------
// 插件私有存储（plugin_data）
// ---------------------------------------------------------------------------

/// 插件存储键白名单（防越权键名 / 控制键注入）。
fn validate_data_key(key: &str) -> Result<()> {
    if key.is_empty() || key.len() > 128 {
        return Err(AppError::Validation("插件存储键长度必须在 1-128".into()));
    }
    if !key
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
    {
        return Err(AppError::Validation(format!(
            "插件存储键只允许字母 / 数字 / 下划线 / 连字符 / 点：{key}"
        )));
    }
    Ok(())
}

/// 读插件私有键。
pub async fn get_plugin_data(
    db: &SqlitePool,
    plugin_id: &str,
    key: &str,
) -> Result<Option<String>> {
    validate_data_key(key)?;
    require_plugin(db, plugin_id).await?;
    let row: Option<(String,)> =
        sqlx::query_as("SELECT value_json FROM plugin_data WHERE plugin_id = ? AND key = ?")
            .bind(plugin_id)
            .bind(key)
            .fetch_optional(db)
            .await?;
    Ok(row.map(|(v,)| v))
}

/// 写插件私有键（upsert；单值 ≤ 64 KiB）。
pub async fn set_plugin_data(
    db: &SqlitePool,
    plugin_id: &str,
    key: &str,
    value_json: &str,
) -> Result<()> {
    validate_data_key(key)?;
    require_plugin(db, plugin_id).await?;
    if value_json.len() > 64 * 1024 {
        return Err(AppError::Validation("插件存储单值超过 64 KiB".into()));
    }
    sqlx::query(
        "INSERT INTO plugin_data (plugin_id, key, value_json, updated_at) VALUES (?, ?, ?, ?) \
         ON CONFLICT(plugin_id, key) DO UPDATE SET value_json = excluded.value_json, \
         updated_at = excluded.updated_at",
    )
    .bind(plugin_id)
    .bind(key)
    .bind(value_json)
    .bind(now())
    .execute(db)
    .await?;
    Ok(())
}

/// 删插件私有键。
pub async fn delete_plugin_data(db: &SqlitePool, plugin_id: &str, key: &str) -> Result<()> {
    validate_data_key(key)?;
    require_plugin(db, plugin_id).await?;
    sqlx::query("DELETE FROM plugin_data WHERE plugin_id = ? AND key = ?")
        .bind(plugin_id)
        .bind(key)
        .execute(db)
        .await?;
    Ok(())
}

/// 列插件私有键（不含值）。
pub async fn list_plugin_keys(db: &SqlitePool, plugin_id: &str) -> Result<Vec<String>> {
    require_plugin(db, plugin_id).await?;
    let rows: Vec<(String,)> =
        sqlx::query_as("SELECT key FROM plugin_data WHERE plugin_id = ? ORDER BY key")
            .bind(plugin_id)
            .fetch_all(db)
            .await?;
    Ok(rows.into_iter().map(|(k,)| k).collect())
}

// ---------------------------------------------------------------------------
// 插件日志（plugin_logs）
// ---------------------------------------------------------------------------

/// 追加插件日志（audit / debug / info / warn / error），并滚动清理到上限。
pub async fn insert_plugin_log(
    db: &SqlitePool,
    plugin_id: &str,
    level: &str,
    message: &str,
    context_json: Option<&str>,
) -> Result<()> {
    if !matches!(level, "debug" | "info" | "warn" | "error" | "audit") {
        return Err(AppError::Validation(format!("非法日志级别：{level}")));
    }
    if message.len() > 8 * 1024 {
        return Err(AppError::Validation("日志单条超过 8 KiB".into()));
    }
    sqlx::query(
        "INSERT INTO plugin_logs (plugin_id, level, message, context_json, created_at) \
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(plugin_id)
    .bind(level)
    .bind(message)
    .bind(context_json)
    .bind(now())
    .execute(db)
    .await?;
    prune_plugin_logs(db, plugin_id).await?;
    Ok(())
}

/// 滚动保留每插件最近 [`PLUGIN_LOG_CAP`] 条。
pub async fn prune_plugin_logs(db: &SqlitePool, plugin_id: &str) -> Result<()> {
    sqlx::query(
        "DELETE FROM plugin_logs WHERE plugin_id = ? AND id NOT IN (\
         SELECT id FROM plugin_logs WHERE plugin_id = ? ORDER BY id DESC LIMIT ?)",
    )
    .bind(plugin_id)
    .bind(plugin_id)
    .bind(PLUGIN_LOG_CAP)
    .execute(db)
    .await?;
    Ok(())
}

/// 插件日志查询（新→旧；level 过滤可选）。
pub async fn list_plugin_logs(
    db: &SqlitePool,
    plugin_id: &str,
    limit: i64,
    level: Option<&str>,
) -> Result<Vec<PluginLogRow>> {
    require_plugin(db, plugin_id).await?;
    let limit = limit.clamp(1, 500);
    let rows: Vec<PluginLogRow> = match level {
        Some(lv) if matches!(lv, "debug" | "info" | "warn" | "error" | "audit") => {
            sqlx::query_as(
                "SELECT id, plugin_id, level, message, context_json, created_at \
                 FROM plugin_logs WHERE plugin_id = ? AND level = ? ORDER BY id DESC LIMIT ?",
            )
            .bind(plugin_id)
            .bind(lv)
            .bind(limit)
            .fetch_all(db)
            .await?
        }
        _ => {
            sqlx::query_as(
                "SELECT id, plugin_id, level, message, context_json, created_at \
                 FROM plugin_logs WHERE plugin_id = ? ORDER BY id DESC LIMIT ?",
            )
            .bind(plugin_id)
            .bind(limit)
            .fetch_all(db)
            .await?
        }
    };
    Ok(rows)
}

/// 日志行（IPC 透出）。
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct PluginLogRow {
    pub id: i64,
    pub plugin_id: String,
    pub level: String,
    pub message: String,
    pub context_json: Option<String>,
    pub created_at: String,
}

/// 清空插件日志。
pub async fn clear_plugin_logs(db: &SqlitePool, plugin_id: &str) -> Result<()> {
    require_plugin(db, plugin_id).await?;
    sqlx::query("DELETE FROM plugin_logs WHERE plugin_id = ?")
        .bind(plugin_id)
        .execute(db)
        .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// 版本历史 / 权限审计 / 全文件复核（0033）
// ---------------------------------------------------------------------------

/// 落库全文件清单复核结果（安装 / 启用 / 手动校验后调用）。
pub async fn set_files_verified(db: &SqlitePool, plugin_id: &str, ok: bool) -> Result<()> {
    sqlx::query("UPDATE plugins SET files_verified = ?, last_verified_at = ? WHERE id = ?")
        .bind(ok)
        .bind(now())
        .bind(plugin_id)
        .execute(db)
        .await?;
    Ok(())
}

/// 版本历史行（回滚选择列表）。
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PluginVersionRow {
    pub id: i64,
    pub plugin_id: String,
    pub version: String,
    pub manifest_json: String,
    pub integrity: Option<String>,
    pub signed: bool,
    pub created_at: String,
}

/// 插入版本历史快照（更新 / 升级前调用——回滚锚点）。
pub async fn insert_plugin_version(
    db: &SqlitePool,
    plugin_id: &str,
    manifest: &PluginManifest,
    integrity: Option<&str>,
    signed: bool,
) -> Result<()> {
    let manifest_json = serde_json::to_string(manifest)
        .map_err(|e| AppError::Validation(format!("manifest 序列化失败：{e}")))?;
    sqlx::query(
        "INSERT INTO plugin_versions (plugin_id, version, manifest_json, integrity, signed, created_at) \
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(plugin_id)
    .bind(&manifest.version)
    .bind(&manifest_json)
    .bind(integrity)
    .bind(signed)
    .bind(now())
    .execute(db)
    .await?;
    // 滚动保留 PLUGIN_VERSION_CAP 条（防无限增长）
    sqlx::query(
        "DELETE FROM plugin_versions WHERE plugin_id = ? AND id NOT IN \
         (SELECT id FROM plugin_versions WHERE plugin_id = ? ORDER BY id DESC LIMIT ?)",
    )
    .bind(plugin_id)
    .bind(plugin_id)
    .bind(PLUGIN_VERSION_CAP)
    .execute(db)
    .await?;
    Ok(())
}

/// 版本历史（新→旧；每插件滚动保留 [`PLUGIN_VERSION_CAP`] 条）。
pub async fn list_plugin_versions(
    db: &SqlitePool,
    plugin_id: &str,
) -> Result<Vec<PluginVersionRow>> {
    let rows: Vec<PluginVersionRow> = sqlx::query_as(
        "SELECT id, plugin_id, version, manifest_json, integrity, signed, created_at \
         FROM plugin_versions WHERE plugin_id = ? ORDER BY id DESC LIMIT ?",
    )
    .bind(plugin_id)
    .bind(PLUGIN_VERSION_CAP)
    .fetch_all(db)
    .await?;
    Ok(rows)
}

/// 删除插件全部版本历史（卸载清理）。
pub async fn delete_plugin_versions(db: &SqlitePool, plugin_id: &str) -> Result<()> {
    sqlx::query("DELETE FROM plugin_versions WHERE plugin_id = ?")
        .bind(plugin_id)
        .execute(db)
        .await?;
    Ok(())
}

/// 每插件版本历史保留上限（滚动，防止无限增长）。
pub const PLUGIN_VERSION_CAP: i64 = 10;

/// 权限审计行。
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PluginPermissionAuditRow {
    pub id: i64,
    pub plugin_id: String,
    pub action: String,
    pub permissions_json: String,
    pub created_at: String,
}

/// 权限授予 / 收回审计（管理中心的「权限管理」动作轨迹）。
pub async fn insert_permission_audit(
    db: &SqlitePool,
    plugin_id: &str,
    action: &str,
    permissions: &[String],
) -> Result<()> {
    let perms = serde_json::to_string(permissions)
        .map_err(|e| AppError::Validation(format!("权限序列化失败：{e}")))?;
    sqlx::query(
        "INSERT INTO plugin_permission_audit (plugin_id, action, permissions_json, created_at) \
         VALUES (?, ?, ?, ?)",
    )
    .bind(plugin_id)
    .bind(action)
    .bind(&perms)
    .bind(now())
    .execute(db)
    .await?;
    // 审计滚动保留 500 条（audit 表无外键、无既有清理——防止无限增长）
    sqlx::query(
        "DELETE FROM plugin_permission_audit WHERE plugin_id = ? AND id NOT IN (\
             SELECT id FROM plugin_permission_audit WHERE plugin_id = ?\
             ORDER BY id DESC LIMIT 500\
         )",
    )
    .bind(plugin_id)
    .bind(plugin_id)
    .execute(db)
    .await?;
    Ok(())
}

/// 权限审计（新→旧；上限 100 条）。
pub async fn list_permission_audit(
    db: &SqlitePool,
    plugin_id: &str,
) -> Result<Vec<PluginPermissionAuditRow>> {
    let rows: Vec<PluginPermissionAuditRow> = sqlx::query_as(
        "SELECT id, plugin_id, action, permissions_json, created_at \
         FROM plugin_permission_audit WHERE plugin_id = ? ORDER BY id DESC LIMIT 100",
    )
    .bind(plugin_id)
    .fetch_all(db)
    .await?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn test_pool() -> SqlitePool {
        let pool = sqlx::SqlitePool::connect(":memory:").await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    fn demo_manifest() -> PluginManifest {
        PluginManifest::parse(
            r#"{
              "id": "com.example.demo",
              "name": "Demo",
              "description": "示例",
              "version": "0.1.0",
              "authors": ["t"],
              "kind": "ui",
              "min_app_version": "0.0.60",
              "sdk_version": 1,
              "permissions": ["std.storage"]
            }"#
            .as_bytes(),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn plugin_version_history_rolls_and_deletes() {
        let db = test_pool().await;
        let manifest = demo_manifest();
        insert_plugin(
            &db,
            &manifest.id,
            &manifest,
            "local",
            None,
            false,
            &["std.storage".into()],
            None,
        )
        .await
        .unwrap();
        for i in 0..12 {
            let m = PluginManifest::parse(
                format!(
                    r#"{{"id":"com.example.demo","name":"Demo","description":"示例","version":"0.1.{i}",
                        "authors":["t"],"kind":"ui","min_app_version":"0.0.60","sdk_version":1,
                        "permissions":["std.storage"]}}"#
                )
                .as_bytes(),
            )
            .unwrap();
            insert_plugin_version(&db, &manifest.id, &m, None, false)
                .await
                .unwrap();
        }
        let rows = list_plugin_versions(&db, &manifest.id).await.unwrap();
        assert_eq!(rows.len() as i64, PLUGIN_VERSION_CAP);
        assert_eq!(rows[0].version, "0.1.11"); // 新→旧
                                               // 卸载清理
        delete_plugin_versions(&db, &manifest.id).await.unwrap();
        assert!(list_plugin_versions(&db, &manifest.id)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn lifecycle_transitions_are_enforced() {
        let db = test_pool().await;
        let m = demo_manifest();
        insert_plugin(
            &db,
            &m.id,
            &m,
            "local",
            Some("abc123"),
            false,
            &["std.storage".into()],
            None,
        )
        .await
        .unwrap();

        // installed → enabled → installed（禁用）→ enabled → error → enabled
        set_plugin_state(&db, &m.id, STATE_ENABLED, None)
            .await
            .unwrap();
        assert!(is_plugin_enabled(&db, &m.id).await.unwrap());
        set_plugin_state(&db, &m.id, STATE_INSTALLED, None)
            .await
            .unwrap();
        assert!(!is_plugin_enabled(&db, &m.id).await.unwrap());
        set_plugin_state(&db, &m.id, STATE_ENABLED, None)
            .await
            .unwrap();
        set_plugin_state(&db, &m.id, STATE_ERROR, Some(r#"{"message":"boom"}"#))
            .await
            .unwrap();
        assert!(!is_plugin_enabled(&db, &m.id).await.unwrap());
        set_plugin_state(&db, &m.id, STATE_ENABLED, None)
            .await
            .unwrap();

        // 非法跃迁：installed → error 之外的状态字符串直接拒绝在入口。
        assert!(set_plugin_state(&db, "com.example.demo", "activated", None)
            .await
            .is_err());

        // 卸载后 is_plugin_enabled = false 且 NotFound。
        delete_plugin(&db, &m.id).await.unwrap();
        assert!(get_plugin(&db, &m.id).await.unwrap().is_none());
        assert!(!is_plugin_enabled(&db, &m.id).await.unwrap());
    }

    #[tokio::test]
    async fn ensure_plugin_enabled_reports_distinct_errors() {
        let db = test_pool().await;
        let m = demo_manifest();
        // 未安装 → NotFound
        let err = ensure_plugin_enabled(&db, &m.id).await.unwrap_err();
        assert!(matches!(err, AppError::NotFound(_)), "{err}");
        insert_plugin(&db, &m.id, &m, "local", None, false, &[], None)
            .await
            .unwrap();
        // 已装未启用 → Forbidden
        let err = ensure_plugin_enabled(&db, &m.id).await.unwrap_err();
        assert!(matches!(err, AppError::Forbidden(_)), "{err}");
        set_plugin_state(&db, &m.id, STATE_ENABLED, None)
            .await
            .unwrap();
        ensure_plugin_enabled(&db, &m.id).await.unwrap();
    }

    #[tokio::test]
    async fn plugin_data_is_scoped_per_plugin() {
        let db = test_pool().await;
        let a = demo_manifest();
        let b_manifest = PluginManifest::parse(
            r#"{
              "id": "com.example.other",
              "name": "Other",
              "description": "另一插件",
              "version": "0.2.0",
              "authors": ["t"],
              "kind": "ui",
              "min_app_version": "0.0.60",
              "sdk_version": 1,
              "permissions": ["std.storage"]
            }"#
            .as_bytes(),
        )
        .unwrap();
        insert_plugin(&db, &a.id, &a, "local", None, false, &[], None)
            .await
            .unwrap();
        insert_plugin(
            &db,
            &b_manifest.id,
            &b_manifest,
            "local",
            None,
            false,
            &[],
            None,
        )
        .await
        .unwrap();

        set_plugin_data(&db, &a.id, "favorite", r#""x""#)
            .await
            .unwrap();
        // A 的键只在 A 的作用域可见；B 读写同名键互不影响。
        assert_eq!(
            get_plugin_data(&db, &a.id, "favorite")
                .await
                .unwrap()
                .as_deref(),
            Some(r#""x""#)
        );
        set_plugin_data(&db, &b_manifest.id, "favorite", r#""y""#)
            .await
            .unwrap();
        assert_eq!(
            get_plugin_data(&db, &a.id, "favorite")
                .await
                .unwrap()
                .as_deref(),
            Some(r#""x""#)
        );
        assert_eq!(
            list_plugin_keys(&db, &b_manifest.id).await.unwrap(),
            ["favorite"]
        );
        delete_plugin(&db, &a.id).await.unwrap();
        // 级联清理：A 的数据随卸载消失。
        assert!(get_plugin_data(&db, &a.id, "favorite").await.is_err());
    }

    #[tokio::test]
    async fn plugin_data_key_and_size_rules() {
        let db = test_pool().await;
        let m = demo_manifest();
        insert_plugin(&db, &m.id, &m, "local", None, false, &[], None)
            .await
            .unwrap();
        assert!(set_plugin_data(&db, &m.id, "../evil", "1").await.is_err());
        assert!(set_plugin_data(&db, &m.id, "", "1").await.is_err());
        let big = "x".repeat(65 * 1024);
        assert!(set_plugin_data(&db, &m.id, "k", &big).await.is_err());
    }

    #[tokio::test]
    async fn plugin_logs_roll_and_filter() {
        let db = test_pool().await;
        let m = demo_manifest();
        insert_plugin(&db, &m.id, &m, "local", None, false, &[], None)
            .await
            .unwrap();
        for i in 0..(PLUGIN_LOG_CAP + 50) {
            insert_plugin_log(&db, &m.id, "info", &format!("msg {i}"), None)
                .await
                .unwrap();
        }
        let logs = list_plugin_logs(&db, &m.id, 500, None).await.unwrap();
        assert_eq!(logs.len() as i64, PLUGIN_LOG_CAP);
        // 新→旧：最旧一条是第 50 条插入的内容。
        assert_eq!(logs.last().unwrap().message, "msg 50");

        insert_plugin_log(&db, &m.id, "audit", "invoke demo", None)
            .await
            .unwrap();
        let audits = list_plugin_logs(&db, &m.id, 100, Some("audit"))
            .await
            .unwrap();
        assert_eq!(audits.len(), 1);
        assert!(clear_plugin_logs(&db, &m.id).await.is_ok());
        assert!(list_plugin_logs(&db, &m.id, 100, None)
            .await
            .unwrap()
            .is_empty());
    }
}
