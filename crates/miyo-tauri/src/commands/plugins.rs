//! 插件系统 Command：安装 / 启停 / 卸载 / 更新 / 权限 / 日志 / 私有存储 / 完整性复核。
//!
//! 职责分层：
//! - 校验与安全策略在 `miyo-plugin`（清单严格校验 / 兼容性 / 包解包防护）；
//! - 持久化在 `miyo-storage::repository::plugins`（状态机 / 隔离存储 / 日志）；
//! - 本模块只做编排：读取入参 → 调领域层 → 组装 view → 审计落日志。
//!
//! 安全约定：
//! - 任何 install / enable 前都重跑 manifest 校验与兼容性检查（不信任库里旧值）；
//! - 权限快照 `granted_permissions` 只能是 `manifest.permissions` 的子集；
//! - 所有生命周期动作写 `audit` 级插件日志（卸载的日志保留，供事后排查）；
//! - 前端 SDK 的日志写入（plugin_record_log）与存储读写都以 plugin_id 为作用域，
//!   越权键名 / 非法级别在存储层直接拒绝。

use serde::Serialize;
use tauri::{AppHandle, State};

use miyo_plugin::manifest::PluginManifest;
use miyo_storage::repository as repo;
use miyo_storage::repository::{
    PluginLogRow, PluginRow, STATE_ENABLED, STATE_ERROR, STATE_INSTALLED,
};

use crate::error::{CommandError, CommandErrorCode, CommandResult};
use crate::state::AppState;

/// 本地插件包解包根目录：`{data_dir}/plugins/<插件 id>/`。
fn plugins_root() -> std::path::PathBuf {
    miyo_storage::db::data_dir().join("plugins")
}

/// 当前应用版本（tauri.conf.json 的 version，兼容性检查基准）。
fn app_version(app: &AppHandle) -> String {
    app.package_info().version.to_string()
}

// ---------------------------------------------------------------------------
// IPC view 类型（ts-rs 导出 frontend/src/types/generated/）
// ---------------------------------------------------------------------------

/// 激活失败信息。
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PluginErrorView {
    pub message: String,
    /// RFC3339 时间戳。
    pub at: String,
}

/// 已安装插件视图。
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PluginView {
    pub id: String,
    /// ui | ui-sidecar。
    pub kind: String,
    pub version: String,
    /// installed | enabled | error。
    pub state: String,
    /// builtin | local | market。
    pub source: String,
    pub manifest: PluginManifest,
    /// manifest 完整性摘要（sha256 hex）。
    pub integrity: Option<String>,
    /// 是否通过官方签名校验。
    pub signed: bool,
    /// 已批准权限（manifest.permissions 的子集）。
    pub granted_permissions: Vec<String>,
    /// 本地插件的安装目录（builtin 为 None）。
    pub install_path: Option<String>,
    /// 最近激活失败信息（state = error 时用于恢复）。
    pub error: Option<PluginErrorView>,
    pub installed_at: String,
    pub updated_at: String,
    /// 便捷标志（state == enabled）。
    pub enabled: bool,
}

/// 市场目录条目（内置目录 + 已安装合并）。
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PluginCatalogView {
    pub manifest: PluginManifest,
    /// builtin | market。
    pub source: String,
    /// 是否已安装。
    pub installed: bool,
    /// 已安装版本（可比较更新）。
    pub installed_version: Option<String>,
    /// 与当前 Miyo 版本是否兼容。
    pub compatible: bool,
}

/// 本地包检视结果（安装确认页；不落盘不注册）。
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PackageInspectionView {
    pub manifest: PluginManifest,
    /// manifest 字节 sha256（hex）。
    pub integrity: String,
    /// 是否通过官方签名校验。
    pub signed: bool,
    /// 来源种类：package（.miyo）| directory（目录直装，开发期）。
    pub origin: String,
}

fn row_to_view(row: PluginRow) -> CommandResult<PluginView> {
    let manifest = row
        .manifest()
        .map_err(|e| CommandError::with_code(CommandErrorCode::Internal, e.user_message()))?;
    let granted_permissions = row.granted_permissions();
    let error = row.error_json.and_then(|raw| {
        serde_json::from_str::<serde_json::Value>(&raw)
            .ok()
            .map(|v| PluginErrorView {
                message: v
                    .get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or_default()
                    .to_string(),
                at: v
                    .get("at")
                    .and_then(|a| a.as_str())
                    .unwrap_or_default()
                    .to_string(),
            })
    });
    Ok(PluginView {
        id: row.id,
        kind: row.kind,
        version: row.version,
        enabled: row.state == STATE_ENABLED,
        state: row.state,
        source: row.source,
        manifest,
        integrity: row.integrity,
        signed: row.signed,
        granted_permissions,
        install_path: row.install_path,
        error,
        installed_at: row.created_at,
        updated_at: row.updated_at,
    })
}

/// 审计日志（生命周期动作；插件未注册时仍尝试，失败仅记 tracing）。
async fn audit(db: &sqlx::SqlitePool, plugin_id: &str, message: &str) {
    if let Err(e) = repo::insert_plugin_log(db, plugin_id, "audit", message, None).await {
        tracing::warn!("插件审计日志写入失败（{plugin_id}）：{e}");
    }
}

// ---------------------------------------------------------------------------
// 查询
// ---------------------------------------------------------------------------

/// 列出全部已安装插件。
#[tauri::command(rename_all = "camelCase")]
pub async fn plugin_list_installed(state: State<'_, AppState>) -> CommandResult<Vec<PluginView>> {
    let rows = repo::list_plugins(&state.db).await?;
    rows.into_iter().map(row_to_view).collect()
}

/// 市场目录：内置插件目录（编译期嵌入）与已安装状态合并。
#[tauri::command(rename_all = "camelCase")]
pub async fn plugin_list_catalog(
    state: State<'_, AppState>,
    app: AppHandle,
) -> CommandResult<Vec<PluginCatalogView>> {
    let version = app_version(&app);
    let installed = repo::list_plugins(&state.db).await?;
    let mut out: Vec<PluginCatalogView> = Vec::new();
    for manifest in miyo_plugin::builtin_manifests() {
        let row = installed.iter().find(|r| r.id == manifest.id);
        out.push(PluginCatalogView {
            manifest: manifest.clone(),
            source: "builtin".into(),
            installed: row.is_some(),
            installed_version: row.map(|r| r.version.clone()),
            compatible: miyo_plugin::is_compatible(&manifest.min_app_version, &version),
        });
    }
    Ok(out)
}

/// 单个已安装插件详情。
#[tauri::command(rename_all = "camelCase")]
pub async fn plugin_get(state: State<'_, AppState>, id: String) -> CommandResult<PluginView> {
    let row = repo::require_plugin(&state.db, &id).await?;
    row_to_view(row)
}

// ---------------------------------------------------------------------------
// 安装 / 更新 / 卸载
// ---------------------------------------------------------------------------

/// 检视本地插件包（.miyo 或目录）：解析清单 + 完整性 + 签名，不落盘不注册。
#[tauri::command(rename_all = "camelCase")]
pub async fn plugin_inspect_package(path: String) -> CommandResult<PackageInspectionView> {
    let p = std::path::PathBuf::from(&path);
    let (info, origin) = if p.is_dir() {
        (
            miyo_plugin::package::inspect_package_dir(&p).map_err(command_err)?,
            "directory",
        )
    } else {
        let bytes = std::fs::read(&p)
            .map_err(|e| CommandError::with_code(CommandErrorCode::Io, e.to_string()))?;
        (
            miyo_plugin::package::inspect_package_bytes(&bytes).map_err(command_err)?,
            "package",
        )
    };
    // 包内 manifest 自带的 integrity 声明必须与实际字节摘要一致（发布流程锚定），
    // 不一致即视为篡改。
    if let Some(claimed) = &info.manifest.integrity {
        if !claimed.eq_ignore_ascii_case(&info.integrity) {
            return Err(CommandError::validation(
                "manifest 内声明的完整性摘要与实际内容不符，插件包可能已被篡改",
            ));
        }
    }
    Ok(PackageInspectionView {
        manifest: info.manifest,
        integrity: info.integrity,
        signed: info.signed,
        origin: origin.into(),
    })
}

/// 安装本地插件包（.miyo 文件或目录）。
///
/// `trust_unsigned`：未签名包必须由用户显式信任（安装确认页勾选）。
#[tauri::command(rename_all = "camelCase")]
pub async fn plugin_install_from_path(
    state: State<'_, AppState>,
    app: AppHandle,
    path: String,
    trust_unsigned: bool,
) -> CommandResult<PluginView> {
    let p = std::path::PathBuf::from(&path);
    let (info, install_path) = install_from_source(&state, &p, trust_unsigned).await?;
    let version = app_version(&app);
    miyo_plugin::ensure_compatible(&info.manifest.id, &info.manifest.min_app_version, &version)
        .map_err(command_err)?;
    miyo_plugin::ensure_sdk_supported(
        info.manifest.sdk_version,
        miyo_plugin::SUPPORTED_SDK_VERSION,
    )
    .map_err(command_err)?;
    if repo::is_plugin_installed(&state.db, &info.manifest.id).await? {
        return Err(CommandError::validation(format!(
            "插件 {} 已安装（{}），如需升级请使用「更新」",
            info.manifest.id,
            repo::require_plugin(&state.db, &info.manifest.id)
                .await?
                .version
        )));
    }
    // 安装即授予 manifest 声明的全部权限（安装确认页已展示；后续可在详情页收权）。
    let granted = info.manifest.permissions.clone();
    repo::insert_plugin(
        &state.db,
        &info.manifest.id,
        &info.manifest,
        "local",
        Some(&info.integrity),
        info.signed,
        &granted,
        install_path.as_deref(),
    )
    .await?;
    audit(
        &state.db,
        &info.manifest.id,
        &format!(
            "install local v{} integrity={} signed={}",
            info.manifest.version, info.integrity, info.signed
        ),
    )
    .await;
    let row = repo::require_plugin(&state.db, &info.manifest.id).await?;
    row_to_view(row)
}

/// 安装内置插件（随应用打包，无需签名确认）。
#[tauri::command(rename_all = "camelCase")]
pub async fn plugin_install_builtin(
    state: State<'_, AppState>,
    app: AppHandle,
    id: String,
) -> CommandResult<PluginView> {
    let manifest = miyo_plugin::builtin_manifest(&id).ok_or_else(|| {
        CommandError::with_code(CommandErrorCode::NotFound, format!("内置插件不存在：{id}"))
    })?;
    let version = app_version(&app);
    miyo_plugin::ensure_compatible(&manifest.id, &manifest.min_app_version, &version)
        .map_err(command_err)?;
    if repo::is_plugin_installed(&state.db, &id).await? {
        return Err(CommandError::validation(format!("插件 {id} 已安装")));
    }
    let manifest = manifest.clone();
    let granted = manifest.permissions.clone();
    repo::insert_plugin(
        &state.db,
        &manifest.id,
        &manifest,
        "builtin",
        None,
        false,
        &granted,
        None,
    )
    .await?;
    audit(
        &state.db,
        &manifest.id,
        &format!("install builtin v{}", manifest.version),
    )
    .await;
    let row = repo::require_plugin(&state.db, &manifest.id).await?;
    row_to_view(row)
}

/// 更新内置插件到随应用打包的新版本（manifest / 完整性替换，状态保留）。
#[tauri::command(rename_all = "camelCase")]
pub async fn plugin_update_builtin(
    state: State<'_, AppState>,
    app: AppHandle,
    id: String,
) -> CommandResult<PluginView> {
    let row = repo::require_plugin(&state.db, &id).await?;
    if row.source != "builtin" {
        return Err(CommandError::validation("仅内置插件支持此更新方式"));
    }
    let manifest = miyo_plugin::builtin_manifest(&id)
        .ok_or_else(|| {
            CommandError::with_code(CommandErrorCode::NotFound, format!("内置插件不存在：{id}"))
        })?
        .clone();
    let version = app_version(&app);
    miyo_plugin::ensure_compatible(&manifest.id, &manifest.min_app_version, &version)
        .map_err(command_err)?;
    // 权限快照保持原有批准集与新清单的交集（新权限需用户重新批准：这里先收敛，
    // 管理中心检测到未批准权限时会提示重新授权）。
    let old_granted = row.granted_permissions();
    let granted: Vec<String> = manifest
        .permissions
        .iter()
        .filter(|p| old_granted.contains(p))
        .cloned()
        .collect();
    repo::update_plugin(&state.db, &id, &manifest, None, false, &granted).await?;
    audit(
        &state.db,
        &id,
        &format!("update builtin → v{}", manifest.version),
    )
    .await;
    let row = repo::require_plugin(&state.db, &id).await?;
    row_to_view(row)
}

/// 卸载插件（级联清理私有存储；本地包目录一并删除）。
#[tauri::command(rename_all = "camelCase")]
pub async fn plugin_uninstall(
    state: State<'_, AppState>,
    app: AppHandle,
    id: String,
) -> CommandResult<()> {
    let row = repo::require_plugin(&state.db, &id).await?;
    // 先停用（运行时回收由前端 deactivate 完成；后端先落状态防窗口期调用）。
    if row.state == STATE_ENABLED {
        repo::set_plugin_state(&state.db, &id, STATE_INSTALLED, None).await?;
    }
    audit(&state.db, &id, &format!("uninstall v{}", row.version)).await;
    // 删除本地包目录：仅允许删除受管目录内的内容（install_path 可能指向开发目录）。
    if let Some(path) = &row.install_path {
        let root = plugins_root();
        if let Ok(p) = std::path::PathBuf::from(path).canonicalize() {
            if p.starts_with(&root) {
                let _ = std::fs::remove_dir_all(p);
            }
        }
    }
    repo::delete_plugin(&state.db, &id).await?;
    let _ = app; // 预留：卸载后 emit 事件
    Ok(())
}

/// 启用 / 停用（启用前重跑兼容性检查；停用即收回运行时入口）。
#[tauri::command(rename_all = "camelCase")]
pub async fn plugin_set_enabled(
    state: State<'_, AppState>,
    app: AppHandle,
    id: String,
    enabled: bool,
) -> CommandResult<PluginView> {
    let row = repo::require_plugin(&state.db, &id).await?;
    if enabled {
        let manifest = row.manifest().map_err(command_err)?;
        let version = app_version(&app);
        miyo_plugin::ensure_compatible(&manifest.id, &manifest.min_app_version, &version).map_err(
            |e| CommandError::with_code(CommandErrorCode::PluginIncompatible, e.user_message()),
        )?;
        miyo_plugin::ensure_sdk_supported(manifest.sdk_version, miyo_plugin::SUPPORTED_SDK_VERSION)
            .map_err(|e| {
                CommandError::with_code(CommandErrorCode::PluginIncompatible, e.user_message())
            })?;
        repo::set_plugin_state(&state.db, &id, STATE_ENABLED, None).await?;
        audit(&state.db, &id, &format!("enable v{}", row.version)).await;
    } else {
        repo::set_plugin_state(&state.db, &id, STATE_INSTALLED, None).await?;
        audit(&state.db, &id, "disable").await;
    }
    let row = repo::require_plugin(&state.db, &id).await?;
    row_to_view(row)
}

/// 调整权限快照（granted ⊆ manifest.permissions；收权后 SDK 调用即被拒）。
#[tauri::command(rename_all = "camelCase")]
pub async fn plugin_set_permissions(
    state: State<'_, AppState>,
    id: String,
    granted_permissions: Vec<String>,
) -> CommandResult<PluginView> {
    let row = repo::require_plugin(&state.db, &id).await?;
    let manifest = row.manifest().map_err(command_err)?;
    for g in &granted_permissions {
        if !manifest.permissions.contains(g) {
            return Err(CommandError::validation(format!(
                "权限 {g} 不在插件声明清单内，无法授予"
            )));
        }
    }
    miyo_plugin::validate_permissions(&granted_permissions).map_err(command_err)?;
    repo::update_plugin(
        &state.db,
        &id,
        &manifest,
        row.integrity.as_deref(),
        row.signed,
        &granted_permissions,
    )
    .await?;
    audit(
        &state.db,
        &id,
        &format!("permissions → [{}]", granted_permissions.join(", ")),
    )
    .await;
    let row = repo::require_plugin(&state.db, &id).await?;
    row_to_view(row)
}

/// 上报激活失败（前端 activate 抛错时调用；保留现场供恢复）。
#[tauri::command(rename_all = "camelCase")]
pub async fn plugin_report_error(
    state: State<'_, AppState>,
    id: String,
    message: String,
) -> CommandResult<()> {
    let error_json = serde_json::json!({
        "message": message,
        "at": chrono::Utc::now().to_rfc3339(),
    })
    .to_string();
    repo::set_plugin_state(&state.db, &id, STATE_ERROR, Some(&error_json)).await?;
    Ok(())
}

/// 完整性复核：重算安装的 manifest 摘要与落库值比对（本地包被篡改即失败）。
#[tauri::command(rename_all = "camelCase")]
pub async fn plugin_verify_integrity(
    state: State<'_, AppState>,
    id: String,
) -> CommandResult<bool> {
    let row = repo::require_plugin(&state.db, &id).await?;
    let Some(expected) = &row.integrity else {
        // builtin / 未记录完整性的插件：无可比对值，视为通过（清单以 DB 内为准）。
        return Ok(true);
    };
    let Some(path) = &row.install_path else {
        return Ok(true);
    };
    let bytes = std::fs::read(std::path::PathBuf::from(path).join("manifest.json"))
        .map_err(|e| CommandError::with_code(CommandErrorCode::Io, e.to_string()))?;
    miyo_plugin::package::verify_manifest_integrity(&bytes, expected).map_err(command_err)?;
    Ok(true)
}

// ---------------------------------------------------------------------------
// 插件日志 / 私有存储（SDK 后端）
// ---------------------------------------------------------------------------

/// 查询插件日志（新→旧；level 过滤可选）。
#[tauri::command(rename_all = "camelCase")]
pub async fn plugin_logs(
    state: State<'_, AppState>,
    id: String,
    limit: Option<i64>,
    level: Option<String>,
) -> CommandResult<Vec<PluginLogRow>> {
    Ok(repo::list_plugin_logs(&state.db, &id, limit.unwrap_or(200), level.as_deref()).await?)
}

/// 清空插件日志。
#[tauri::command(rename_all = "camelCase")]
pub async fn plugin_clear_logs(state: State<'_, AppState>, id: String) -> CommandResult<()> {
    Ok(repo::clear_plugin_logs(&state.db, &id).await?)
}

/// SDK logger 落库入口（debug/info/warn/error/audit；级别非法在存储层拒绝）。
#[tauri::command(rename_all = "camelCase")]
pub async fn plugin_record_log(
    state: State<'_, AppState>,
    id: String,
    level: String,
    message: String,
    context_json: Option<String>,
) -> CommandResult<()> {
    repo::insert_plugin_log(&state.db, &id, &level, &message, context_json.as_deref()).await?;
    Ok(())
}

/// 读插件私有键（值为 JSON）。
#[tauri::command(rename_all = "camelCase")]
pub async fn plugin_data_get(
    state: State<'_, AppState>,
    id: String,
    key: String,
) -> CommandResult<Option<serde_json::Value>> {
    match repo::get_plugin_data(&state.db, &id, &key).await? {
        None => Ok(None),
        Some(raw) => serde_json::from_str(&raw)
            .map(Some)
            .map_err(|e| CommandError::with_code(CommandErrorCode::Internal, e.to_string())),
    }
}

/// 写插件私有键（upsert；单值 ≤ 64 KiB）。
#[tauri::command(rename_all = "camelCase")]
pub async fn plugin_data_set(
    state: State<'_, AppState>,
    id: String,
    key: String,
    value: serde_json::Value,
) -> CommandResult<()> {
    let raw = serde_json::to_string(&value)
        .map_err(|e| CommandError::with_code(CommandErrorCode::Internal, e.to_string()))?;
    repo::set_plugin_data(&state.db, &id, &key, &raw).await?;
    Ok(())
}

/// 删插件私有键。
#[tauri::command(rename_all = "camelCase")]
pub async fn plugin_data_delete(
    state: State<'_, AppState>,
    id: String,
    key: String,
) -> CommandResult<()> {
    Ok(repo::delete_plugin_data(&state.db, &id, &key).await?)
}

/// 列插件私有键。
#[tauri::command(rename_all = "camelCase")]
pub async fn plugin_data_keys(
    state: State<'_, AppState>,
    id: String,
) -> CommandResult<Vec<String>> {
    Ok(repo::list_plugin_keys(&state.db, &id).await?)
}

// ---------------------------------------------------------------------------
// 内部工具
// ---------------------------------------------------------------------------

/// 从 .miyo 文件或目录安装到受管目录：解包 / 校验 / 完整性。
/// 返回（检视结果, install_path）。
async fn install_from_source(
    state: &State<'_, AppState>,
    source: &std::path::Path,
    trust_unsigned: bool,
) -> CommandResult<(miyo_plugin::package::PackageInfo, Option<String>)> {
    let _ = &state.db;
    if source.is_dir() {
        // 目录直装（开发期）：记录路径本身，不复制。
        let info = miyo_plugin::package::inspect_package_dir(source).map_err(command_err)?;
        if !info.signed && !trust_unsigned {
            return Err(unsigned_error(&info));
        }
        return Ok((info, Some(source.to_string_lossy().to_string())));
    }
    let bytes = std::fs::read(source)
        .map_err(|e| CommandError::with_code(CommandErrorCode::Io, e.to_string()))?;
    let info = miyo_plugin::package::inspect_package_bytes(&bytes).map_err(command_err)?;
    if !info.signed && !trust_unsigned {
        return Err(unsigned_error(&info));
    }
    let dest = plugins_root().join(&info.manifest.id);
    // 重装（卸载残留 / 版本替换）：先清空目标目录。
    if dest.exists() {
        std::fs::remove_dir_all(&dest)
            .map_err(|e| CommandError::with_code(CommandErrorCode::Io, e.to_string()))?;
    }
    miyo_plugin::package::extract_package_to_dir(&bytes, &dest).map_err(command_err)?;
    Ok((info, Some(dest.to_string_lossy().to_string())))
}

fn unsigned_error(info: &miyo_plugin::package::PackageInfo) -> CommandError {
    CommandError::with_code(
        CommandErrorCode::Forbidden,
        format!(
            "插件 {} 未签名（开发包）。确认来源可信后勾选「信任此未签名插件」再安装",
            info.manifest.id
        ),
    )
}

/// AppError → CommandError（统一走 From 映射；包一层以携带上下文）。
fn command_err(e: miyo_core::AppError) -> CommandError {
    CommandError::from(e)
}

#[allow(dead_code)]
fn state_name(s: &str) -> &'static str {
    match s {
        STATE_INSTALLED => "installed",
        STATE_ENABLED => "enabled",
        STATE_ERROR => "error",
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// plugins_root 必须位于数据目录下（受管子目录）。
    #[test]
    fn plugins_root_is_under_data_dir() {
        let root = plugins_root();
        assert!(root.starts_with(miyo_storage::db::data_dir()));
        assert!(root.ends_with("plugins"));
    }

    /// 状态名映射只认三态。
    #[test]
    fn state_name_mapping() {
        assert_eq!(state_name("installed"), "installed");
        assert_eq!(state_name("enabled"), "enabled");
        assert_eq!(state_name("error"), "error");
        assert_eq!(state_name("other"), "unknown");
    }
}
