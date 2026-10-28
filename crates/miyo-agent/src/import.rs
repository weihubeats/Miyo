//! cURL → Endpoint 的转换逻辑。
//!
//! 与前端「导入为草稿」流程（`openCurlDraft`）对齐：
//! - URL 拆成 origin + path + query 参数，path 落库（校验要求以 `/` 开头）；
//! - origin 写入项目变量 `base_url`（仅当项目尚无该变量时，不覆盖用户配置）；
//! - 状态置为 `designing`（草稿语义），名称缺省从路径末段推导。

use miyo_core::curl_parser::CurlParsed;
use miyo_core::error::AppError;
use miyo_core::model::{BodySpec, Endpoint, EndpointStatus, KeyValue, RequestSpec};
use url::Url;

/// 把导入 URL 拆为 `(origin, path, query 参数)`；无 scheme 时按 https 补全
/// （与前端 `splitUrl` 行为一致）。
pub fn split_url(raw: &str) -> Result<(String, String, Vec<KeyValue>), AppError> {
    let trimmed = raw.trim();
    let with_scheme = if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        format!("https://{trimmed}")
    };
    let parsed =
        Url::parse(&with_scheme).map_err(|e| AppError::Validation(format!("URL 无法解析：{e}")))?;
    let origin = parsed.origin().ascii_serialization();
    let path = if parsed.path().is_empty() {
        "/".to_string()
    } else {
        parsed.path().to_string()
    };
    let mut params = Vec::new();
    for (key, value) in parsed.query_pairs() {
        params.push(KeyValue::new(key, value));
    }
    Ok((origin, path, params))
}

/// 从 URL 推导接口名：取路径最后一个非空段；无段时回退主机名。
pub fn derive_name(origin: &str, path: &str) -> String {
    path.split('/')
        .rfind(|s| !s.is_empty())
        .map(|s| s.to_string())
        .or_else(|| {
            let candidate = if origin.contains("://") {
                origin
            } else {
                &format!("https://{origin}")
            };
            Url::parse(candidate)
                .ok()
                .and_then(|u| u.host_str().map(|h| h.to_string()))
        })
        .unwrap_or_else(|| "Agent 导入".to_string())
}

/// 由解析结果构造 `Endpoint`（未落库）。
///
/// `name` 为空时自动推导；query 参数进入 `request.params`，
/// header / body / auth 原样映射到 `RequestSpec`。
pub fn endpoint_from_curl(
    parsed: CurlParsed,
    project_id: uuid::Uuid,
    folder_id: Option<uuid::Uuid>,
    name: Option<String>,
) -> Result<(Endpoint, String), AppError> {
    let (origin, path, params) = split_url(&parsed.url)?;
    let name = match name {
        Some(n) if !n.trim().is_empty() => n.trim().to_string(),
        _ => derive_name(&origin, &path),
    };
    let now = chrono::Utc::now();
    let endpoint = Endpoint {
        id: uuid::Uuid::new_v4(),
        project_id,
        folder_id,
        name,
        method: parsed.method,
        path,
        description: String::new(),
        status: EndpointStatus::Designing,
        sort_order: 0,
        request: RequestSpec {
            params,
            headers: parsed.headers,
            path_variables: Vec::new(),
            auth: parsed.auth,
            body: parsed.body.unwrap_or(BodySpec::None),
            active_tab: None,
            timeout_ms: None,
            follow_redirects: true,
            tests: None,
            disable_cookies: false,
            body_docs: std::collections::HashMap::new(),
            scripts: None,
        },
        created_at: now,
        updated_at: now,
    };
    Ok((endpoint, origin))
}

#[cfg(test)]
mod tests {
    use super::*;
    use miyo_core::model::{AuthSpec, HttpMethod};

    #[test]
    fn split_url_extracts_origin_path_and_query() {
        let (origin, path, params) =
            split_url("https://api.example.com/posts?userId=1&page=2").unwrap();
        assert_eq!(origin, "https://api.example.com");
        assert_eq!(path, "/posts");
        assert_eq!(params.len(), 2);
        assert_eq!(params[0].key, "userId");
        assert_eq!(params[1].value, "2");
    }

    #[test]
    fn split_url_fills_https_and_root_path() {
        let (origin, path, _) = split_url("api.example.com").unwrap();
        assert_eq!(origin, "https://api.example.com");
        assert_eq!(path, "/");
    }

    #[test]
    fn derive_name_prefers_last_segment_then_host() {
        assert_eq!(derive_name("https://a.com", "/users/42"), "42");
        assert_eq!(derive_name("https://a.com", "/"), "a.com");
    }

    #[test]
    fn endpoint_from_curl_maps_all_fields() {
        let parsed = miyo_core::parse_curl(
            r#"curl -X POST -H "Content-Type: application/json" -d '{"a":1}' https://api.example.com/orders?x=1"#,
        )
        .unwrap();
        let pid = uuid::Uuid::new_v4();
        let (ep, origin) = endpoint_from_curl(parsed, pid, None, Some("创建订单".into())).unwrap();
        assert_eq!(ep.project_id, pid);
        assert_eq!(ep.name, "创建订单");
        assert_eq!(ep.method, HttpMethod::POST);
        assert_eq!(ep.path, "/orders");
        assert_eq!(ep.status, EndpointStatus::Designing);
        assert_eq!(ep.request.params.len(), 1);
        assert_eq!(ep.request.headers[0].key, "Content-Type");
        assert!(matches!(ep.request.body, BodySpec::Json { .. }));
        assert_eq!(ep.request.auth, AuthSpec::None);
        assert_eq!(origin, "https://api.example.com");
    }
}
