//! HTTP 设置 Command：全局代理（持久化 + 应用到 miyo-http 共享客户端）。

use tauri::State;

use miyo_storage::repository as repo;

use crate::error::{CommandError, CommandResult};
use crate::state::AppState;

/// settings 表中的代理键；值为 JSON 字符串（`null` = 直连）。
const PROXY_KEY: &str = "http_proxy";

/// settings 表中的请求超时键；值为 JSON 数字（毫秒）。
const HTTP_TIMEOUT_KEY: &str = "http_timeout_ms";

/// 读取请求超时设置（毫秒）。未配置时返回 `None`，由调用方兜底默认值。
pub async fn read_http_timeout_ms(db: &sqlx::SqlitePool) -> CommandResult<Option<u64>> {
    let raw = repo::get_setting(db, HTTP_TIMEOUT_KEY).await?;
    match raw {
        None => Ok(None),
        Some(json) => serde_json::from_str::<Option<u64>>(&json)
            .map_err(|e| CommandError::with_code("INTERNAL", format!("超时设置解析失败：{e}"))),
    }
}

/// settings 表中 TLS 客户端证书路径键；值为 JSON 字符串（`null` = 不使用）。
const TLS_IDENTITY_KEY: &str = "tls_identity_path";
/// settings 表中「放行自签 / 无效证书」键；值为 JSON bool。
const TLS_ACCEPT_INVALID_KEY: &str = "tls_accept_invalid_certs";

/// TLS 设置（设置页回显结构）。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TlsSettings {
    /// 客户端证书 PEM 文件路径（证书链 + 私钥同文件；`null` = 不使用）。
    pub identity_path: Option<String>,
    /// 放行自签名 / 无效证书（内网调试）。
    pub accept_invalid_certs: bool,
}

/// 读取 TLS 设置（settings 表回显）。
#[tauri::command(rename_all = "camelCase")]
pub async fn get_tls_settings(state: State<'_, AppState>) -> CommandResult<TlsSettings> {
    let identity = repo::get_setting(&state.db, TLS_IDENTITY_KEY).await?;
    let accept = repo::get_setting(&state.db, TLS_ACCEPT_INVALID_KEY).await?;
    Ok(TlsSettings {
        identity_path: identity
            .and_then(|v| serde_json::from_str::<Option<String>>(&v).ok().flatten()),
        accept_invalid_certs: accept
            .and_then(|v| serde_json::from_str::<bool>(&v).ok())
            .unwrap_or(false),
    })
}

/// 保存 TLS 设置并立即应用到共享 HTTP 客户端（整体重建，代理设置保持）。
///
/// 证书在本命令内先读取并解析一次：失败即报错，不落库（避免坏配置静默
/// 生效到后续所有请求）。路径只存本机 settings 表（不进备份 / 同步键集）。
#[tauri::command(rename_all = "camelCase")]
pub async fn set_tls_settings(
    state: State<'_, AppState>,
    identity_path: Option<String>,
    accept_invalid_certs: bool,
) -> CommandResult<()> {
    let identity = identity_path
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty());
    miyo_http::client::set_tls_options(&miyo_http::client::TlsOptions {
        identity_path: identity.clone(),
        accept_invalid_certs,
    })
    .map_err(|e| CommandError::validation(e.user_message()))?;
    let identity_json = serde_json::to_string(&identity)
        .map_err(|e| CommandError::with_code("INTERNAL", format!("序列化失败：{e}")))?;
    repo::set_setting(&state.db, TLS_IDENTITY_KEY, &identity_json).await?;
    repo::set_setting(
        &state.db,
        TLS_ACCEPT_INVALID_KEY,
        &accept_invalid_certs.to_string(),
    )
    .await?;
    Ok(())
}

/// 启动时恢复 TLS 设置（失败静默保持默认：无证书 + 标准校验）。
pub async fn apply_saved_tls(db: &sqlx::SqlitePool) {
    let identity = match repo::get_setting(db, TLS_IDENTITY_KEY).await {
        Ok(Some(v)) => serde_json::from_str::<Option<String>>(&v).ok().flatten(),
        _ => None,
    };
    let accept = match repo::get_setting(db, TLS_ACCEPT_INVALID_KEY).await {
        Ok(Some(v)) => serde_json::from_str::<bool>(&v).unwrap_or(false),
        _ => false,
    };
    if identity.is_none() && !accept {
        return;
    }
    let _ = miyo_http::client::set_tls_options(&miyo_http::client::TlsOptions {
        identity_path: identity,
        accept_invalid_certs: accept,
    });
}

/// 读取全局请求超时（毫秒；未设置时返回 None，前端展示用）。
#[tauri::command(rename_all = "camelCase")]
pub async fn get_http_timeout_ms(state: State<'_, AppState>) -> CommandResult<Option<u64>> {
    read_http_timeout_ms(&state.db).await
}

/// 设置全局请求超时（毫秒；范围 1000ms ~ 1h）。
#[tauri::command(rename_all = "camelCase")]
pub async fn set_http_timeout_ms(state: State<'_, AppState>, timeout_ms: u64) -> CommandResult<()> {
    if !(1000..=3_600_000).contains(&timeout_ms) {
        return Err(CommandError::validation("超时需在 1 秒 ~ 1 小时之间"));
    }
    let json = serde_json::to_string(&Some(timeout_ms))
        .map_err(|e| CommandError::with_code("INTERNAL", format!("序列化失败：{e}")))?;
    repo::set_setting(&state.db, HTTP_TIMEOUT_KEY, &json).await?;
    state.invalidate_global_settings();
    Ok(())
}

/// 读取全局代理地址（None = 直连）。
#[tauri::command(rename_all = "camelCase")]
pub async fn get_http_proxy(state: State<'_, AppState>) -> CommandResult<Option<String>> {
    let raw = repo::get_setting(&state.db, PROXY_KEY)
        .await
        .map_err(CommandError::from)?;
    match raw {
        None => Ok(None),
        Some(json) => serde_json::from_str::<Option<String>>(&json)
            .map_err(|e| CommandError::with_code("INTERNAL", format!("代理设置解析失败：{e}"))),
    }
}

/// 设置全局代理（`http://host:port` / `socks5://host:port`；None = 直连）。
///
/// 持久化到 settings 并立即应用到共享 HTTP 客户端；应用启动时
/// （[`crate::state`] 初始化后）通过 [`apply_saved_proxy`] 恢复。
#[tauri::command(rename_all = "camelCase")]
pub async fn set_http_proxy(
    state: State<'_, AppState>,
    proxy: Option<String>,
) -> CommandResult<()> {
    let trimmed = proxy
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty());
    if let Some(p) = &trimmed {
        // 提前校验格式，避免坏地址在每次发请求时才报错
        miyo_http::client::validate_proxy(p)
            .map_err(|e| CommandError::validation(e.user_message()))?;
    }
    miyo_http::client::set_proxy(trimmed.as_deref())
        .map_err(|e| CommandError::validation(e.user_message()))?;
    let json = serde_json::to_string(&trimmed)
        .map_err(|e| CommandError::with_code("INTERNAL", format!("序列化失败：{e}")))?;
    repo::set_setting(&state.db, PROXY_KEY, &json)
        .await
        .map_err(CommandError::from)?;
    Ok(())
}

/// 代理连通性测试结果。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyTestResult {
    pub ok: bool,
    pub status: u16,
    pub duration_ms: f64,
    pub message: String,
}

/// 测试全局代理连通性：经当前共享客户端（含已设代理）向目标 URL 发一次请求。
///
/// 复用 `execute_request` 同款客户端，因此若代理已设置则请求会走代理；未设置代理
/// 时即直连探测（用于校验目标可达性）。超时 8 秒，不落历史。
#[tauri::command(rename_all = "camelCase")]
pub async fn test_http_proxy(target: Option<String>) -> CommandResult<ProxyTestResult> {
    use miyo_core::model::{HttpMethod, RequestSpec};
    let url = target
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| "https://www.gstatic.com/generate_204".to_string());
    let spec = RequestSpec::default();
    let started = std::time::Instant::now();
    match miyo_http::client::send_request(HttpMethod::GET, &url, &spec, Some(8_000)).await {
        Ok(resp) => Ok(ProxyTestResult {
            ok: true,
            status: resp.status,
            duration_ms: started.elapsed().as_secs_f64() * 1000.0,
            message: format!(
                "连通成功（HTTP {}，{:.0}ms）",
                resp.status, resp.duration_ms
            ),
        }),
        Err(e) => Ok(ProxyTestResult {
            ok: false,
            status: 0,
            duration_ms: started.elapsed().as_secs_f64() * 1000.0,
            message: format!("连通失败：{}", e.user_message()),
        }),
    }
}

/// 启动时恢复持久化的代理设置（设置加载失败时静默保持直连）。
pub async fn apply_saved_proxy(db: &sqlx::SqlitePool) {
    let raw = match repo::get_setting(db, PROXY_KEY).await {
        Ok(Some(json)) => json,
        _ => return,
    };
    if let Ok(Some(proxy)) = serde_json::from_str::<Option<String>>(&raw) {
        if !proxy.is_empty() {
            let _ = miyo_http::client::set_proxy(Some(proxy.as_str()));
        }
    }
}

/// 当前生效的数据目录（miyo.db / master.key / logs / snapshots 所在）。
#[tauri::command(rename_all = "camelCase")]
pub async fn get_data_dir() -> CommandResult<String> {
    Ok(miyo_storage::db::data_dir().to_string_lossy().to_string())
}

/// 默认数据目录（无覆盖时；恢复默认用）。
#[tauri::command(rename_all = "camelCase")]
pub async fn get_default_data_dir() -> CommandResult<String> {
    Ok(miyo_storage::db::default_data_dir()
        .to_string_lossy()
        .to_string())
}

/// 设置数据目录：校验绝对路径 + 建目录 + 可写探测，通过后写入 bootstrap 文件。
/// 即时生效需重启（DB 连接池与密钥缓存与进程同生命周期）。
/// 注意：新目录从空数据启动，旧数据请先用备份 JSON 导出、切换后导入。
#[tauri::command(rename_all = "camelCase")]
pub async fn set_data_dir(path: String) -> CommandResult<()> {
    validate_data_dir_candidate(&path)?;
    let target = std::path::PathBuf::from(path.trim());
    // 异步文件系统调用：不在 IPC 线程上做同步目录 / 文件 IO。
    tokio::fs::create_dir_all(&target)
        .await
        .map_err(|e| CommandError::with_code("IO", format!("创建目录失败：{e}")))?;
    // 可写探测（建删空文件；失败即拒收，避免重启后打不开库）。
    let probe = target.join(".miyo-write-test");
    tokio::fs::write(&probe, b"ok")
        .await
        .map_err(|e| CommandError::with_code("IO", format!("目录不可写：{e}")))?;
    let _ = tokio::fs::remove_file(&probe).await;
    let bootstrap = miyo_storage::db::bootstrap_path();
    if let Some(parent) = bootstrap.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| CommandError::with_code("IO", format!("创建配置目录失败：{e}")))?;
    }
    tokio::fs::write(&bootstrap, target.to_string_lossy().as_bytes())
        .await
        .map_err(|e| CommandError::with_code("IO", format!("写入目录配置失败：{e}")))?;
    Ok(())
}

/// 恢复默认数据目录（删除 bootstrap 覆盖；重启后生效）。
#[tauri::command(rename_all = "camelCase")]
pub async fn reset_data_dir() -> CommandResult<()> {
    let bootstrap = miyo_storage::db::bootstrap_path();
    if tokio::fs::try_exists(&bootstrap).await.unwrap_or(false) {
        tokio::fs::remove_file(&bootstrap)
            .await
            .map_err(|e| CommandError::with_code("IO", format!("删除目录配置失败：{e}")))?;
    }
    Ok(())
}

/// 校验候选目录：非空、绝对路径、与默认目录不相同。
fn validate_data_dir_candidate(raw: &str) -> CommandResult<()> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(CommandError::validation("数据目录不能为空"));
    }
    let p = std::path::PathBuf::from(trimmed);
    if !p.is_absolute() {
        return Err(CommandError::validation("数据目录必须使用绝对路径"));
    }
    if same_path(&p, &miyo_storage::db::default_data_dir()) {
        return Err(CommandError::validation("已是默认目录，无需设置"));
    }
    Ok(())
}

/// 路径等价比较（大小写/分隔符按平台语义；`~` 不展开——候选已要求绝对路径）。
#[cfg(not(windows))]
fn same_path(a: &std::path::Path, b: &std::path::Path) -> bool {
    a == b
}

/// Windows 上路径大小写不敏感，逐组件比较。
#[cfg(windows)]
fn same_path(a: &std::path::Path, b: &std::path::Path) -> bool {
    let norm = |p: &std::path::Path| {
        p.components()
            .map(|c| c.as_os_str().to_string_lossy().to_lowercase())
            .collect::<Vec<_>>()
    };
    norm(a) == norm(b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::OnceLock;

    /// 环境变量是进程全局的：涉及覆盖解析的用例串行执行。
    ///
    /// 用 `tokio::sync::Mutex` 而非 std Mutex：守卫要跨 `.await` 持有，
    /// std 守卫是线程绑定的，任务在 await 间被调度到别的线程会死锁
    ///（clippy `await_holding_lock` 同样会拦下）。
    async fn env_serial() -> tokio::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
            .lock()
            .await
    }

    /// 隔离 bootstrap 落盘位置（否则单测会污染真实数据目录）。
    fn isolated_bootstrap(name: &str) -> PathBufGuard {
        let dir =
            std::env::temp_dir().join(format!("miyo-bootstrap-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建隔离目录");
        let prev = std::env::var_os(miyo_core::paths::BOOTSTRAP_DIR_ENV);
        std::env::set_var(miyo_core::paths::BOOTSTRAP_DIR_ENV, &dir);
        PathBufGuard { dir, prev }
    }

    struct PathBufGuard {
        dir: std::path::PathBuf,
        prev: Option<std::ffi::OsString>,
    }

    impl Drop for PathBufGuard {
        fn drop(&mut self) {
            match &self.prev {
                Some(v) => std::env::set_var(miyo_core::paths::BOOTSTRAP_DIR_ENV, v),
                None => std::env::remove_var(miyo_core::paths::BOOTSTRAP_DIR_ENV),
            }
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn validate_rejects_bad_candidates() {
        assert!(validate_data_dir_candidate("").is_err());
        assert!(validate_data_dir_candidate("   ").is_err());
        assert!(validate_data_dir_candidate("relative/path").is_err());
        let def = miyo_storage::db::default_data_dir()
            .to_string_lossy()
            .to_string();
        assert!(
            validate_data_dir_candidate(&def).is_err(),
            "与默认目录相同应拒收"
        );
    }

    #[tokio::test]
    async fn set_reset_data_dir_roundtrip() {
        let _serial = env_serial().await;
        let _iso = isolated_bootstrap("roundtrip");
        // 确保环境变量覆盖不干扰 bootstrap 路径断言
        let prev_data_env = std::env::var_os(miyo_core::paths::DATA_DIR_ENV);
        std::env::remove_var(miyo_core::paths::DATA_DIR_ENV);

        let target = std::env::temp_dir().join(format!("miyo-datadir-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&target);
        set_data_dir(target.to_string_lossy().to_string())
            .await
            .expect("合法目录应写入成功");
        assert!(
            target.join("miyo.db").parent().is_some(),
            "目标目录应已创建"
        );
        assert_eq!(
            miyo_core::paths::data_dir_override(),
            Some(target.clone()),
            "写入后覆盖应生效"
        );
        assert_eq!(
            get_data_dir().await.expect("读取"),
            target.to_string_lossy().to_string()
        );

        reset_data_dir().await.expect("重置应成功");
        assert_eq!(miyo_core::paths::data_dir_override(), None);
        let _ = std::fs::remove_dir_all(&target);
        match prev_data_env {
            Some(v) => std::env::set_var(miyo_core::paths::DATA_DIR_ENV, v),
            None => std::env::remove_var(miyo_core::paths::DATA_DIR_ENV),
        }
    }
}
