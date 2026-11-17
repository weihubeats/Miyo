//! Java Debug Runtime HTTP 客户端（无状态，按调用即连）。
//!
//! Runtime 是目标 JVM 内嵌的 HTTP 端点，HTTP 本身无连接状态——「连接/断开」
//! 在 Miyo 侧只是健康探测与状态展示，不做长连接生命周期管理，自然支持
//! 多项目多实例并发调用。

use std::time::Duration;

use miyo_core::AppError;
use serde_json::json;
use uuid::Uuid;

use crate::policy::{blacklisted, SecurityPolicy};
use crate::protocol::{
    build_method_id, validate_method_id, CancelResponse, InvocationStatus, InvokeRequest,
    InvokeResponse, JavaClassInfo, JavaMethodInfo, RuntimeInfo, HEADER_AUTHORIZATION,
    HEADER_PROTOCOL, PROTOCOL_VERSION,
};

/// 探测 / 发现类请求的响应超时（快进快出）。
const PROBE_TIMEOUT_MS: u64 = 10_000;
/// 握手失败时前端展示的统一前缀。
const RUNTIME_LABEL: &str = "Java Runtime";

/// Java Debug Runtime 客户端。
///
/// 克隆共享同一连接池；每个 Java 项目按配置构造自己的实例（endpoint + token）。
#[derive(Clone)]
pub struct JavaRuntimeClient {
    http: reqwest::Client,
    endpoint: String,
    token: Option<String>,
    policy: SecurityPolicy,
}

impl JavaRuntimeClient {
    /// 构造客户端。endpoint 归一化（去尾部斜杠）并校验 scheme。
    pub fn new(
        endpoint: &str,
        token: Option<&str>,
        policy: SecurityPolicy,
    ) -> Result<Self, AppError> {
        let trimmed = endpoint.trim().trim_end_matches('/');
        if trimmed.is_empty() {
            return Err(AppError::Validation("Runtime 地址不能为空".into()));
        }
        let lower = trimmed.to_ascii_lowercase();
        if !lower.starts_with("http://") && !lower.starts_with("https://") {
            return Err(AppError::Validation(format!(
                "Runtime 地址仅支持 http/https：{endpoint}"
            )));
        }
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_millis(5_000))
            .build()
            .map_err(|e| AppError::Connection(format!("HTTP 客户端构建失败：{e}")))?;
        Ok(JavaRuntimeClient {
            http,
            endpoint: trimmed.to_string(),
            token: token.map(str::to_owned).filter(|t| !t.trim().is_empty()),
            policy,
        })
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// 运行时信息（健康探测 + 握手校验协议版本）。
    pub async fn info(&self) -> Result<RuntimeInfo, AppError> {
        let info: RuntimeInfo = self
            .request_json("GET", "/miyo/debug/info", None, PROBE_TIMEOUT_MS)
            .await?;
        if info.protocol_version != PROTOCOL_VERSION {
            return Err(AppError::Connection(format!(
                "{RUNTIME_LABEL} 协议版本不匹配：Runtime 支持 v{}，Miyo 需要 v{PROTOCOL_VERSION}（请升级 SDK 或 Miyo）",
                info.protocol_version
            )));
        }
        Ok(info)
    }

    /// 白名单类清单。
    pub async fn classes(&self) -> Result<Vec<JavaClassInfo>, AppError> {
        self.request_json("GET", "/miyo/debug/classes", None, PROBE_TIMEOUT_MS)
            .await
    }

    /// 某类的白名单方法清单。
    pub async fn methods(&self, class_name: &str) -> Result<Vec<JavaMethodInfo>, AppError> {
        if blacklisted(class_name) {
            return Err(AppError::Forbidden(format!("{class_name} 属于受限类")));
        }
        let url = format!(
            "/miyo/debug/methods?class={}",
            urlencoding_quote(class_name)
        );
        self.request_json("GET", &url, None, PROBE_TIMEOUT_MS).await
    }

    /// 执行方法（同步等待结果）。
    ///
    /// 流程：method_id 校验 → 黑名单预检 → 参数体积预检 → 超时收敛 →
    /// POST invoke（请求超时 = 调用超时 + 网络余量）。
    pub async fn invoke(
        &self,
        method_id: &str,
        arguments: Vec<serde_json::Value>,
        timeout_ms: Option<u64>,
        invocation_id: Uuid,
    ) -> Result<InvokeResponse, AppError> {
        validate_method_id(method_id)?;
        self.policy.check_method_allowed(method_id)?;
        self.policy.validate_arguments(&arguments)?;
        let effective_timeout = self.policy.resolve_timeout_ms(timeout_ms)?;
        // 方法超时之外留网络传输余量；Runtime 侧到点先行返回 timeout 状态。
        let http_timeout = effective_timeout + 10_000;
        let req = InvokeRequest {
            invocation_id,
            method_id: method_id.to_string(),
            arguments,
            timeout_ms: Some(effective_timeout),
        };
        let mut resp: InvokeResponse = self
            .request_json(
                "POST",
                "/miyo/debug/invoke",
                Some(&json!(req)),
                http_timeout,
            )
            .await?;
        if resp.invocation_id == Uuid::nil() {
            resp.invocation_id = invocation_id;
        }
        if resp.method_id.is_empty() {
            resp.method_id = method_id.to_string();
        }
        if resp.result_truncated {
            // SDK 侧已截断；Miyo 侧策略更严时再截一遍（取更小口径）。
            if let Some(result) = resp.result.take() {
                let (clipped, clipped_flag) = self.policy.truncate_result(result);
                resp.result = Some(clipped);
                resp.result_truncated = resp.result_truncated || clipped_flag;
            }
        }
        Ok(resp)
    }

    /// 请求取消在途调用（协作式中断：不保证目标方法立即停止）。
    pub async fn cancel(&self, invocation_id: Uuid) -> Result<bool, AppError> {
        let resp: CancelResponse = self
            .request_json(
                "POST",
                &format!("/miyo/debug/invocations/{invocation_id}/cancel"),
                None,
                PROBE_TIMEOUT_MS,
            )
            .await?;
        Ok(resp.delivered)
    }

    /// 结果状态 → AppError 语义（SDK 返回非 Success 且调用方需要 Err 语义时用）。
    ///
    /// Failed / Timeout / Cancelled 都携带详情（异常 / 耗时），调用方通常
    /// 不转 Err 而是原样展示；RuntimeDisconnected 才是链路级失败。
    pub fn status_error(resp: &InvokeResponse) -> Option<AppError> {
        match resp.status {
            InvocationStatus::RuntimeDisconnected => Some(AppError::Connection(format!(
                "{RUNTIME_LABEL} 在调用完成前断开"
            ))),
            _ => None,
        }
    }

    /// 统一请求：鉴权头 + 协议版本头 + 错误归一。
    async fn request_json<T: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        path_and_query: &str,
        body: Option<&serde_json::Value>,
        timeout_ms: u64,
    ) -> Result<T, AppError> {
        let url = format!("{}{}", self.endpoint, path_and_query);
        let mut req = match method {
            "GET" => self.http.get(&url),
            "POST" => self.http.post(&url),
            _ => unreachable!("仅 GET / POST"),
        };
        if let Some(token) = &self.token {
            req = req.header(HEADER_AUTHORIZATION, format!("Bearer {token}"));
        }
        req = req
            .header(HEADER_PROTOCOL, PROTOCOL_VERSION.to_string())
            .timeout(Duration::from_millis(timeout_ms));
        if let Some(body) = body {
            req = req.json(body);
        }
        let resp = req.send().await.map_err(|e| {
            AppError::classify(&e)
                .unwrap_or_else(|| AppError::Connection(format!("{RUNTIME_LABEL} 请求失败：{e}")))
        })?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| AppError::Connection(format!("{RUNTIME_LABEL} 响应读取失败：{e}")))?;
        if !status.is_success() {
            return Err(runtime_error(status.as_u16(), &text));
        }
        serde_json::from_str(&text).map_err(|e| {
            AppError::Connection(format!("{RUNTIME_LABEL} 响应不是合法协议 JSON：{e}"))
        })
    }
}

/// HTTP 状态 + 错误体 → AppError（SDK 错误码映射，见 protocol::RuntimeErrorKind）。
fn runtime_error(status: u16, body: &str) -> AppError {
    use crate::protocol::RuntimeErrorKind::*;
    let message = extract_error_message(body);
    match crate::protocol::RuntimeErrorKind::parse(body) {
        NotAuthorized => {
            AppError::Forbidden(format!("{RUNTIME_LABEL} 鉴权失败：token 不匹配或缺失"))
        }
        NotRegistered => AppError::NotFound(format!("类或方法未在 Runtime 白名单注册：{message}")),
        NotAllowed => {
            AppError::Forbidden(format!("{RUNTIME_LABEL} 拒绝该调用（安全策略）：{message}"))
        }
        BadArguments => AppError::Validation(format!("参数错误：{message}")),
        BadRequest => AppError::Validation(format!("请求体非法：{message}")),
        RateLimited => {
            AppError::Forbidden(format!("{RUNTIME_LABEL} 触发频率 / 并发限制：{message}"))
        }
        UnknownInvocation => AppError::NotFound(format!("在途调用不存在或已结束：{message}")),
        Internal => AppError::Connection(format!("{RUNTIME_LABEL} 内部错误：{message}")),
        Other => match status {
            401 | 403 => AppError::Forbidden(format!("{RUNTIME_LABEL} 拒绝访问（HTTP {status}）")),
            404 => AppError::NotFound(format!(
                "{RUNTIME_LABEL} 路径不存在（HTTP 404）：请确认 SDK 端点已启用"
            )),
            400 => AppError::Validation(format!("请求被 Runtime 拒绝：{message}")),
            _ => AppError::Connection(format!("{RUNTIME_LABEL} 返回 HTTP {status}：{message}")),
        },
    }
}

/// 错误体提取 message（非法 JSON 原样截断展示）。
fn extract_error_message(body: &str) -> String {
    let parsed: Result<serde_json::Value, _> = serde_json::from_str(body);
    let msg = parsed
        .ok()
        .and_then(|v| v.get("error")?.get("message")?.as_str().map(str::to_owned))
        .unwrap_or_else(|| body.chars().take(200).collect());
    if msg.is_empty() {
        "（无详情）".to_string()
    } else {
        msg
    }
}

/// 极简 query 值编码（类名只含字母数字点下划线 `$`，其余按百分号编码）。
fn urlencoding_quote(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for b in value.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'.' | b'_' | b'-' | b'~' | b'$' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// 便捷构造：类 + 方法 + 参数类型表 → 校验过的 method_id。
pub fn make_method_id(
    class_name: &str,
    method_name: &str,
    param_types: &[String],
) -> Result<String, AppError> {
    let id = build_method_id(class_name, method_name, param_types);
    validate_method_id(&id)?;
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_normalization() {
        let c = JavaRuntimeClient::new("http://localhost:9918/", None, SecurityPolicy::default())
            .unwrap();
        assert_eq!(c.endpoint(), "http://localhost:9918");
    }

    #[test]
    fn endpoint_rejects_bad_scheme() {
        for bad in ["", "ftp://x", "localhost:9918"] {
            assert!(JavaRuntimeClient::new(bad, None, SecurityPolicy::default()).is_err());
        }
    }

    #[test]
    fn token_empty_treated_as_none() {
        let c = JavaRuntimeClient::new(
            "http://localhost:9918",
            Some("  "),
            SecurityPolicy::default(),
        )
        .unwrap();
        assert!(c.token.is_none());
    }

    #[test]
    fn make_method_id_validation() {
        let id = make_method_id("com.example.S", "m", &["String".into()]).unwrap();
        assert_eq!(id, "com.example.S#m(String)");
        assert!(make_method_id("S", "m", &[]).is_err());
    }

    #[test]
    fn query_encoding() {
        assert_eq!(
            urlencoding_quote("com.example.Order$Inner"),
            "com.example.Order$Inner"
        );
        assert_eq!(urlencoding_quote("com.a.B"), "com.a.B");
        assert_eq!(urlencoding_quote("com a/B"), "com%20a%2FB");
    }

    #[tokio::test]
    async fn invoke_rejects_blacklisted_before_network() {
        let c = JavaRuntimeClient::new(
            "http://127.0.0.1:1", // 不可达端口——若发起网络请求则测试失败
            None,
            SecurityPolicy::default(),
        )
        .unwrap();
        let err = c
            .invoke(
                "java.lang.Runtime#exec(String)",
                vec![json!("ls")],
                Some(1000),
                Uuid::new_v4(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Forbidden(_)));
    }

    #[tokio::test]
    async fn invoke_rejects_oversize_args_before_network() {
        let policy = SecurityPolicy {
            max_args_bytes: 16,
            ..Default::default()
        };
        let c = JavaRuntimeClient::new("http://127.0.0.1:1", None, policy).unwrap();
        let err = c
            .invoke(
                "com.example.S#m(String)",
                vec![json!("x".repeat(64))],
                Some(1000),
                Uuid::new_v4(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Validation(_)));
    }
}
