//! Java 命令族共享门禁（v1 Runtime + v2 Attach 两侧同源）。
//!
//! 两层：
//! 1. [`ensure_java_plugin`]：插件安装 / 启用态（未装 → PLUGIN_NOT_INSTALLED，
//!    停用 → PLUGIN_DISABLED）——全部 28 个 java 命令的前置；
//! 2. [`ensure_java_permission`]：权限细分（`java.*` 子权限，granted 快照
//!    由 `plugin_set_permissions` 维护、`granted_permissions_json` 落库）——
//!    高危命令按命令族映射逐条检查；含 `java.debug` 聚合权限的存量行
//!    等价展开（见 [`miyo_plugin::permission::granted_effective`]）。

use sqlx::SqlitePool;

use miyo_storage::repository as repo;

use crate::error::{CommandError, CommandErrorCode, CommandResult};

pub(crate) const JAVA_PLUGIN_ID: &str = miyo_plugin::JAVA_DEBUG_PLUGIN_ID;

/// 插件门禁：Java 调试命令只在内置插件「已安装且启用」时放行。
pub(crate) async fn ensure_java_plugin(db: &SqlitePool) -> CommandResult<()> {
    if !repo::is_plugin_installed(db, JAVA_PLUGIN_ID).await? {
        return Err(CommandError::with_code(
            CommandErrorCode::PluginNotInstalled,
            "Java 调试插件未安装：请在插件管理中安装后使用",
        ));
    }
    if !repo::is_plugin_enabled(db, JAVA_PLUGIN_ID).await? {
        return Err(CommandError::with_code(
            CommandErrorCode::PluginDisabled,
            "Java 调试插件已停用：请在插件管理中启用",
        ));
    }
    Ok(())
}

/// 权限门禁：在插件门禁之上检查 granted 快照是否持有指定 `java.*` 子权限。
pub(crate) async fn ensure_java_permission(
    db: &SqlitePool,
    permission: &str,
) -> CommandResult<()> {
    ensure_java_plugin(db).await?;
    let granted = repo::require_plugin(db, JAVA_PLUGIN_ID)
        .await?
        .granted_permissions();
    if !miyo_plugin::permission::granted_effective(&granted, permission) {
        return Err(CommandError::with_code(
            CommandErrorCode::Forbidden,
            format!("插件缺少权限「{permission}」：请在插件管理 → 权限中授权"),
        ));
    }
    Ok(())
}
