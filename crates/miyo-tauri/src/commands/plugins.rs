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

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

use miyo_plugin::manifest::PluginManifest;
use miyo_storage::repository as repo;
use miyo_storage::repository::{
    PluginLogRow, PluginRow, STATE_ENABLED, STATE_ERROR, STATE_INSTALLED,
};

use crate::error::{CommandError, CommandErrorCode, CommandResult};
use crate::state::AppState;

/// 会话校验失败 → 统一 FORBIDDEN（不区分未知 / 过期 / 不匹配，防探测）。
fn session_err() -> CommandError {
    CommandError::with_code(
        CommandErrorCode::Forbidden,
        "缺少或无效的插件会话凭证（sessionToken）：请重新激活插件获取会话",
    )
}

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
    /// 全文件清单最近一次复核是否通过（manifest 2.0 files；无清单恒 false）。
    pub files_verified: bool,
    /// 最近一次全文件复核时间（RFC3339；NULL = 从未复核）。
    pub last_verified_at: Option<String>,
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
    /// 签名信任级：official（官方 keyring）| foreign（自签 / 未命中）| unsigned。
    pub trust: String,
    /// 全文件清单校验（检视阶段流式预验）：None = 无 files 声明（v1 包）。
    pub files_verified: Option<bool>,
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
        id: row.id.clone(),
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
        files_verified: row.files_verified,
        last_verified_at: row.last_verified_at,
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
        trust: match info.trust {
            miyo_plugin::package::TrustLevel::Official => "official".into(),
            miyo_plugin::package::TrustLevel::Foreign => "foreign".into(),
            miyo_plugin::package::TrustLevel::Unsigned => "unsigned".into(),
        },
        files_verified: info.files_verified,
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
    let version = app_version(&app);
    install_impl(
        &state.db,
        &state.plugin_sessions,
        std::path::Path::new(&path),
        trust_unsigned,
        &version,
    )
    .await
}

/// 安装核心（State 解耦——可测）。
pub(crate) async fn install_impl(
    db: &sqlx::SqlitePool,
    _sessions: &crate::sessions::PluginSessions,
    source: &std::path::Path,
    trust_unsigned: bool,
    app_version: &str,
) -> CommandResult<PluginView> {
    let (info, install_path) = install_from_source(db, source, trust_unsigned).await?;
    let version = app_version;
    miyo_plugin::ensure_compatible(&info.manifest.id, &info.manifest.min_app_version, version)
        .map_err(command_err)?;
    miyo_plugin::ensure_sdk_supported(
        info.manifest.sdk_version,
        miyo_plugin::SUPPORTED_SDK_VERSION,
    )
    .map_err(command_err)?;
    if repo::is_plugin_installed(db, &info.manifest.id).await? {
        return Err(CommandError::validation(format!(
            "插件 {} 已安装（{}），如需升级请使用「更新」",
            info.manifest.id,
            repo::require_plugin(db, &info.manifest.id).await?.version
        )));
    }
    // 依赖满足性检查（缺失 / 版本过低 / 不兼容——允许先装主插件再装依赖，
    // 但声明了 plugin_deps 的插件安装时依赖必须已安装）
    check_dependencies(db, &info.manifest, version).await?;
    // 安装即授予 manifest 声明的全部权限（安装确认页已展示；后续可在详情页收权）。
    let granted = info.manifest.permissions.clone();
    repo::insert_plugin(
        db,
        &info.manifest.id,
        &info.manifest,
        "local",
        Some(&info.integrity),
        info.signed,
        &granted,
        install_path.as_deref(),
    )
    .await?;
    // 全文件清单复核结果落库（extract 已完成校验：Some(true) 才会走到这里）
    if info.files_verified.is_some() {
        repo::set_files_verified(db, &info.manifest.id, true).await?;
    }
    audit(
        db,
        &info.manifest.id,
        &format!(
            "install local v{} integrity={} signed={}",
            info.manifest.version, info.integrity, info.signed
        ),
    )
    .await;
    let row = repo::require_plugin(db, &info.manifest.id).await?;
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
    let old_manifest_for_history = row
        .manifest()
        .map_err(|e| CommandError::with_code(CommandErrorCode::Internal, e.user_message()))?;
    let granted: Vec<String> = manifest
        .permissions
        .iter()
        .filter(|p| old_granted.contains(p))
        .cloned()
        .collect();
    // 版本历史快照（回滚锚点；每插件滚动保留 10 条）
    let _ = repo::insert_plugin_version(
        &state.db,
        &id,
        &old_manifest_for_history,
        row.integrity.as_deref(),
        row.signed,
    )
    .await;
    repo::update_plugin(&state.db, &id, &manifest, None, false, &granted).await?;
    state.plugin_sessions.revoke_plugin(&id);
    audit(
        &state.db,
        &id,
        &format!("update builtin → v{}", manifest.version),
    )
    .await;
    let row = repo::require_plugin(&state.db, &id).await?;
    row_to_view(row)
}

/// 更新本地插件包（备份 → staging 校验 → 原子换入 → 失败回滚）。
///
/// 流程：检视（签名/信任级同安装）→ 同 id 校验 → 版本相同拒绝 → 兼容性 /
/// SDK / 依赖满足性检查 → 旧清单落 `plugin_versions` 快照 → staging 解包 +
/// 全文件清单校验 → 旧目录改名 `.rollback-<id>` 备份 → staging 原子换入 →
/// DB 更新（granted 收敛为旧批准 ∩ 新声明）→ 吊销会话 → 审计。
/// 换入后任何失败：备份目录原样回位（更新失败回滚）。
#[tauri::command(rename_all = "camelCase")]
pub async fn plugin_update_from_path(
    state: State<'_, AppState>,
    app: AppHandle,
    path: String,
    trust_unsigned: bool,
) -> CommandResult<PluginView> {
    let version = app_version(&app);
    update_impl(
        &state.db,
        &state.plugin_sessions,
        std::path::Path::new(&path),
        trust_unsigned,
        &version,
    )
    .await
}

/// 更新核心（State 解耦——可测）。
pub(crate) async fn update_impl(
    db: &sqlx::SqlitePool,
    sessions: &crate::sessions::PluginSessions,
    source: &std::path::Path,
    trust_unsigned: bool,
    app_version: &str,
) -> CommandResult<PluginView> {
    let p = source.to_path_buf();
    if p.is_dir() {
        return Err(CommandError::validation(
            "更新仅支持 .miyo 包（目录直装属开发形态，请重新安装）",
        ));
    }
    const MAX_PACKAGE_BYTES: u64 = 64 * 1024 * 1024;
    let meta = std::fs::metadata(&p)
        .map_err(|e| CommandError::with_code(CommandErrorCode::Io, e.to_string()))?;
    if meta.len() > MAX_PACKAGE_BYTES {
        return Err(CommandError::validation("插件包超过 64 MiB 上限"));
    }
    let bytes = std::fs::read(&p)
        .map_err(|e| CommandError::with_code(CommandErrorCode::Io, e.to_string()))?;
    let info = miyo_plugin::package::inspect_package_bytes(&bytes).map_err(command_err)?;
    if !info.signed && !trust_unsigned {
        return Err(unsigned_error(&info));
    }
    // 检视期自声明 integrity 比对（与安装同口径）
    if let Some(claimed) = &info.manifest.integrity {
        if !claimed.eq_ignore_ascii_case(&info.integrity) {
            return Err(CommandError::validation(
                "manifest 内声明的完整性摘要与实际内容不符，插件包可能已被篡改",
            ));
        }
    }

    let row = repo::require_plugin(db, &info.manifest.id).await?;
    if row.source == "builtin" {
        return Err(CommandError::validation(
            "内置插件请使用「更新」（plugin_update_builtin）",
        ));
    }
    if info.manifest.version == row.version {
        return Err(CommandError::validation(format!(
            "新版本与当前版本相同（{}）",
            row.version
        )));
    }
    // 兼容性 / SDK / 依赖满足性（依赖按已安装清单逐项检查）
    let version = app_version;
    miyo_plugin::ensure_compatible(&info.manifest.id, &info.manifest.min_app_version, version)
        .map_err(command_err)?;
    miyo_plugin::ensure_sdk_supported(
        info.manifest.sdk_version,
        miyo_plugin::SUPPORTED_SDK_VERSION,
    )
    .map_err(command_err)?;
    check_dependencies(db, &info.manifest, version).await?;

    // 旧版本快照（回滚锚点）
    let old_manifest = row
        .manifest()
        .map_err(|e| CommandError::with_code(CommandErrorCode::Internal, e.user_message()))?;
    repo::insert_plugin_version(
        db,
        &row.id,
        &old_manifest,
        row.integrity.as_deref(),
        row.signed,
    )
    .await?;

    let root = plugins_root();
    let dest = root.join(&row.id);
    let staging = root.join(format!(".staging-{}", uuid::Uuid::new_v4().simple()));
    let backup = root.join(format!(".rollback-{}", row.id));

    // staging 解包 + 全文件清单校验（失败即清理，不动现有安装）
    let extracted =
        miyo_plugin::package::extract_package_to_dir(&bytes, &staging).map_err(|e| {
            let _ = std::fs::remove_dir_all(&staging);
            command_err(e)
        })?;

    // 原子换入：旧目录 → 备份；staging → dest；任何一步失败整体回位
    let swap = || -> CommandResult<()> {
        if dest.exists() {
            std::fs::rename(&dest, &backup)
                .map_err(|e| CommandError::with_code(CommandErrorCode::Io, e.to_string()))?;
        }
        if let Err(e) = std::fs::rename(&staging, &dest) {
            if backup.exists() {
                let _ = std::fs::rename(&backup, &dest);
            }
            return Err(CommandError::with_code(CommandErrorCode::Io, e.to_string()));
        }
        Ok(())
    };
    if let Err(e) = swap() {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(e);
    }

    // DB 更新（granted 收敛为旧批准 ∩ 新声明——新权限需重新批准）
    let old_granted = row.granted_permissions();
    let granted: Vec<String> = info
        .manifest
        .permissions
        .iter()
        .filter(|p| old_granted.contains(*p))
        .cloned()
        .collect();
    let granted_changed = granted != old_granted;
    let db_result = async {
        repo::update_plugin(
            db,
            &row.id,
            &info.manifest,
            Some(&info.integrity),
            info.signed,
            &granted,
        )
        .await?;
        if info.files_verified.is_some() {
            repo::set_files_verified(db, &row.id, true).await?;
        }
        Ok::<(), CommandError>(())
    }
    .await;
    if let Err(e) = db_result {
        // DB 失败：文件层回位（更新失败回滚）
        let _ = std::fs::remove_dir_all(&dest);
        if backup.exists() {
            let _ = std::fs::rename(&backup, &dest);
        }
        return Err(e);
    }
    // 成功：清理 staging 残留与旧备份之外的过期 staging
    let _ = std::fs::remove_dir_all(&staging);
    cleanup_stale_staging(&root);
    // 权限快照变化 / 版本变化 → 会话吊销（撤销即时生效）
    if granted_changed || info.manifest.version != row.version {
        sessions.revoke_plugin(&row.id);
    }
    audit(
        db,
        &row.id,
        &format!(
            "update local {} → v{} signed={} files_ok={}",
            row.version,
            info.manifest.version,
            info.signed,
            extracted
                .files_verified
                .map(|v| v.to_string())
                .unwrap_or_else(|| "n/a".into())
        ),
    )
    .await;
    let row = repo::require_plugin(db, &row.id).await?;
    row_to_view(row)
}

/// 回滚到上一版本（plugin_versions 最新快照 + 备份目录原样换回）。
#[tauri::command(rename_all = "camelCase")]
pub async fn plugin_rollback(state: State<'_, AppState>, id: String) -> CommandResult<PluginView> {
    rollback_impl(&state.db, &state.plugin_sessions, &id).await
}

/// 回滚核心（State 解耦——可测）。
pub(crate) async fn rollback_impl(
    db: &sqlx::SqlitePool,
    sessions: &crate::sessions::PluginSessions,
    id: &str,
) -> CommandResult<PluginView> {
    let row = repo::require_plugin(db, id).await?;
    if row.source == "builtin" {
        return Err(CommandError::validation(
            "内置插件回滚：请重新安装上一随包版本（builtin 无文件备份）",
        ));
    }
    let versions = repo::list_plugin_versions(db, id).await?;
    let snapshot = versions.first().ok_or_else(|| {
        CommandError::with_code(CommandErrorCode::NotFound, "无版本历史（未曾更新过）")
    })?;
    let snapshot_manifest = miyo_plugin::PluginManifest::parse(snapshot.manifest_json.as_bytes())
        .map_err(|e| {
        CommandError::with_code(CommandErrorCode::Internal, format!("版本快照损坏：{e}"))
    })?;
    let root = plugins_root();
    let dest = root.join(id);
    let backup = root.join(format!(".rollback-{id}"));
    if !backup.is_dir() {
        return Err(CommandError::with_code(
            CommandErrorCode::NotFound,
            "无可用回滚备份目录（更新流程未产生备份）",
        ));
    }
    // 原子换回：当前 → tmp；备份 → dest；tmp 删除
    let tmp = root.join(format!(".staging-{}", uuid::Uuid::new_v4().simple()));
    if dest.exists() {
        std::fs::rename(&dest, &tmp)
            .map_err(|e| CommandError::with_code(CommandErrorCode::Io, e.to_string()))?;
    }
    if let Err(e) = std::fs::rename(&backup, &dest) {
        if tmp.exists() {
            let _ = std::fs::rename(&tmp, &dest);
        }
        return Err(CommandError::with_code(CommandErrorCode::Io, e.to_string()));
    }
    let _ = std::fs::remove_dir_all(&tmp);
    // DB 回写快照（granted 收敛为当前批准 ∩ 快照声明）
    let old_granted = row.granted_permissions();
    let granted: Vec<String> = snapshot_manifest
        .permissions
        .iter()
        .filter(|p| old_granted.contains(*p))
        .cloned()
        .collect();
    repo::update_plugin(
        db,
        id,
        &snapshot_manifest,
        snapshot.integrity.as_deref(),
        snapshot.signed,
        &granted,
    )
    .await?;
    match &snapshot_manifest.files {
        Some(files) => {
            let files_ok = miyo_plugin::package::verify_dir_files(&dest, files).is_ok();
            repo::set_files_verified(db, id, files_ok).await?;
        }
        None => repo::set_files_verified(db, id, false).await?,
    }
    // 已消费该快照（防重复回滚到同一版本）；会话吊销（版本已回退）
    let _ = repo::delete_plugin_versions(db, id).await;
    sessions.revoke_plugin(id);
    audit(
        db,
        id,
        &format!("rollback {} → v{}", row.version, snapshot_manifest.version),
    )
    .await;
    let row = repo::require_plugin(db, id).await?;
    row_to_view(row)
}

/// 依赖满足性检查（安装 / 更新 / 启用共用）：缺失 / 版本过低 / 不兼容。
/// 依赖「未启用」不在安装期拒绝（允许先装主插件再装依赖），启用期强校验。
async fn check_dependencies(
    db: &sqlx::SqlitePool,
    manifest: &PluginManifest,
    app_version: &str,
) -> CommandResult<()> {
    if manifest.plugin_deps.as_ref().is_none_or(|d| d.is_empty()) {
        return Ok(());
    }
    let all = repo::list_plugins(db).await?;
    let lookup = |id: &str| {
        all.iter().find(|r| r.id == id).and_then(|r| {
            let m = r.manifest().ok()?;
            Some(miyo_plugin::deps::DepInfo {
                manifest: m,
                installed: true,
                enabled: r.state == STATE_ENABLED,
            })
        })
    };
    miyo_plugin::deps::check_enable(manifest, &lookup, app_version)
        .map_err(|e| CommandError::validation(format!("依赖检查未通过：{}", e.user_message())))
        .map(|_| ())
}

/// 清理孤儿 staging 目录（更新 / 安装流程异常退出的残留）。
fn cleanup_stale_staging(root: &std::path::Path) {
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            if name.to_string_lossy().starts_with(".staging-") {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    }
}

/// 卸载插件（级联清理私有存储；本地包目录一并删除；会话吊销）。
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
    // 数据清理：版本历史（审计与插件日志按既有口径保留）+ 全部在途会话
    let _ = repo::delete_plugin_versions(&state.db, &id).await;
    state.plugin_sessions.revoke_plugin(&id);
    let _ = app; // 预留：卸载后 emit 事件
    Ok(())
}

/// 启用 / 停用。
///
/// 启用前：重跑兼容性检查 + **全文件完整性复核**（manifest 有 files 清单
/// 时逐文件 sha256——安装后文件被替换即拒绝启用）；停用即吊销会话。
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
        // 依赖满足性检查（缺失 / 版本过低 / 不兼容 / 依赖停用 / 循环）
        check_dependencies(&state.db, &manifest, &version).await?;
        // 全文件完整性复核（manifest.integrity + files 清单双重锚定）
        verify_installed_files(&state.db, &row)
            .await
            .map_err(command_err)?;
        repo::set_plugin_state(&state.db, &id, STATE_ENABLED, None).await?;
        audit(&state.db, &id, &format!("enable v{}", row.version)).await;
    } else {
        repo::set_plugin_state(&state.db, &id, STATE_INSTALLED, None).await?;
        state.plugin_sessions.revoke_plugin(&id);
        audit(&state.db, &id, "disable").await;
    }
    let row = repo::require_plugin(&state.db, &id).await?;
    row_to_view(row)
}

/// 安装态完整性复核：manifest 字节摘要 + files 全文件清单（如有）。
/// 结果落库（files_verified / last_verified_at）供详情页展示。
async fn verify_installed_files(
    db: &sqlx::SqlitePool,
    row: &repo::PluginRow,
) -> Result<(), miyo_core::AppError> {
    use miyo_storage::repository as repo;
    let Some(path) = &row.install_path else {
        return Ok(()); // builtin：清单以 DB 内为准，无磁盘文件可复核
    };
    let dir = std::path::PathBuf::from(path);
    if let Some(expected) = &row.integrity {
        let bytes = std::fs::read(dir.join("manifest.json")).map_err(miyo_core::AppError::Io)?;
        miyo_plugin::package::verify_manifest_integrity(&bytes, expected)?;
    }
    let manifest = row.manifest()?;
    match &manifest.files {
        Some(files) => match miyo_plugin::package::verify_dir_files(&dir, files) {
            Ok(()) => {
                repo::set_files_verified(db, &row.id, true).await?;
                Ok(())
            }
            Err(e) => {
                let _ = repo::set_files_verified(db, &row.id, false).await;
                Err(e)
            }
        },
        None => Ok(()), // v1 包：无清单（integrity 已按需复核）
    }
}

/// 调整权限快照（granted ⊆ manifest.permissions）。
///
/// 后端强制闭环：收权即吊销该插件全部会话（在途 SDK / 管理界面会话全部
/// 失效，撤销即时生效）+ 权限变动写 plugin_permission_audit 审计表。
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
    let old_granted = row.granted_permissions();
    repo::update_plugin(
        &state.db,
        &id,
        &manifest,
        row.integrity.as_deref(),
        row.signed,
        &granted_permissions,
    )
    .await?;
    // 审计：授予 / 收回分别记录（diff 基于旧快照）
    let granted_diff: Vec<&String> = granted_permissions
        .iter()
        .filter(|p| !old_granted.contains(*p))
        .collect();
    let revoked_diff: Vec<&String> = old_granted
        .iter()
        .filter(|p| !granted_permissions.contains(*p))
        .collect();
    if !granted_diff.is_empty() {
        let list: Vec<String> = granted_diff.iter().map(|s| s.to_string()).collect();
        let _ = repo::insert_permission_audit(&state.db, &id, "grant", &list).await;
    }
    if !revoked_diff.is_empty() {
        let list: Vec<String> = revoked_diff.iter().map(|s| s.to_string()).collect();
        let _ = repo::insert_permission_audit(&state.db, &id, "revoke", &list).await;
    }
    // 撤销即时生效：吊销全部在途会话
    state.plugin_sessions.revoke_plugin(&id);
    audit(
        &state.db,
        &id,
        &format!("permissions → [{}]", granted_permissions.join(", ")),
    )
    .await;
    let row = repo::require_plugin(&state.db, &id).await?;
    row_to_view(row)
}

// ---------------------------------------------------------------------------
// 市场索引 / 版本历史
// ---------------------------------------------------------------------------

/// 官方静态市场索引条目（编译期嵌入 docs/market/index.json；远端索引为
/// 路线图——本期不做服务端）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct MarketEntryView {
    pub id: String,
    pub name: String,
    pub version: String,
    pub authors: Vec<String>,
    pub description: String,
    pub category: String,
    pub permissions: Vec<String>,
    pub min_app_version: String,
    pub sdk_version: u32,
    pub changelog: String,
    pub download_url: String,
}

#[derive(Debug, Deserialize)]
struct MarketIndexFile {
    plugins: Vec<MarketEntryView>,
}

/// 官方市场索引（静态；与已安装状态合并由前端完成）。
#[tauri::command(rename_all = "camelCase")]
pub async fn plugin_market_index() -> CommandResult<Vec<MarketEntryView>> {
    const INDEX_JSON: &str = include_str!("../../../../docs/market/index.json");
    let index: MarketIndexFile = serde_json::from_str(INDEX_JSON)
        .map_err(|e| CommandError::with_code(CommandErrorCode::Internal, format!("市场索引损坏：{e}")))?;
    Ok(index.plugins)
}

/// 插件版本历史条目（回滚选择列表；不含 manifest 原文）。
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct PluginVersionView {
    pub version: String,
    pub created_at: String,
    pub signed: bool,
}

/// 查询插件版本历史（新→旧；有历史即显示「回滚」入口）。
#[tauri::command(rename_all = "camelCase")]
pub async fn plugin_versions(
    state: State<'_, AppState>,
    id: String,
) -> CommandResult<Vec<PluginVersionView>> {
    let rows = repo::list_plugin_versions(&state.db, &id).await?;
    Ok(rows
        .into_iter()
        .map(|r| PluginVersionView {
            version: r.version,
            created_at: r.created_at,
            signed: r.signed,
        })
        .collect())
}

/// 签发插件会话凭证（std 族能力的后端强制凭据）。
///
/// 仅启用态插件可签发；同插件多会话并存（上限 16，超限吊销最旧）。
/// SDK activate 与管理界面（配置 / 日志操作）都经此换取 token。
#[tauri::command(rename_all = "camelCase")]
pub async fn plugin_session_create(
    state: State<'_, AppState>,
    id: String,
) -> CommandResult<String> {
    let row = repo::require_plugin(&state.db, &id).await?;
    if row.state != STATE_ENABLED {
        return Err(CommandError::with_code(
            CommandErrorCode::PluginDisabled,
            "插件未启用：无法获取会话",
        ));
    }
    let granted = row.granted_permissions();
    Ok(state.plugin_sessions.create(&id, granted))
}

/// 上报激活失败（前端 activate 抛错时调用；保留现场供恢复）。
/// 落 error 态并吊销会话（error 态插件不得继续持有 std 族凭证）。
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
    state.plugin_sessions.revoke_plugin(&id);
    Ok(())
}

/// 完整性复核：manifest 字节摘要 + files 全文件清单（如有），结果落库。
#[tauri::command(rename_all = "camelCase")]
pub async fn plugin_verify_integrity(
    state: State<'_, AppState>,
    id: String,
) -> CommandResult<bool> {
    let row = repo::require_plugin(&state.db, &id).await?;
    let Some(path) = &row.install_path else {
        // builtin / 无 install_path：无可比对值，视为通过（清单以 DB 内为准）。
        return Ok(true);
    };
    if let Some(expected) = &row.integrity {
        let bytes = std::fs::read(std::path::PathBuf::from(path).join("manifest.json"))
            .map_err(|e| CommandError::with_code(CommandErrorCode::Io, e.to_string()))?;
        miyo_plugin::package::verify_manifest_integrity(&bytes, expected).map_err(command_err)?;
    }
    let manifest = row.manifest().map_err(command_err)?;
    if let Some(files) = &manifest.files {
        let dir = std::path::PathBuf::from(path);
        match miyo_plugin::package::verify_dir_files(&dir, files) {
            Ok(()) => {
                repo::set_files_verified(&state.db, &id, true).await?;
            }
            Err(e) => {
                let _ = repo::set_files_verified(&state.db, &id, false).await;
                return Err(command_err(e));
            }
        }
    }
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
///
/// `session_token`：会话凭证（`plugin_session_create` 签发）——std.log 属
/// std 族能力，后端逐调用强制校验 token ↔ plugin_id 绑定（收权 / 停用即失效）。
#[tauri::command(rename_all = "camelCase")]
pub async fn plugin_record_log(
    state: State<'_, AppState>,
    id: String,
    level: String,
    message: String,
    context_json: Option<String>,
    session_token: Option<String>,
) -> CommandResult<()> {
    validate_session(&state, &id, session_token)?;
    repo::insert_plugin_log(&state.db, &id, &level, &message, context_json.as_deref()).await?;
    Ok(())
}

/// 会话凭证校验（缺 token / 过期 / 跨插件 → FORBIDDEN）。
fn validate_session(
    state: &State<'_, AppState>,
    id: &str,
    session_token: Option<String>,
) -> CommandResult<()> {
    let Some(token) = session_token else {
        return Err(session_err());
    };
    state
        .plugin_sessions
        .validate(&token, id)
        .map_err(|_| session_err())
}

/// 读插件私有键（值为 JSON）。
#[tauri::command(rename_all = "camelCase")]
pub async fn plugin_data_get(
    state: State<'_, AppState>,
    id: String,
    key: String,
    session_token: Option<String>,
) -> CommandResult<Option<serde_json::Value>> {
    validate_session(&state, &id, session_token)?;
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
    session_token: Option<String>,
) -> CommandResult<()> {
    validate_session(&state, &id, session_token)?;
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
    session_token: Option<String>,
) -> CommandResult<()> {
    validate_session(&state, &id, session_token)?;
    Ok(repo::delete_plugin_data(&state.db, &id, &key).await?)
}

/// 列插件私有键。
#[tauri::command(rename_all = "camelCase")]
pub async fn plugin_data_keys(
    state: State<'_, AppState>,
    id: String,
    session_token: Option<String>,
) -> CommandResult<Vec<String>> {
    validate_session(&state, &id, session_token)?;
    Ok(repo::list_plugin_keys(&state.db, &id).await?)
}

/// 读插件包内资源（插件入口 HTML 等）。
///
/// 收口点：替代通用 `read_text_file`——路径强制限制在受管 `install_path`
/// 内（canonicalize + 前缀比对）、拒绝符号链接、扩展名白名单、大小上限。
#[tauri::command(rename_all = "camelCase")]
pub async fn plugin_read_asset(
    state: State<'_, AppState>,
    id: String,
    rel_path: String,
) -> CommandResult<String> {
    const MAX_ASSET_BYTES: u64 = 5 * 1024 * 1024;
    const ASSET_EXTENSIONS: &[&str] = &[
        "html", "htm", "js", "mjs", "css", "json", "svg", "txt", "md", "map",
    ];
    let row = repo::require_plugin(&state.db, &id).await?;
    let Some(install_path) = &row.install_path else {
        return Err(CommandError::with_code(
            CommandErrorCode::NotFound,
            "内置插件无磁盘资源可读",
        ));
    };
    // 相对路径校验（与 manifest.files / 解包白名单同口径）
    if rel_path.is_empty()
        || rel_path.len() > 200
        || rel_path.starts_with('/')
        || rel_path.starts_with('\\')
        || rel_path.contains('\\')
        || rel_path
            .split('/')
            .any(|seg| seg.is_empty() || seg == "." || seg == "..")
    {
        return Err(CommandError::validation(format!(
            "插件资源路径非法：{rel_path}"
        )));
    }
    let ext = rel_path
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !ASSET_EXTENSIONS.contains(&ext.as_str()) {
        return Err(CommandError::validation(format!(
            "插件资源扩展名 .{ext} 不在白名单内"
        )));
    }
    let root = std::path::PathBuf::from(install_path)
        .canonicalize()
        .map_err(|e| {
            CommandError::with_code(CommandErrorCode::Io, format!("插件目录解析失败：{e}"))
        })?;
    let target = root.join(&rel_path);
    let meta = std::fs::symlink_metadata(&target).map_err(|_| {
        CommandError::with_code(
            CommandErrorCode::NotFound,
            format!("插件资源不存在：{rel_path}"),
        )
    })?;
    if meta.is_symlink() {
        return Err(CommandError::with_code(
            CommandErrorCode::Forbidden,
            "插件资源是符号链接，已拒绝",
        ));
    }
    let canonical = target.canonicalize().map_err(|e| {
        CommandError::with_code(CommandErrorCode::Io, format!("插件资源解析失败：{e}"))
    })?;
    if !canonical.starts_with(&root) {
        return Err(CommandError::with_code(
            CommandErrorCode::Forbidden,
            "插件资源路径逃逸，已拒绝",
        ));
    }
    if meta.len() > MAX_ASSET_BYTES {
        return Err(CommandError::validation("插件资源超过 5 MiB 上限"));
    }
    let bytes = std::fs::read(&canonical)
        .map_err(|e| CommandError::with_code(CommandErrorCode::Io, e.to_string()))?;
    String::from_utf8(bytes).map_err(|_| {
        CommandError::validation("插件资源不是 UTF-8 文本（二进制资源不经此命令读取）")
    })
}

// ---------------------------------------------------------------------------
// 内部工具
// ---------------------------------------------------------------------------

/// 从 .miyo 文件或目录安装到受管目录：解包 / 校验 / 完整性。
/// 返回（检视结果, install_path）。
///
/// 包安装为**原子两段式**：先解包到 `{plugins_root}/.staging-<uuid>` 并完成
/// 全文件清单校验，全部通过后一次性 rename 到 `plugins/<id>`——失败时目标
/// 目录保持原状（更新失败回滚的落点，见 plugin_update_from_path）。
async fn install_from_source(
    _db: &sqlx::SqlitePool,
    source: &std::path::Path,
    trust_unsigned: bool,
) -> CommandResult<(miyo_plugin::package::PackageInfo, Option<String>)> {
    if source.is_dir() {
        // 目录直装（开发期）：记录路径本身，不复制（风险已在 UI 明示）。
        let info = miyo_plugin::package::inspect_package_dir(source).map_err(command_err)?;
        if !info.signed && !trust_unsigned {
            return Err(unsigned_error(&info));
        }
        return Ok((info, Some(source.to_string_lossy().to_string())));
    }
    let bytes = std::fs::read(source)
        .map_err(|e| CommandError::with_code(CommandErrorCode::Io, e.to_string()))?;
    // 预读大小上限（解包限制之外的前置闸：读入内存前先拒绝超大包）
    const MAX_PACKAGE_BYTES: u64 = 64 * 1024 * 1024;
    let meta = std::fs::metadata(source)
        .map_err(|e| CommandError::with_code(CommandErrorCode::Io, e.to_string()))?;
    if meta.len() > MAX_PACKAGE_BYTES {
        return Err(CommandError::validation("插件包超过 64 MiB 上限"));
    }
    let info = miyo_plugin::package::inspect_package_bytes(&bytes).map_err(command_err)?;
    if !info.signed && !trust_unsigned {
        return Err(unsigned_error(&info));
    }
    let dest = plugins_root().join(&info.manifest.id);
    let staging = plugins_root().join(format!(".staging-{}", uuid::Uuid::new_v4().simple()));
    // 解包 + 全文件清单校验（staging 内完成；失败即清理，不动 dest）
    let extracted =
        miyo_plugin::package::extract_package_to_dir(&bytes, &staging).map_err(|e| {
            let _ = std::fs::remove_dir_all(&staging);
            command_err(e)
        })?;
    // 重装（卸载残留 / 版本替换）：清空目标目录后原子换入
    if dest.exists() {
        if let Err(e) = std::fs::remove_dir_all(&dest) {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(CommandError::with_code(CommandErrorCode::Io, e.to_string()));
        }
    }
    if let Err(e) = std::fs::rename(&staging, &dest) {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(CommandError::with_code(CommandErrorCode::Io, e.to_string()));
    }
    Ok((extracted, Some(dest.to_string_lossy().to_string())))
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

    // ---------------- 更新 / 回滚 E2E（真实 .miyo 包，非 mock） ----------------

    use crate::sessions::PluginSessions;

    /// 构造内存 tar.gz（与 miyo-plugin package.rs 测试同法）。
    fn build_tar_gz(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        for (name, content) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(content.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder
                .append_data(&mut header, name, std::io::Cursor::new(*content))
                .unwrap();
        }
        let tar_bytes = builder.into_inner().unwrap();
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        std::io::Write::write_all(&mut gz, &tar_bytes).unwrap();
        gz.finish().unwrap()
    }

    fn sha(content: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        hex::encode(Sha256::digest(content))
    }

    /// vN 插件包（带 files 清单，manifest 自声明 integrity）。
    fn build_package(version: &str, html: &[u8]) -> Vec<u8> {
        let manifest = format!(
            r#"{{"id":"com.e2e.upd","name":"Upd","description":"E2E 更新","version":"{version}",
                "kind":"ui","min_app_version":"0.0.60","sdk_version":1,
                "permissions":["std.storage"],"entry":"index.html",
                "files":[{{"path":"index.html","sha256":"{sha}","size":{size}}}]}}"#,
            version = version,
            sha = sha(html),
            size = html.len()
        );
        // 不带 integrity 自声明（安装端现算 manifest 字节摘要并落库——
        // 自引用字段无法自洽，hello-world 示例同样不带）
        build_tar_gz(&[("manifest.json", manifest.as_bytes()), ("index.html", html)])
    }

    async fn test_db() -> (tempfile::TempDir, sqlx::SqlitePool) {
        let tmp = tempfile::tempdir().unwrap();
        let db = miyo_storage::db::init_db(&tmp.path().join("t.db"))
            .await
            .unwrap();
        (tmp, db)
    }

    #[tokio::test]
    async fn install_update_rollback_tamper_full_cycle() {
        let (_dir, db) = test_db().await;
        let sessions = PluginSessions::new();
        let app_version = "0.0.60";
        let pkg_dir = tempfile::tempdir().unwrap();

        // ---- 安装 v1 ----
        let v1 = build_package("1.0.0", b"<html>v1</html>");
        let p1 = pkg_dir.path().join("v1.miyo");
        std::fs::write(&p1, &v1).unwrap();
        let view = install_impl(&db, &sessions, &p1, true, app_version)
            .await
            .unwrap();
        assert_eq!(view.version, "1.0.0");
        assert!(view.files_verified);
        repo::set_plugin_state(&db, "com.e2e.upd", STATE_ENABLED, None)
            .await
            .unwrap();
        // 会话可用（std 族 token 签发）
        sessions.create("com.e2e.upd", vec![]);

        // ---- 篡改包：文件内容与清单不符 → 更新拒绝，v1 原样 ----
        let bad = build_tar_gz(&[
            (
                "manifest.json",
                &build_manifest_bytes("2.0.0", b"<html>v2</html>"),
            ),
            ("index.html", b"<html>TAMPERED</html>"),
        ]);
        let pb = pkg_dir.path().join("bad.miyo");
        std::fs::write(&pb, &bad).unwrap();
        assert!(update_impl(&db, &sessions, &pb, true, app_version)
            .await
            .is_err());
        assert_eq!(
            repo::require_plugin(&db, "com.e2e.upd")
                .await
                .unwrap()
                .version,
            "1.0.0"
        );

        // ---- 更新 v2 ----
        let v2 = build_package("2.0.0", b"<html>v2</html>");
        let p2 = pkg_dir.path().join("v2.miyo");
        std::fs::write(&p2, &v2).unwrap();
        let view = update_impl(&db, &sessions, &p2, true, app_version)
            .await
            .unwrap();
        assert_eq!(view.version, "2.0.0");
        assert!(view.files_verified);
        // 版本历史落库 + 备份目录存在
        assert_eq!(
            repo::list_plugin_versions(&db, "com.e2e.upd")
                .await
                .unwrap()
                .len(),
            1
        );
        assert!(plugins_root().join(".rollback-com.e2e.upd").is_dir());

        // ---- 回滚 → v1 ----
        let view = rollback_impl(&db, &sessions, "com.e2e.upd").await.unwrap();
        assert_eq!(view.version, "1.0.0");
        // 快照已消费（防重复回滚）
        assert_eq!(
            repo::list_plugin_versions(&db, "com.e2e.upd")
                .await
                .unwrap()
                .len(),
            0
        );
        assert!(!plugins_root().join(".rollback-com.e2e.upd").exists());
        // 回滚后的目录文件确为 v1 内容（备份目录原样换回）
        let html = std::fs::read(plugins_root().join("com.e2e.upd/index.html")).unwrap();
        assert_eq!(html, b"<html>v1</html>");
    }

    /// 构造 manifest 字节（含 integrity 自声明）。
    fn build_manifest_bytes(version: &str, html: &[u8]) -> Vec<u8> {
        let manifest = format!(
            r#"{{"id":"com.e2e.upd","name":"Upd","description":"E2E 更新","version":"{version}",
                "kind":"ui","min_app_version":"0.0.60","sdk_version":1,
                "permissions":["std.storage"],"entry":"index.html",
                "files":[{{"path":"index.html","sha256":"{sha}","size":{size}}}]}}"#,
            version = version,
            sha = sha(html),
            size = html.len()
        );
        manifest.into_bytes()
    }
}
