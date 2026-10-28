//! OAuth2 授权核心：浏览器授权码流、code 换取 token、refresh_token 静默刷新、
//! 非交互式模式（client_credentials / password）直接换取令牌。

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use axum::extract::Query;
use axum::response::Html;
use axum::routing::get;
use axum::Router;
use miyo_core::model::{AuthSpec, OAuth2Grant, OAuth2Token};
use tokio::sync::{mpsc, watch, Mutex as AsyncMutex};
use url::Url;

use crate::{cache, REFRESH_AHEAD};

/// OAuth2 流程错误（全部为面向用户的中文消息）。
#[derive(Debug, thiserror::Error)]
pub enum OAuth2Error {
    #[error("{0}")]
    Config(String),
    #[error("{0}")]
    Callback(String),
    #[error("{0}")]
    Exchange(String),
    #[error("{0}")]
    Refresh(String),
    #[error("{0}")]
    Unauthorized(String),
}

/// 授权流程互斥：同一时刻只有一个授权会话在监听 9090 端口。
static AUTHORIZE_LOCK: OnceLock<AsyncMutex<()>> = OnceLock::new();

/// 每 key 刷新互斥：并发请求时只发一次刷新请求，其余等待结果。
///
/// 无人持有后惰性摘除（`refresh_with_dedupe` 末尾检查引用计数），
/// 避免长期运行的 key 常驻增长。
static REFRESH_LOCKS: OnceLock<Mutex<HashMap<Key, Arc<AsyncMutex<()>>>>> = OnceLock::new();

/// token 端点共享客户端：连接池复用（原来每次刷新/换码都 `Client::new()`，
/// 高频刷新时多出 TLS 握手；miyo-http 侧已有全局池，但 miyo-http 依赖本 crate，
/// 为避循环依赖此处自建等价单例）。
static TOKEN_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

fn token_client() -> &'static reqwest::Client {
    TOKEN_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .build()
            .expect("OAuth token 客户端构建不应失败")
    })
}

type Key = (String, String);

fn refresh_lock(client_id: &str, token_url: &str) -> Arc<AsyncMutex<()>> {
    let mut map = REFRESH_LOCKS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    map.entry((client_id.to_string(), token_url.to_string()))
        .or_insert_with(|| Arc::new(AsyncMutex::new(())))
        .clone()
}

/// 刷新完成后若无其他持有者，从表中摘除（调用时锁已释放）。
fn evict_refresh_lock(client_id: &str, token_url: &str, lock: &Arc<AsyncMutex<()>>) {
    // map 自身 1 + 本次调用 1：无并发等待者时摘除。
    if Arc::strong_count(lock) > 2 {
        return;
    }
    if let Ok(mut map) = REFRESH_LOCKS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
    {
        let key = (client_id.to_string(), token_url.to_string());
        if map.get(&key).is_some_and(|v| Arc::ptr_eq(v, lock)) && Arc::strong_count(lock) <= 2 {
            map.remove(&key);
        }
    }
}

/// 从 AuthSpec 提取 OAuth2 配置。
fn oauth2_fields(auth: &AuthSpec) -> Result<OAuth2Fields, OAuth2Error> {
    let AuthSpec::OAuth2 {
        client_id,
        client_secret,
        auth_url,
        token_url,
        scope,
        redirect_uri,
        grant,
        username,
        password,
        ..
    } = auth
    else {
        return Err(OAuth2Error::Config("认证方式不是 OAuth2".to_string()));
    };
    if client_id.trim().is_empty() {
        return Err(OAuth2Error::Config("client_id 未配置".to_string()));
    }
    if token_url.trim().is_empty() {
        return Err(OAuth2Error::Config("token_url 未配置".to_string()));
    }
    Ok(OAuth2Fields {
        client_id: client_id.clone(),
        client_secret: client_secret.clone(),
        auth_url: auth_url.clone(),
        token_url: token_url.clone(),
        scope: scope.clone(),
        redirect_uri: if redirect_uri.trim().is_empty() {
            crate::DEFAULT_REDIRECT_URI.to_string()
        } else {
            redirect_uri.clone()
        },
        grant: *grant,
        username: username.clone(),
        password: password.clone(),
    })
}

struct OAuth2Fields {
    client_id: String,
    client_secret: String,
    auth_url: String,
    token_url: String,
    scope: String,
    redirect_uri: String,
    grant: OAuth2Grant,
    username: Option<String>,
    password: Option<String>,
}

/// 取 access token：缓存有效（未过期且未临近过期）直接返回；
/// 过期 / 即将过期 → 用 refresh_token 静默刷新（每 key 串行，防并发重复刷新）。
pub async fn access_token_for(auth: &AuthSpec) -> Result<String, OAuth2Error> {
    let fields = oauth2_fields(auth)?;
    let AuthSpec::OAuth2 { token, .. } = auth else {
        unreachable!()
    };

    // 1. 缓存命中且有效 → 直接用。
    if let Some(t) = cache::cached_token(&fields.client_id, &fields.token_url) {
        if !t.is_expired() && !t.expires_within(REFRESH_AHEAD) {
            return Ok(t.access_token);
        }
    }
    // 2. 缓存为空 → 以 spec 内嵌 token 作种子（首次请求，尚未刷新过）。
    if cache::cached_token(&fields.client_id, &fields.token_url).is_none() {
        if let Some(t) = token {
            cache::store_token(&fields.client_id, &fields.token_url, t.clone());
        }
    }
    let cached = cache::cached_token(&fields.client_id, &fields.token_url);
    // 非交互式模式（client_credentials / password）：无凭据或过期 → 直接重取
    // （这类模式通常不返回 refresh_token，整体重取即续期）。
    let interactive = fields.grant == OAuth2Grant::AuthorizationCode;
    match cached {
        // 有效直接用；交互模式临近过期走静默刷新续期。
        Some(t) if !t.is_expired() => {
            if interactive && t.expires_within(REFRESH_AHEAD) {
                refresh_with_dedupe(&fields, t).await
            } else {
                Ok(t.access_token)
            }
        }
        // 已过期：交互模式刷新；非交互模式由下方兜底重取。
        Some(t) if interactive => refresh_with_dedupe(&fields, t).await,
        // 已过期（非交互）或无凭据：非交互模式直接重取；交互模式报未授权。
        Some(_) | None => {
            if !interactive {
                fetch_token_with_fields(&fields)
                    .await
                    .map(|t| t.access_token)
            } else {
                Err(OAuth2Error::Unauthorized(
                    "尚未完成 OAuth2 授权，请先在认证页点击「立即授权」".to_string(),
                ))
            }
        }
    }
}

/// 非交互式获取令牌（client_credentials / password 模式）：直接向 token_url
/// 换取，结果写入进程内缓存并返回（调用方可写回 AuthSpec 展示有效期）。
/// authorization_code 模式不走这里（需浏览器授权，见 [`authorize`]）。
pub async fn fetch_token(auth: &AuthSpec) -> Result<OAuth2Token, OAuth2Error> {
    let fields = oauth2_fields(auth)?;
    fetch_token_with_fields(&fields).await
}

/// [`fetch_token`] 的字段预提取版（access_token_for 内避免二次解析）。
async fn fetch_token_with_fields(fields: &OAuth2Fields) -> Result<OAuth2Token, OAuth2Error> {
    let fresh = match fields.grant {
        OAuth2Grant::ClientCredentials => exchange_client_credentials(fields).await?,
        OAuth2Grant::Password => exchange_password(fields).await?,
        OAuth2Grant::AuthorizationCode => {
            return Err(OAuth2Error::Config(
                "authorization_code 模式请在认证页点击「立即授权」走浏览器流程".to_string(),
            ));
        }
    };
    cache::store_token(&fields.client_id, &fields.token_url, fresh.clone());
    Ok(fresh)
}

/// client_credentials：POST token_url（form：grant_type=client_credentials）。
async fn exchange_client_credentials(fields: &OAuth2Fields) -> Result<OAuth2Token, OAuth2Error> {
    let mut form = vec![
        ("grant_type", "client_credentials"),
        ("client_id", fields.client_id.as_str()),
        ("client_secret", fields.client_secret.as_str()),
    ];
    if !fields.scope.trim().is_empty() {
        form.push(("scope", fields.scope.trim()));
    }
    request_token(fields, &form).await
}

/// password：POST token_url（form：grant_type=password + 资源所有者凭据）。
async fn exchange_password(fields: &OAuth2Fields) -> Result<OAuth2Token, OAuth2Error> {
    let username = fields
        .username
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| OAuth2Error::Config("password 模式需要填写用户名".to_string()))?;
    let password = fields.password.as_deref().unwrap_or("");
    let mut form = vec![
        ("grant_type", "password"),
        ("username", username),
        ("password", password),
        ("client_id", fields.client_id.as_str()),
        ("client_secret", fields.client_secret.as_str()),
    ];
    if !fields.scope.trim().is_empty() {
        form.push(("scope", fields.scope.trim()));
    }
    request_token(fields, &form).await
}

/// 发送 token 请求并解析响应（成功 → OAuth2Token；401/400 → 凭据被拒）。
async fn request_token(
    fields: &OAuth2Fields,
    form: &[(&str, &str)],
) -> Result<OAuth2Token, OAuth2Error> {
    let resp = token_client()
        .post(&fields.token_url)
        .form(form)
        .send()
        .await
        .map_err(|e| OAuth2Error::Exchange(format!("令牌请求失败：{e}")))?;
    let status = resp.status().as_u16();
    let text = resp.text().await.unwrap_or_else(|_| String::new());
    if !(200..300).contains(&status) {
        if status == 401 || status == 400 {
            return Err(OAuth2Error::Unauthorized(format!(
                "令牌请求被拒绝（HTTP {status}），请检查 client_id / secret / 用户名密码"
            )));
        }
        return Err(OAuth2Error::Exchange(format!(
            "令牌请求失败（HTTP {status}）：{}",
            text.chars().take(200).collect::<String>()
        )));
    }
    parse_token_payload(&text).map_err(OAuth2Error::Exchange)
}

/// 带去重的刷新：同一 key 的并发调用共享一次网络刷新。
async fn refresh_with_dedupe(
    fields: &OAuth2Fields,
    expired: OAuth2Token,
) -> Result<String, OAuth2Error> {
    let lock = refresh_lock(&fields.client_id, &fields.token_url);
    let result = async {
        let _guard = lock.lock().await;
        // 抢到锁后复查：可能前一个调用者已刷新完成。
        if let Some(t) = cache::cached_token(&fields.client_id, &fields.token_url) {
            if !t.is_expired() && !t.expires_within(REFRESH_AHEAD) {
                return Ok(t.access_token);
            }
        }
        let refresh_token = expired.refresh_token.clone().ok_or_else(|| {
            OAuth2Error::Unauthorized("缺少 refresh_token，请重新授权".to_string())
        })?;
        let fresh = exchange_refresh(fields, &refresh_token).await?;
        cache::store_token(&fields.client_id, &fields.token_url, fresh.clone());
        Ok(fresh.access_token)
    }
    .await;
    // 锁已释放：无等待者时摘除表项，防 key 常驻增长。
    evict_refresh_lock(&fields.client_id, &fields.token_url, &lock);
    result
}

/// 刷新令牌：POST token_url（form：grant_type=refresh_token）。
async fn exchange_refresh(
    fields: &OAuth2Fields,
    refresh_token: &str,
) -> Result<OAuth2Token, OAuth2Error> {
    let resp = token_client()
        .post(&fields.token_url)
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", fields.client_id.as_str()),
            ("client_secret", fields.client_secret.as_str()),
        ])
        .send()
        .await
        .map_err(|e| OAuth2Error::Refresh(format!("刷新请求失败：{e}")))?;

    let status = resp.status().as_u16();
    let text = resp.text().await.unwrap_or_else(|_| String::new());
    if !(200..300).contains(&status) {
        if status == 401 || status == 400 {
            return Err(OAuth2Error::Unauthorized(format!(
                "刷新令牌被拒绝（HTTP {status}），请重新授权"
            )));
        }
        return Err(OAuth2Error::Refresh(format!(
            "刷新令牌失败（HTTP {status}）：{}",
            text.chars().take(200).collect::<String>()
        )));
    }
    parse_token_payload(&text).map_err(OAuth2Error::Refresh)
}

/// 从 redirect_uri 推导回调监听地址与路径（host 必须是本机回环地址，
/// 端口/路径可自定义以避开占用——如代理软件常占 9090）。
fn redirect_binding(redirect_uri: &str) -> Result<(String, u16, String), OAuth2Error> {
    let u = Url::parse(redirect_uri)
        .map_err(|e| OAuth2Error::Config(format!("redirect_uri 无效：{e}")))?;
    match u.host_str() {
        Some("127.0.0.1" | "localhost") => {}
        _ => {
            return Err(OAuth2Error::Config(
                "redirect_uri 必须指向本机回环地址（127.0.0.1 或 localhost）".to_string(),
            ))
        }
    }
    let port = u.port().unwrap_or(crate::CALLBACK_PORT);
    let path = if u.path().is_empty() {
        "/callback".to_string()
    } else {
        u.path().to_string()
    };
    Ok(("127.0.0.1".to_string(), port, path))
}

/// 授权码流完整流程：
/// 1. 本地回调服务器监听 redirect_uri 端口（默认 127.0.0.1:9090）；
/// 2. 打开系统浏览器跳转授权页（response_type=code + state）；
/// 3. 收到回调 code（校验 state）后向 token_url 换取令牌；
/// 4. 令牌写入缓存并返回（由调用方写回 AuthSpec 持久化）。
pub async fn authorize(auth: &AuthSpec) -> Result<OAuth2Token, OAuth2Error> {
    authorize_inner(auth, |url| {
        webbrowser::open(url).map_err(|e| format!("打开系统浏览器失败：{e}"))
    })
    .await
}

/// authorize 的可测试实现：浏览器打开动作由 `open` 回调注入
/// （生产走 webbrowser；测试可改为将授权 URL 交给外部模拟浏览器）。
async fn authorize_inner(
    auth: &AuthSpec,
    open: impl Fn(&str) -> Result<(), String>,
) -> Result<OAuth2Token, OAuth2Error> {
    let fields = oauth2_fields(auth)?;
    if fields.auth_url.trim().is_empty() {
        return Err(OAuth2Error::Config("auth_url 未配置".to_string()));
    }

    let _guard = AUTHORIZE_LOCK
        .get_or_init(|| AsyncMutex::new(()))
        .lock()
        .await;

    // 1. 授权 URL。
    let state = uuid::Uuid::new_v4().to_string();
    let mut auth_url = Url::parse(&fields.auth_url)
        .map_err(|e| OAuth2Error::Config(format!("auth_url 无效：{e}")))?;
    {
        let mut q = auth_url.query_pairs_mut();
        q.append_pair("response_type", "code");
        q.append_pair("client_id", &fields.client_id);
        q.append_pair("redirect_uri", &fields.redirect_uri);
        q.append_pair("state", &state);
        if !fields.scope.trim().is_empty() {
            q.append_pair("scope", fields.scope.trim());
        }
    }

    // 2. 回调服务器（端口/路径来自 redirect_uri）。上一次授权会话可能尚未完全退出，短暂重试绑定。
    let (cb_host, cb_port, cb_path) = redirect_binding(&fields.redirect_uri)?;
    let listener = bind_callback_listener(&cb_host, cb_port).await?;
    let (code_tx, mut code_rx) = mpsc::unbounded_channel::<Result<String, String>>();
    let (shutdown_tx, shutdown_rx) = watch::channel(());
    let shutdown_tx_handler = shutdown_tx.clone();
    let app = Router::new().route(
        &cb_path,
        get(move |Query(params): Query<HashMap<String, String>>| {
            let code_tx = code_tx.clone();
            let shutdown_tx = shutdown_tx_handler.clone();
            let state_expected = state.clone();
            async move {
                let (channel, html) = match params.get("state") {
                    Some(s) if *s == state_expected => match params.get("code") {
                        Some(code) => {
                            let _ = code_tx.send(Ok(code.clone()));
                            (
                                "授权成功",
                                "授权成功，已获取授权码，可以关闭此页面并返回 Miyo。",
                            )
                        }
                        None => {
                            let _ = code_tx.send(Err("回调缺少 code 参数".to_string()));
                            ("授权失败", "回调缺少 code 参数，请重新授权。")
                        }
                    },
                    _ => {
                        let _ = code_tx.send(Err("state 校验失败（可能存在 CSRF 风险）".to_string()));
                        ("授权失败", "state 校验失败，请重新授权。")
                    }
                };
                let _ = shutdown_tx.send(());
                Html(format!(
                    "<html><head><meta charset=\"utf-8\"><style>\
                     body{{font-family:system-ui;display:flex;align-items:center;justify-content:center;height:90vh;\
                     background:#0f172a;color:#f9fafb}}\
                     .box{{text-align:center;padding:32px;border:1px solid #1f2937;border-radius:12px}}\
                     h2{{margin:0 0 8px}}</style></head>\
                     <body><div class=\"box\"><h2>{channel}</h2><p>{html}</p></div></body></html>"
                ))
            }
        }),
    );
    let mut server = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let mut rx = shutdown_rx;
                let _ = rx.changed().await;
            })
            .await
    });

    // 3. 打开浏览器（测试注入：把 auth_url 交给模拟浏览器）。
    if let Err(e) = open(auth_url.as_str()) {
        server.abort();
        return Err(OAuth2Error::Callback(e));
    }

    // 4. 等待回调（限时）。
    let code = match tokio::time::timeout(
        Duration::from_secs(crate::AUTHORIZE_TIMEOUT.num_seconds() as u64),
        code_rx.recv(),
    )
    .await
    {
        Err(_) => {
            server.abort();
            return Err(OAuth2Error::Callback(format!(
                "等待授权回调超时（{} 秒），请重试",
                crate::AUTHORIZE_TIMEOUT.num_seconds()
            )));
        }
        Ok(Some(Ok(code))) => code,
        Ok(Some(Err(msg))) => {
            server.abort();
            return Err(OAuth2Error::Callback(msg));
        }
        Ok(None) => {
            server.abort();
            return Err(OAuth2Error::Callback("授权流程意外中断".to_string()));
        }
    };

    // 5. 停止回调服务器（优雅，最多等 3 秒）。
    let _ = shutdown_tx.send(());
    tokio::select! {
        _ = &mut server => {}
        _ = tokio::time::sleep(Duration::from_secs(3)) => { server.abort(); }
    }

    // 6. 换取令牌。
    let token = exchange_code(&fields, &code).await?;
    cache::store_token(&fields.client_id, &fields.token_url, token.clone());
    Ok(token)
}

/// 绑定回调监听端口；会话切换可能残留占用，重试至多 ~2 秒。
async fn bind_callback_listener(
    host: &str,
    port: u16,
) -> Result<tokio::net::TcpListener, OAuth2Error> {
    let addr = format!("{host}:{port}");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        match tokio::net::TcpListener::bind(&addr).await {
            Ok(l) => return Ok(l),
            Err(e) if tokio::time::Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(120)).await;
                if e.kind() != std::io::ErrorKind::AddrInUse {
                    return Err(OAuth2Error::Callback(format!(
                        "回调端口 {port} 绑定失败：{e}"
                    )));
                }
            }
            Err(e) => {
                return Err(OAuth2Error::Callback(format!(
                    "回调端口 {port} 被占用：{e}。请关闭占用该端口的程序（代理软件如 \
                     ClashX/小火箭常占用 9090），或在认证配置中把「回调地址」改为 \
                     其他端口，如 http://127.0.0.1:9091/callback"
                )));
            }
        }
    }
}

/// 用授权码换取令牌：POST token_url（form：grant_type=authorization_code）。
async fn exchange_code(fields: &OAuth2Fields, code: &str) -> Result<OAuth2Token, OAuth2Error> {
    let resp = token_client()
        .post(&fields.token_url)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", fields.redirect_uri.as_str()),
            ("client_id", fields.client_id.as_str()),
            ("client_secret", fields.client_secret.as_str()),
        ])
        .send()
        .await
        .map_err(|e| OAuth2Error::Exchange(format!("换取令牌请求失败：{e}")))?;

    let status = resp.status().as_u16();
    let text = resp.text().await.unwrap_or_else(|_| String::new());
    if !(200..300).contains(&status) {
        return Err(OAuth2Error::Exchange(format!(
            "换取令牌失败（HTTP {status}）：{}",
            text.chars().take(200).collect::<String>()
        )));
    }
    parse_token_payload(&text).map_err(OAuth2Error::Exchange)
}

/// 解析 token 响应：兼容 JSON 与 form 编码两种格式；`expires_in` 兼容数字与字符串。
pub(crate) fn parse_token_payload(text: &str) -> Result<OAuth2Token, String> {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(text) {
        let access = value
            .get("access_token")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "token 响应缺少 access_token".to_string())?;
        let expires_in = value
            .get("expires_in")
            .and_then(|v| v.as_i64().or_else(|| v.as_str()?.parse::<i64>().ok()))
            .ok_or_else(|| "token 响应缺少 expires_in".to_string())?;
        return Ok(OAuth2Token {
            access_token: access.to_string(),
            token_type: value
                .get("token_type")
                .and_then(|v| v.as_str())
                .unwrap_or("Bearer")
                .to_string(),
            refresh_token: value
                .get("refresh_token")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            expires_at: chrono::Utc::now() + chrono::Duration::seconds(expires_in),
        });
    }

    // form 编码兜底（部分服务商返回 application/x-www-form-urlencoded）。
    let form: HashMap<String, String> = url::form_urlencoded::parse(text.as_bytes())
        .into_owned()
        .collect();
    if form.is_empty() {
        return Err("token 响应不是合法 JSON 或表单".to_string());
    }
    let access = form
        .get("access_token")
        .ok_or_else(|| "token 响应缺少 access_token".to_string())?;
    let expires_in = form
        .get("expires_in")
        .ok_or_else(|| "token 响应缺少 expires_in".to_string())?
        .parse::<i64>()
        .map_err(|_| "expires_in 不是合法数字".to_string())?;
    Ok(OAuth2Token {
        access_token: access.clone(),
        token_type: form
            .get("token_type")
            .cloned()
            .unwrap_or_else(|| "Bearer".to_string()),
        refresh_token: form.get("refresh_token").cloned(),
        expires_at: chrono::Utc::now() + chrono::Duration::seconds(expires_in),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};
    use miyo_core::model::AuthSpec;

    /// 启动小运行时执行 async 测试代码（joined 环境受限，直接建 runtime）。
    fn block_on<T>(fut: impl std::future::Future<Output = T>) -> T {
        tokio::runtime::Runtime::new().unwrap().block_on(fut)
    }

    fn spec_auth(client_id: &str, token_url: &str) -> AuthSpec {
        AuthSpec::OAuth2 {
            client_id: client_id.into(),
            client_secret: "t-secret".into(),
            auth_url: "https://idp.test/authorize".into(),
            token_url: token_url.into(),
            scope: "read".into(),
            redirect_uri: crate::DEFAULT_REDIRECT_URI.to_string(),
            grant: OAuth2Grant::default(),
            username: None,
            password: None,
            token: None,
        }
    }

    fn valid_token(access: &str, expires_in: i64) -> OAuth2Token {
        OAuth2Token {
            access_token: access.into(),
            token_type: "Bearer".into(),
            refresh_token: Some("rt-1".into()),
            expires_at: Utc::now() + Duration::seconds(expires_in),
        }
    }

    #[test]
    fn parse_json_token_payload() {
        let t = parse_token_payload(
            r#"{"access_token":"at-9","token_type":"Bearer","refresh_token":"rt-9","expires_in":3600}"#,
        )
        .unwrap();
        assert_eq!(t.access_token, "at-9");
        assert_eq!(t.token_type, "Bearer");
        assert_eq!(t.refresh_token.as_deref(), Some("rt-9"));
        assert!(!t.is_expired());
        assert!(t.expires_within(Duration::hours(2)));
    }

    #[test]
    fn parse_json_with_string_expires_in_and_missing_type() {
        let t = parse_token_payload(r#"{"access_token":"at","expires_in":"60"}"#).unwrap();
        assert_eq!(t.token_type, "Bearer");
        assert!(t.expires_within(Duration::minutes(2)));
    }

    #[test]
    fn parse_form_encoded_token_payload() {
        let t = parse_token_payload(
            "access_token=at-form&expires_in=1800&refresh_token=rt-form&token_type=bearer",
        )
        .unwrap();
        assert_eq!(t.access_token, "at-form");
        assert_eq!(t.token_type, "bearer");
        assert_eq!(t.refresh_token.as_deref(), Some("rt-form"));
        assert!(!t.is_expired());
    }

    #[test]
    fn parse_rejects_garbage() {
        assert!(parse_token_payload("<html>error</html>").is_err());
        assert!(parse_token_payload(r#"{"foo":1}"#).is_err());
    }

    #[test]
    fn config_validation_rejects_non_oauth2() {
        let _guard = crate::cache::test_lock().lock().unwrap();
        crate::cache::reset_cache();
        let auth = AuthSpec::Bearer { token: "x".into() };
        let err = block_on(access_token_for(&auth)).unwrap_err();
        assert!(err.to_string().contains("不是 OAuth2"));
    }

    #[test]
    fn missing_credentials_reports_config_error() {
        let _guard = crate::cache::test_lock().lock().unwrap();
        crate::cache::reset_cache();
        let err = block_on(access_token_for(&spec_auth("", ""))).unwrap_err();
        assert!(err.to_string().contains("client_id"));
    }

    #[test]
    fn valid_cached_token_returned_without_network() {
        let _guard = crate::cache::test_lock().lock().unwrap();
        crate::cache::reset_cache();
        let token_url = "http://127.0.0.1:1/token-cached";
        crate::cache::store_token("c-valid", token_url, valid_token("at-valid", 3600));
        let auth = spec_auth("c-valid", token_url);
        assert_eq!(block_on(access_token_for(&auth)).unwrap(), "at-valid");
        crate::cache::reset_cache();
    }

    #[test]
    fn spec_token_seeds_cache_and_is_used() {
        let _guard = crate::cache::test_lock().lock().unwrap();
        crate::cache::reset_cache();
        let token_url = "http://127.0.0.1:1/token-seed";
        let mut auth = spec_auth("c-seed", token_url);
        if let AuthSpec::OAuth2 { token, .. } = &mut auth {
            *token = Some(valid_token("at-seed", 3600));
        }
        // token_url 端口 1 → 连接必拒；能拿到值说明走了种子缓存而非网络。
        assert_eq!(block_on(access_token_for(&auth)).unwrap(), "at-seed");
        crate::cache::reset_cache();
    }

    #[test]
    fn unauthorized_without_any_token() {
        let _guard = crate::cache::test_lock().lock().unwrap();
        crate::cache::reset_cache();
        let err = block_on(access_token_for(&spec_auth(
            "c-none",
            "http://127.0.0.1:1/token-none",
        )))
        .unwrap_err();
        assert!(err.to_string().contains("尚未完成"));
        crate::cache::reset_cache();
    }

    /// 无 refresh_token 且已过期 → 明确提示重新授权。
    #[test]
    fn expired_token_without_refresh_reports_reauthorize() {
        let _guard = crate::cache::test_lock().lock().unwrap();
        crate::cache::reset_cache();
        let token_url = "http://127.0.0.1:1/token-noref";
        let mut auth = spec_auth("c-noref", token_url);
        if let AuthSpec::OAuth2 { token, .. } = &mut auth {
            *token = Some(OAuth2Token {
                access_token: "at-dead".into(),
                token_type: "Bearer".into(),
                refresh_token: None,
                expires_at: Utc::now() - Duration::seconds(10),
            });
        }
        let err = block_on(access_token_for(&auth)).unwrap_err();
        assert!(err.to_string().contains("refresh_token"), "{err}");
        crate::cache::reset_cache();
    }

    // ---- 端到端：本地模拟 IdP + 真实回调端口 9090 ----

    /// e2e 测试串行锁：共享 9090 回调端口与全局缓存，不能并行。
    static E2E_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    fn e2e_lock() -> &'static Mutex<()> {
        E2E_LOCK.get_or_init(|| Mutex::new(()))
    }

    /// 本地模拟授权服务器：/authorize 302 到回调、/token 按 grant_type 发令牌，
    /// 并把收到的请求记录到共享日志供断言。
    async fn mock_idp(
        log: Arc<Mutex<Vec<(String, String)>>>,
    ) -> (String, tokio::task::JoinHandle<()>) {
        use axum::extract::Form;
        use axum::http::{header, StatusCode, Uri};
        use axum::response::IntoResponse;
        use axum::routing::post;
        use axum::Json;

        let log_auth = log.clone();
        let log_token = log.clone();
        let app = Router::new()
            .route(
                "/authorize",
                get(move |Query(q): Query<HashMap<String, String>>| {
                    let log = log_auth.clone();
                    async move {
                        log.lock()
                            .unwrap()
                            .push(("authorize".into(), format!("{q:?}")));
                        let mut loc = q
                            .get("redirect_uri")
                            .cloned()
                            .unwrap_or_else(|| crate::DEFAULT_REDIRECT_URI.to_string());
                        loc.push_str("?code=mock-code-1");
                        if let Some(s) = q.get("state") {
                            loc.push_str(&format!("&state={s}"));
                        }
                        (
                            StatusCode::FOUND,
                            [(header::LOCATION, Uri::try_from(loc).unwrap().to_string())],
                            (),
                        )
                            .into_response()
                    }
                }),
            )
            .route(
                "/token",
                post(move |Form(f): Form<HashMap<String, String>>| {
                    let log = log_token.clone();
                    async move {
                        let grant = f.get("grant_type").cloned().unwrap_or_default();
                        log.lock().unwrap().push((
                            "token".into(),
                            format!(
                                "grant_type={grant} code={:?} client_id={:?}",
                                f.get("code"),
                                f.get("client_id")
                            ),
                        ));
                        let access = if grant == "refresh_token" {
                            "at-refresh-1"
                        } else {
                            "at-code-1"
                        };
                        Json(serde_json::json!({
                            "access_token": access,
                            "token_type": "Bearer",
                            "refresh_token": "rt-1",
                            "expires_in": 3600,
                        }))
                    }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{addr}"), handle)
    }

    /// 挑选可用回调地址：9090 被占用（如代理软件）时退回自定义端口 9091，
    /// 同时覆盖「自定义 redirect_uri」功能路径。
    async fn pick_redirect_uri() -> String {
        if tokio::net::TcpListener::bind("127.0.0.1:9090")
            .await
            .is_ok()
        {
            crate::DEFAULT_REDIRECT_URI.to_string()
        } else {
            eprintln!("注意：127.0.0.1:9090 被占用，e2e 改用 127.0.0.1:9091 验证自定义回调地址");
            "http://127.0.0.1:9091/custom-cb".to_string()
        }
    }

    /// 授权码流端到端：模拟浏览器访问 auth_url → 302 → 本地回调 → 换 token。
    #[test]
    fn e2e_authorization_code_flow() {
        let _e2e = e2e_lock().lock().unwrap();
        let _cache = crate::cache::test_lock().lock().unwrap();
        crate::cache::reset_cache();
        block_on(async {
            let redirect_uri = pick_redirect_uri().await;
            let log = Arc::new(Mutex::new(Vec::new()));
            let (idp_url, _idp) = mock_idp(log.clone()).await;

            let mut auth = spec_auth("c-e2e", &format!("{idp_url}/token"));
            if let AuthSpec::OAuth2 {
                auth_url,
                redirect_uri: ru,
                ..
            } = &mut auth
            {
                *auth_url = format!("{idp_url}/authorize");
                *ru = redirect_uri.clone();
            }

            let (url_tx, url_rx) = std::sync::mpsc::channel();
            let auth2 = auth.clone();
            let task = tokio::spawn(async move {
                let tx = url_tx;
                authorize_inner(&auth2, move |url| {
                    let _ = tx.send(url.to_string());
                    Ok(())
                })
                .await
            });

            // 模拟浏览器：跟随 mock IdP 的 302 直到本地回调服务器。
            let auth_url = url_rx
                .recv_timeout(std::time::Duration::from_secs(5))
                .expect("授权流程应把 auth_url 交给浏览器");
            let resp = reqwest::get(&auth_url).await.unwrap();
            assert!(resp.status().is_success(), "回调应返回成功页");

            let token = task.await.unwrap().expect("授权流程应成功");
            assert_eq!(token.access_token, "at-code-1");
            assert_eq!(token.refresh_token.as_deref(), Some("rt-1"));
            assert!(!token.is_expired());

            // IdP 收到：authorize(带 response_type/client_id/redirect_uri/state) 与
            // authorization_code 换 token。
            {
                let log = log.lock().unwrap();
                let auth_line = log
                    .iter()
                    .find(|(k, _)| k == "authorize")
                    .map(|(_, v)| v.clone())
                    .unwrap();
                for expected in [
                    "response_type\": \"code\"",
                    "client_id\": \"c-e2e\"",
                    &format!("redirect_uri\": \"{redirect_uri}\""),
                    "state\": \"",
                    "scope\": \"read\"",
                ] {
                    assert!(
                        auth_line.contains(expected),
                        "authorize 参数缺失 {expected}: {auth_line}"
                    );
                }
                let token_line = log
                    .iter()
                    .find(|(k, _)| k == "token")
                    .map(|(_, v)| v.clone())
                    .unwrap();
                assert!(
                    token_line.contains("grant_type=authorization_code"),
                    "{token_line}"
                );
                assert!(
                    token_line.contains("code=Some(\"mock-code-1\")"),
                    "{token_line}"
                );
                assert!(
                    token_line.contains("client_id=Some(\"c-e2e\")"),
                    "{token_line}"
                );
            }
            // 令牌应已入缓存，后续 access_token_for 直接命中。
            assert_eq!(access_token_for(&auth).await.unwrap(), "at-code-1");
        });
    }

    /// 刷新流端到端：token 临近过期 → access_token_for 静默刷新。
    #[test]
    fn e2e_refresh_token_flow() {
        let _e2e = e2e_lock().lock().unwrap();
        let _cache = crate::cache::test_lock().lock().unwrap();
        crate::cache::reset_cache();
        block_on(async {
            let log = Arc::new(Mutex::new(Vec::new()));
            let (idp_url, _idp) = mock_idp(log.clone()).await;

            let mut auth = spec_auth("c-e2e-r", &format!("{idp_url}/token"));
            if let AuthSpec::OAuth2 { auth_url, .. } = &mut auth {
                *auth_url = format!("{idp_url}/authorize");
            }

            // 种入临近过期的令牌（expires_in=30 < 60s 提前刷新阈值）。
            crate::cache::store_token(
                "c-e2e-r",
                &format!("{idp_url}/token"),
                OAuth2Token {
                    access_token: "at-old".into(),
                    token_type: "Bearer".into(),
                    refresh_token: Some("rt-1".into()),
                    expires_at: Utc::now() + Duration::seconds(30),
                },
            );

            // 触发静默刷新 → 拿新令牌。
            let fresh = access_token_for(&auth).await.unwrap();
            assert_eq!(fresh, "at-refresh-1");
            let cached =
                crate::cache::cached_token("c-e2e-r", &format!("{idp_url}/token")).unwrap();
            assert_eq!(cached.access_token, "at-refresh-1");
            assert!(
                cached.expires_at > Utc::now() + Duration::minutes(50),
                "新令牌应获得 mock 的 1 小时寿命"
            );

            let log = log.lock().unwrap();
            assert!(
                log.iter()
                    .any(|(k, v)| k == "token" && v.contains("grant_type=refresh_token")),
                "应发 refresh_token 请求: {log:?}"
            );
        });
    }
}
