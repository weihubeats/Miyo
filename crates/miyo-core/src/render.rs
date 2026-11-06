//! 请求规格变量渲染：把 `{{var}}` 替换进 URL / 规格的每个角落。
//!
//! 自 miyo-tauri 下沉至此（场景链路需要在 miyo-test 执行期二次渲染）：
//! 纯模型层操作，只依赖 `model` 与 `variable`。

use crate::model::{
    AuthSpec, BodySpec, DynamicSignatureConfig, KeyValue, MultipartField, RequestSpec,
};
use crate::variable::VariableMap;

/// 渲染请求规格中的全部变量（key/value、认证、body）。
pub fn render_spec(spec: &RequestSpec, vars: &VariableMap) -> RequestSpec {
    RequestSpec {
        params: render_kv(&spec.params, vars),
        headers: render_kv(&spec.headers, vars),
        path_variables: render_kv(&spec.path_variables, vars),
        auth: match &spec.auth {
            AuthSpec::None => AuthSpec::None,
            AuthSpec::Bearer { token } => AuthSpec::Bearer {
                token: crate::resolve_variables(token, vars),
            },
            AuthSpec::Basic { username, password } => AuthSpec::Basic {
                username: crate::resolve_variables(username, vars),
                password: crate::resolve_variables(password, vars),
            },
            AuthSpec::ApiKey {
                key,
                value,
                location,
            } => AuthSpec::ApiKey {
                key: crate::resolve_variables(key, vars),
                value: crate::resolve_variables(value, vars),
                location: *location,
            },
            AuthSpec::OAuth2 {
                client_id,
                client_secret,
                auth_url,
                token_url,
                scope,
                redirect_uri,
                grant,
                username,
                password,
                token,
                use_pkce,
            } => AuthSpec::OAuth2 {
                client_id: crate::resolve_variables(client_id, vars),
                client_secret: crate::resolve_variables(client_secret, vars),
                auth_url: crate::resolve_variables(auth_url, vars),
                token_url: crate::resolve_variables(token_url, vars),
                scope: crate::resolve_variables(scope, vars),
                redirect_uri: crate::resolve_variables(redirect_uri, vars),
                grant: *grant,
                username: username.clone(),
                password: password.clone(),
                token: token.clone(),
                use_pkce: *use_pkce,
            },
            AuthSpec::Digest { username, password } => AuthSpec::Digest {
                username: crate::resolve_variables(username, vars),
                password: crate::resolve_variables(password, vars),
            },
            AuthSpec::Hawk { key_id, key } => AuthSpec::Hawk {
                key_id: crate::resolve_variables(key_id, vars),
                key: crate::resolve_variables(key, vars),
            },
            AuthSpec::AwsV4 {
                access_key,
                secret_key,
                region,
                service,
                session_token,
            } => AuthSpec::AwsV4 {
                access_key: crate::resolve_variables(access_key, vars),
                secret_key: crate::resolve_variables(secret_key, vars),
                region: crate::resolve_variables(region, vars),
                service: crate::resolve_variables(service, vars),
                session_token: session_token
                    .as_deref()
                    .map(|t| crate::resolve_variables(t, vars)),
            },
            AuthSpec::Hmac {
                access_key,
                secret_key,
            } => AuthSpec::Hmac {
                access_key: crate::resolve_variables(access_key, vars),
                secret_key: crate::resolve_variables(secret_key, vars),
            },
            // 动态签名：Key/Secret 支持 {{变量}} 插值（如 {{$app_key}}）；
            // 模板本身也参与渲染，便于把环境变量拼进载荷。时间戳在发送时生成。
            AuthSpec::DynamicSignature { config } => AuthSpec::DynamicSignature {
                config: DynamicSignatureConfig {
                    app_key: crate::resolve_variables(&config.app_key, vars),
                    app_secret: crate::resolve_variables(&config.app_secret, vars),
                    key_header: crate::resolve_variables(&config.key_header, vars),
                    timestamp_header: crate::resolve_variables(&config.timestamp_header, vars),
                    sig_header: crate::resolve_variables(&config.sig_header, vars),
                    algorithm: config.algorithm,
                    encoding: config.encoding,
                    payload_template: crate::resolve_variables(&config.payload_template, vars),
                },
            },
        },
        body: match &spec.body {
            BodySpec::None => BodySpec::None,
            BodySpec::Json { raw } => BodySpec::Json {
                raw: crate::resolve_variables(raw, vars),
            },
            BodySpec::Text { raw } => BodySpec::Text {
                raw: crate::resolve_variables(raw, vars),
            },
            BodySpec::UrlEncoded { fields } => BodySpec::UrlEncoded {
                fields: render_kv(fields, vars),
            },
            BodySpec::Multipart { fields } => BodySpec::Multipart {
                fields: fields
                    .iter()
                    .map(|f| MultipartField {
                        key: crate::resolve_variables(&f.key, vars),
                        value_type: f.value_type,
                        value: crate::resolve_variables(&f.value, vars),
                        enabled: f.enabled,
                    })
                    .collect(),
            },
            BodySpec::GraphQL { spec } => BodySpec::GraphQL {
                spec: crate::model::GraphQLSpec {
                    query: crate::resolve_variables(&spec.query, vars),
                    variables: crate::resolve_variables(&spec.variables, vars),
                    operation_name: crate::resolve_variables(&spec.operation_name, vars),
                },
            },
            // 文件路径同样支持 {{变量}}（与 multipart 文件字段一致）。
            BodySpec::Binary { path } => BodySpec::Binary {
                path: crate::resolve_variables(path, vars),
            },
            BodySpec::Grpc { spec } => BodySpec::Grpc {
                spec: crate::model::GrpcSpec {
                    service: crate::resolve_variables(&spec.service, vars),
                    method: crate::resolve_variables(&spec.method, vars),
                    message: crate::resolve_variables(&spec.message, vars),
                    use_tls: spec.use_tls,
                    proto_ids: spec.proto_ids.clone(),
                },
            },
        },
        active_tab: spec.active_tab.clone(),
        timeout_ms: spec.timeout_ms,
        follow_redirects: spec.follow_redirects,
        tests: spec.tests.clone(),
        disable_cookies: spec.disable_cookies,
        body_docs: spec.body_docs.clone(),
        scripts: spec.scripts.clone(),
    }
}

/// 渲染键值对列表（Query / Header / Path 变量）。
pub fn render_kv(items: &[KeyValue], vars: &VariableMap) -> Vec<KeyValue> {
    items
        .iter()
        .map(|kv| KeyValue {
            key: crate::resolve_variables(&kv.key, vars),
            value: crate::resolve_variables(&kv.value, vars),
            enabled: kv.enabled,
            description: kv.description.clone(),
            // 设计元数据（类型/必填/示例）随环境变量渲染原样保留。
            field_type: kv.field_type,
            required: kv.required,
            example: kv.example.clone(),
        })
        .collect()
}

/// 渲染请求规格（JSON Body 类型保留版，场景执行链路专用）。
///
/// 与 [`render_spec`] 同一口径，差别仅在 JSON Body：模板里整值恰为
/// `{{name}}` 的字段，若 `typed` 中有该变量的非字符串 JSON 值（提取时
/// 保留的数字 / 布尔 / 数组 / 对象），整体替换为该类型值——上一步提取的
/// 数字进下一步 Body 仍是数字，不被字符串化。其余位置（URL / 头 / 表单）
/// 与混合插值（`"prefix-{{x}}"`）一律字符串解析；模板非法 JSON 时整体
/// 回退 [`render_spec`] 行为。
pub fn render_spec_typed(
    spec: &RequestSpec,
    vars: &VariableMap,
    typed: &std::collections::HashMap<String, serde_json::Value>,
) -> RequestSpec {
    let mut out = render_spec(spec, vars);
    if typed.is_empty() {
        return out;
    }
    if let BodySpec::Json { raw } = &spec.body {
        if let Ok(template) = serde_json::from_str::<serde_json::Value>(raw) {
            out.body = BodySpec::Json {
                raw: render_json_value(&template, vars, typed).to_string(),
            };
        }
    }
    out
}

/// 递归渲染 JSON 值树：整值占位走 typed 替换，其余字符串走变量解析。
fn render_json_value(
    v: &serde_json::Value,
    vars: &VariableMap,
    typed: &std::collections::HashMap<String, serde_json::Value>,
) -> serde_json::Value {
    match v {
        serde_json::Value::String(s) => {
            let trimmed = s.trim();
            if trimmed.len() > 4 && trimmed.starts_with("{{") && trimmed.ends_with("}}") {
                let key = &trimmed[2..trimmed.len() - 2];
                if let Some(t) = typed.get(key.trim()) {
                    return t.clone();
                }
            }
            serde_json::Value::String(crate::resolve_variables(s, vars))
        }
        serde_json::Value::Array(items) => serde_json::Value::Array(
            items
                .iter()
                .map(|x| render_json_value(x, vars, typed))
                .collect(),
        ),
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.iter()
                .map(|(k, x)| {
                    (
                        crate::resolve_variables(k, vars),
                        render_json_value(x, vars, typed),
                    )
                })
                .collect(),
        ),
        other => other.clone(),
    }
}

/// 全局参数注入：将启用的全局参数并入请求规格（query → params，header → headers）。
///
/// 规则：请求自身已存在的同名键优先（全局参数只做补缺，不覆盖显式配置）。
/// key 大小写不敏感比较（Header 常见 `x-request-id` / `X-Request-Id` 混写）。
///
/// 自 miyo-tauri 下沉至此（Agent 控制面的执行类路由与 Tauri 命令共用同一
/// 物化口径）：纯模型层操作，只依赖 `model` 与 `variable`。
pub fn apply_global_params(
    spec: &mut RequestSpec,
    params: &[crate::model::GlobalParam],
    vars: &VariableMap,
) {
    use crate::model::GlobalParamLocation;

    for gp in params.iter().filter(|p| p.enabled) {
        if gp.key.trim().is_empty() {
            continue;
        }
        let key_lower = gp.key.trim().to_ascii_lowercase();
        let value = crate::resolve_variables(&gp.value, vars);
        match gp.location {
            GlobalParamLocation::Query => {
                let exists = spec
                    .params
                    .iter()
                    .any(|kv| kv.enabled && kv.key.trim().to_ascii_lowercase() == key_lower);
                if !exists {
                    let mut kv = KeyValue::new(gp.key.trim(), value);
                    kv.description = "全局参数".into();
                    spec.params.push(kv);
                }
            }
            GlobalParamLocation::Header => {
                let exists = spec
                    .headers
                    .iter()
                    .any(|kv| kv.enabled && kv.key.trim().to_ascii_lowercase() == key_lower);
                if !exists {
                    let mut kv = KeyValue::new(gp.key.trim(), value);
                    kv.description = "全局参数".into();
                    spec.headers.push(kv);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::HttpMethod;
    use std::collections::HashMap;

    #[test]
    fn renders_url_headers_and_body() {
        let mut vars = HashMap::new();
        vars.insert("base_url".to_string(), "https://api.test".to_string());
        vars.insert("token".to_string(), "t-9".to_string());
        let spec = RequestSpec {
            headers: vec![KeyValue::new("Authorization", "Bearer {{token}}")],
            body: BodySpec::Json {
                raw: r#"{"name":"{{token}}"}"#.into(),
            },
            ..Default::default()
        };
        let rendered = render_spec(&spec, &vars);
        assert_eq!(rendered.headers[0].value, "Bearer t-9");
        assert_eq!(
            rendered.body,
            BodySpec::Json {
                raw: r#"{"name":"t-9"}"#.into()
            }
        );
        // 原规格不被修改（模板保留）。
        assert_eq!(spec.headers[0].value, "Bearer {{token}}");
    }

    #[test]
    fn grpc_and_binary_paths_render() {
        let mut vars = HashMap::new();
        vars.insert("svc".to_string(), "pkg.Svc".to_string());
        let spec = RequestSpec {
            body: BodySpec::Binary {
                path: "/data/{{svc}}.bin".into(),
            },
            ..Default::default()
        };
        assert_eq!(HttpMethod::GET.as_str(), "GET");
        let rendered = render_spec(&spec, &vars);
        match rendered.body {
            BodySpec::Binary { path } => assert_eq!(path, "/data/pkg.Svc.bin"),
            other => panic!("意外 body：{other:?}"),
        }
    }

    #[test]
    fn typed_render_preserves_json_types_in_body() {
        let vars = HashMap::new(); // 数值来自提取层（typed），字符串表为空
        let mut typed = HashMap::new();
        typed.insert("amount".to_string(), serde_json::json!(388.2));
        typed.insert("flag".to_string(), serde_json::json!(true));
        let spec = RequestSpec {
            body: BodySpec::Json {
                raw: r#"{"amount": "{{amount}}", "ok": "{{flag}}", "note": "合计-{{amount}}", "missing": "{{nope}}"}"#.into(),
            },
            ..Default::default()
        };
        let rendered = render_spec_typed(&spec, &vars, &typed);
        match rendered.body {
            BodySpec::Json { raw } => {
                let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
                assert_eq!(v["amount"], serde_json::json!(388.2)); // 数字，不是字符串
                assert_eq!(v["ok"], serde_json::json!(true));
                assert_eq!(v["note"], serde_json::json!("合计-{{amount}}")); // 混合插值：字符串表为空，原样保留
                assert_eq!(v["missing"], serde_json::json!("{{nope}}"));
            }
            other => panic!("意外 body：{other:?}"),
        }
    }

    #[test]
    fn typed_render_falls_back_on_invalid_json() {
        let mut typed = HashMap::new();
        typed.insert("x".to_string(), serde_json::json!(1));
        let spec = RequestSpec {
            body: BodySpec::Json {
                raw: "{not json {{x}}".into(),
            },
            ..Default::default()
        };
        let rendered = render_spec_typed(&spec, &HashMap::new(), &typed);
        match rendered.body {
            // 非法 JSON：退回字符串解析口径（未定义变量原样保留）。
            BodySpec::Json { raw } => assert_eq!(raw, "{not json {{x}}"),
            other => panic!("意外 body：{other:?}"),
        }
    }
}
