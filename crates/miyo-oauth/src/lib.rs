//! OAuth2 授权码流（Authorization Code Grant）实现。
//!
//! 职责：
//! - [`authorize`]：启动本地回调服务器（默认 127.0.0.1:9090）→ 打开系统浏览器
//!   跳转授权页 → 收到 code → 向 `token_url` 换取 access_token / refresh_token；
//!   回调端口与路径由 `AuthSpec.redirect_uri` 决定（默认 9090/callback，可自定义
//!   以避开端口占用，如代理软件常占 9090）；
//! - [`access_token_for`]：请求发送前取 access token，过期或即将过期时用
//!   refresh_token 静默刷新（缓存 + 每 key 互斥锁防并发重复刷新）；
//! - [`cached_token`] / [`clear_token`]：缓存读写，供 UI 状态展示与清除。
//!
//! token 缓存为进程内全局（key = client_id + token_url）；授权结果由调用方
//! 写回 `AuthSpec::OAuth2.token` 并持久化，刷新后的新 token 由调用方
//! （miyo-http 请求路径的宿主）回读缓存持久化。

pub mod cache;
pub mod client;
pub mod pkce;

pub use cache::{cached_token, clear_token, reset_cache, store_token};
pub use client::{access_token_for, authorize, OAuth2Error};

/// 本地回调默认监听地址与端口（redirect_uri 未指定时使用）。
pub const CALLBACK_HOST: &str = "127.0.0.1";
pub const CALLBACK_PORT: u16 = 9090;
/// 默认回调地址（须与授权服务注册的回调一致）。
pub const DEFAULT_REDIRECT_URI: &str = "http://127.0.0.1:9090/callback";

/// 距过期不足该时长即提前刷新（静默续期）。
pub const REFRESH_AHEAD: chrono::Duration = chrono::Duration::seconds(60);
/// 等待浏览器回调的最长时间。
pub const AUTHORIZE_TIMEOUT: chrono::Duration = chrono::Duration::seconds(120);
