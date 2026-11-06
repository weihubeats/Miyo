//! 快照脱敏：推送到共享远端（git / S3 / db）前，把敏感值替换为占位符。
//!
//! 威胁模型：远端对**全团队可见**（同事、CI、仓库读者）。任何"只属于某台机器
//! 或某个人"的凭证都不应离开本机——一旦推出去，撤销成本远高于本地重填。
//!
//! 策略：
//! - 推送前：命中敏感规则的值替换为 [`PLACEHOLDER`]，并登记到 [`RedactionLog`]；
//! - 拉取后：占位符不覆盖本地已有值（本地值为空时留占位符，提示用户补填）。
//!
//! 注意：这是**尽力而为**的启发式过滤，不是保证。命中规则见 [`is_sensitive_key`]。

use serde::{Deserialize, Serialize};

use miyo_backup::BackupFile;
use miyo_core::model::{AuthSpec, BodySpec, KeyValue, RequestSpec};

/// 敏感值占位符。
pub const PLACEHOLDER: &str = "__MIYO_REDACTED__";

/// 品牌更名前写入远端快照的旧占位符：拉取合并时同样识别，
/// 避免旧快照的占位值覆盖本地真实值。
pub const LEGACY_PLACEHOLDER: &str = "__RUSTFOX_REDACTED__";

/// 判断值是否为（当前或历史版本的）脱敏占位符。
pub fn is_placeholder(value: &str) -> bool {
    value == PLACEHOLDER || value == LEGACY_PLACEHOLDER
}

/// 脱敏日志：本次推送中被替换的字段（供前端展示"同步了什么被过滤"）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RedactionLog {
    /// 被脱敏的项，形如 `环境变量 prod_token`、`接口 GET /login 的请求头 Authorization`。
    pub entries: Vec<String>,
}

impl RedactionLog {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// 敏感键名启发式：命中即视为需要脱敏。
///
/// 覆盖常见凭证词根；`*_url` / `endpoint` 之类的地址键**不**命中
/// （团队内通常需要共享，且不是凭证）。
pub fn is_sensitive_key(key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    const TOKENS: &[&str] = &[
        "authorization",
        "cookie",
        "token",
        "secret",
        "password",
        "passwd",
        "pwd",
        "api-key",
        "apikey",
        "api_key",
        "access_key",
        "accesskey",
        "private",
        "credential",
        "sessionid",
        "session_id",
        "x-auth",
    ];
    TOKENS.iter().any(|t| k.contains(t))
}

/// 脱敏结果。
#[derive(Debug, Clone)]
pub struct RedactionReport {
    pub snapshot: BackupFile,
    pub log: RedactionLog,
}

/// 对快照做脱敏，返回脱敏后的副本与日志（原快照不变）。
pub fn redact_snapshot(snapshot: &BackupFile) -> RedactionReport {
    let mut out = snapshot.clone();
    let mut log = RedactionLog::default();

    redact_project(&mut out, &mut log);
    for endpoint in &mut out.endpoints {
        redact_request(&mut endpoint.request, &mut log, &endpoint.name);
    }
    for env in &mut out.environments {
        for var in &mut env.variables {
            if is_sensitive_key(&var.key) && !var.remote_value.is_empty() {
                var.remote_value = PLACEHOLDER.to_string();
                log.entries
                    .push(format!("环境 {} 的变量 {}", env.name, var.key));
            }
            // 本地私有覆盖是明确的"只在本机生效"语义，永不外发。
            if !var.local_value.is_empty() {
                var.local_value = String::new();
                log.entries
                    .push(format!("环境 {} 的本地覆盖 {}", env.name, var.key));
            }
        }
    }
    for mock in &mut out.mock_rules {
        let sensitive: Vec<String> = mock
            .response_headers
            .keys()
            .filter(|k| is_sensitive_key(k))
            .cloned()
            .collect();
        for key in sensitive {
            let changed = mock
                .response_headers
                .get(&key)
                .map(|v| !v.is_empty())
                .unwrap_or(false);
            if changed {
                mock.response_headers
                    .insert(key.clone(), PLACEHOLDER.to_string());
                log.entries
                    .push(format!("Mock 规则 {} 的响应头 {}", mock.path, key));
            }
        }
    }
    // 请求用例是完整请求快照（含 auth），与接口本体同口径脱敏。
    for example in &mut out.request_examples {
        redact_request(
            &mut example.request,
            &mut log,
            &format!("请求用例 {}", example.name),
        );
    }
    // 响应示例的响应头同样可能携带 Set-Cookie / 令牌。
    for example in &mut out.response_examples {
        let sensitive: Vec<String> = example
            .headers
            .keys()
            .filter(|k| is_sensitive_key(k))
            .cloned()
            .collect();
        for key in sensitive {
            let changed = example
                .headers
                .get(&key)
                .map(|v| !v.is_empty())
                .unwrap_or(false);
            if changed {
                example.headers.insert(key.clone(), PLACEHOLDER.to_string());
                log.entries
                    .push(format!("响应示例 {} 的响应头 {}", example.name, key));
            }
        }
    }
    // 场景 / 请求 / 响应示例的 body 模板与场景脚本属于自由文本，无法可靠自动
    // 判断是否内嵌凭据——登记为「需人工检查」项（明确提示，不做静默假设）。
    for scenario in &mut out.scenarios {
        if scenario
            .script
            .as_deref()
            .is_some_and(|s| !s.trim().is_empty())
        {
            log.entries.push(format!(
                "场景 {} 含自定义脚本：请人工确认未内嵌真实凭据",
                scenario.name
            ));
        }
    }
    for endpoint in &mut out.endpoints {
        if let Some(scripts) = &endpoint.request.scripts {
            if !scripts.pre_request.trim().is_empty() || !scripts.post_response.trim().is_empty() {
                log.entries.push(format!(
                    "接口 {} 含前置/后置脚本：请人工确认未内嵌真实凭据",
                    endpoint.name
                ));
            }
        }
    }
    report_unique(&mut log);
    RedactionReport { snapshot: out, log }
}

/// 拉取后还原：占位符不覆盖本地已有值。
///
/// - 本地值为空（或原本就是占位符）→ 保留占位符，提示用户补填；
/// - 本地值非空 → 用本地值覆盖占位符（凭证留在本机）。
pub fn restore_local_values(snapshot: &mut BackupFile, local: &BackupFile) {
    use std::collections::HashMap;
    let local_env: HashMap<&str, &miyo_core::model::Environment> = local
        .environments
        .iter()
        .map(|e| (e.name.as_str(), e))
        .collect();

    for env in &mut snapshot.environments {
        let Some(local_env) = local_env.get(env.name.as_str()) else {
            continue;
        };
        let local_vars: HashMap<&str, &miyo_core::model::EnvironmentVariable> = local_env
            .variables
            .iter()
            .map(|v| (v.key.as_str(), v))
            .collect();
        for var in &mut env.variables {
            let Some(local_var) = local_vars.get(var.key.as_str()) else {
                continue;
            };
            if is_placeholder(&var.remote_value) && !local_var.remote_value.is_empty() {
                var.remote_value = local_var.remote_value.clone();
            }
            var.local_value = local_var.local_value.clone();
        }
    }

    let local_endpoints: HashMap<(&str, &str), &miyo_core::model::Endpoint> = local
        .endpoints
        .iter()
        .map(|e| ((e.method.as_str(), e.path.as_str()), e))
        .collect();
    for endpoint in &mut snapshot.endpoints {
        let Some(local_ep) =
            local_endpoints.get(&(endpoint.method.as_str(), endpoint.path.as_str()))
        else {
            continue;
        };
        restore_request(&mut endpoint.request, &local_ep.request);
    }
}

/// 加密存储占位符？否——占位符本身无敏感信息，直接明文可读，
/// 便于同事看到"这里有个凭证需要自行填写"而不是一团密文。
fn redact_project(snapshot: &mut BackupFile, log: &mut RedactionLog) {
    let keys: Vec<String> = snapshot
        .project
        .variables
        .keys()
        .filter(|k| is_sensitive_key(k))
        .cloned()
        .collect();
    for key in keys {
        snapshot
            .project
            .variables
            .insert(key.clone(), PLACEHOLDER.into());
        log.entries.push(format!("项目变量 {key}"));
    }
}

fn redact_request(request: &mut RequestSpec, log: &mut RedactionLog, scope: &str) {
    redact_kv_list(&mut request.headers, log, scope, "请求头");
    redact_kv_list(&mut request.params, log, scope, "查询参数");
    redact_kv_list(&mut request.path_variables, log, scope, "路径变量");
    redact_auth(&mut request.auth, log, scope);
    redact_body(&mut request.body, log, scope);
}

/// 脱敏键值列表。**禁用项也脱敏**：禁用只是不发送，值仍随快照外发；
/// 禁用的历史 Token 泄出去同样是泄露。
fn redact_kv_list(items: &mut [KeyValue], log: &mut RedactionLog, scope: &str, label: &str) {
    for kv in items.iter_mut() {
        if kv.value.is_empty() {
            continue;
        }
        if is_sensitive_key(&kv.key) || looks_like_secret(&kv.value) {
            kv.value = PLACEHOLDER.to_string();
            let disabled = if kv.enabled { "" } else { "（已禁用）" };
            log.entries
                .push(format!("接口 {scope} 的{label} {}{disabled}", kv.key));
        }
    }
}

/// Body 内凭证脱敏：
///
/// - JSON：解析后对敏感键的字符串值替换占位符（解析失败按自由文本跳过，
///   不做整段替换以免毁掉正常文档）；
/// - UrlEncoded / Multipart：字段按键名启发式脱敏；
/// - Text / Binary / GraphQL / gRPC：自由内容无法可靠判断，交由
///   脚本/内容人工审查提示（日志中的「请人工确认」项）覆盖。
fn redact_body(body: &mut BodySpec, log: &mut RedactionLog, scope: &str) {
    match body {
        BodySpec::Json { raw } => {
            let Ok(mut value) = serde_json::from_str::<serde_json::Value>(raw) else {
                return;
            };
            let before = value.clone();
            redact_json_secrets(&mut value);
            if value != before {
                if let Ok(new_raw) = serde_json::to_string(&value) {
                    *raw = new_raw;
                    log.entries
                        .push(format!("接口 {scope} 的 JSON Body 敏感字段"));
                }
            }
        }
        BodySpec::UrlEncoded { fields } => {
            redact_kv_list(fields, log, scope, "表单字段");
        }
        BodySpec::Multipart { fields } => {
            for field in fields.iter_mut() {
                if field.value.is_empty() || !field.enabled {
                    continue;
                }
                // 文件类型字段的 value 是路径/文件引用，不是凭证，只处理文本字段。
                if matches!(field.value_type, miyo_core::model::MultipartValueType::Text)
                    && is_sensitive_key(&field.key)
                {
                    field.value = PLACEHOLDER.to_string();
                    log.entries
                        .push(format!("接口 {scope} 的 Multipart 字段 {}", field.key));
                }
            }
        }
        _ => {}
    }
}

/// 递归替换 JSON 中敏感键的字符串值（值形态如 JWT 也命中）。
fn redact_json_secrets(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, v) in map.iter_mut() {
                if is_sensitive_key(key) {
                    if let Some(s) = v.as_str() {
                        if !s.is_empty() && (is_sensitive_key(key) || looks_like_secret(s)) {
                            *v = serde_json::Value::String(PLACEHOLDER.to_string());
                            continue;
                        }
                    }
                }
                redact_json_secrets(v);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items.iter_mut() {
                redact_json_secrets(item);
            }
        }
        _ => {}
    }
}

fn redact_auth(auth: &mut AuthSpec, log: &mut RedactionLog, scope: &str) {
    match auth {
        AuthSpec::None => {}
        AuthSpec::Bearer { token } => {
            if !token.is_empty() {
                *token = PLACEHOLDER.into();
                log.entries.push(format!("接口 {scope} 的 Bearer Token"));
            }
        }
        AuthSpec::Basic { password, .. } => {
            if !password.is_empty() {
                *password = PLACEHOLDER.into();
                log.entries.push(format!("接口 {scope} 的 Basic 密码"));
            }
        }
        AuthSpec::ApiKey { value, key, .. } => {
            if !value.is_empty() {
                *value = PLACEHOLDER.into();
                log.entries
                    .push(format!("接口 {scope} 的 API Key（{key}）"));
            }
        }
        AuthSpec::DynamicSignature { config } => {
            if !config.app_secret.is_empty() {
                config.app_secret = PLACEHOLDER.into();
                log.entries.push(format!("接口 {scope} 的签名 Secret"));
            }
        }
        AuthSpec::OAuth2 {
            client_secret,
            token,
            ..
        } => {
            if !client_secret.is_empty() {
                *client_secret = PLACEHOLDER.into();
                log.entries
                    .push(format!("接口 {scope} 的 OAuth2 Client Secret"));
            }
            // 令牌带过期时间且属个人会话，绝不同步。
            if token.is_some() {
                *token = None;
                log.entries.push(format!("接口 {scope} 的 OAuth2 令牌"));
            }
        }
        AuthSpec::Digest { password, .. } => {
            if !password.is_empty() {
                *password = PLACEHOLDER.into();
                log.entries.push(format!("接口 {scope} 的 Digest 密码"));
            }
        }
        AuthSpec::Hawk { key, .. } => {
            if !key.is_empty() {
                *key = PLACEHOLDER.into();
                log.entries.push(format!("接口 {scope} 的 Hawk 密钥"));
            }
        }
        AuthSpec::AwsV4 {
            secret_key,
            session_token,
            ..
        } => {
            if !secret_key.is_empty() {
                *secret_key = PLACEHOLDER.into();
                log.entries.push(format!("接口 {scope} 的 AWS Secret Key"));
            }
            if session_token.is_some() {
                *session_token = Some(PLACEHOLDER.into());
                log.entries
                    .push(format!("接口 {scope} 的 AWS 临时会话令牌"));
            }
        }
        AuthSpec::Hmac { secret_key, .. } => {
            if !secret_key.is_empty() {
                *secret_key = PLACEHOLDER.into();
                log.entries.push(format!("接口 {scope} 的 HMAC Secret Key"));
            }
        }
    }
}

fn restore_request(target: &mut RequestSpec, local: &RequestSpec) {
    for kv in target.headers.iter_mut() {
        if kv.value != PLACEHOLDER {
            continue;
        }
        if let Some(local_kv) = local.headers.iter().find(|l| l.key == kv.key) {
            if !local_kv.value.is_empty() {
                kv.value = local_kv.value.clone();
            }
        }
    }
    // 查询参数同样可能被脱敏（token / sig 类键），占位符从本地还原。
    for kv in target.params.iter_mut() {
        if kv.value != PLACEHOLDER {
            continue;
        }
        if let Some(local_kv) = local.params.iter().find(|l| l.key == kv.key) {
            if !local_kv.value.is_empty() {
                kv.value = local_kv.value.clone();
            }
        }
    }
    match (&mut target.auth, &local.auth) {
        (AuthSpec::Bearer { token }, AuthSpec::Bearer { token: local_token }) => {
            if *token == PLACEHOLDER && !local_token.is_empty() {
                *token = local_token.clone();
            }
        }
        (
            AuthSpec::Basic { password, .. },
            AuthSpec::Basic {
                password: local_password,
                ..
            },
        ) => {
            if *password == PLACEHOLDER && !local_password.is_empty() {
                *password = local_password.clone();
            }
        }
        (
            AuthSpec::ApiKey { value, .. },
            AuthSpec::ApiKey {
                value: local_value, ..
            },
        ) => {
            if *value == PLACEHOLDER && !local_value.is_empty() {
                *value = local_value.clone();
            }
        }
        (
            AuthSpec::Digest { password, .. },
            AuthSpec::Digest {
                password: local_password,
                ..
            },
        ) => {
            if *password == PLACEHOLDER && !local_password.is_empty() {
                *password = local_password.clone();
            }
        }
        (AuthSpec::Hawk { key, .. }, AuthSpec::Hawk { key: local_key, .. }) => {
            if *key == PLACEHOLDER && !local_key.is_empty() {
                *key = local_key.clone();
            }
        }
        (
            AuthSpec::AwsV4 { secret_key, .. },
            AuthSpec::AwsV4 {
                secret_key: local_secret,
                ..
            },
        ) => {
            if *secret_key == PLACEHOLDER && !local_secret.is_empty() {
                *secret_key = local_secret.clone();
            }
        }
        (
            AuthSpec::Hmac { secret_key, .. },
            AuthSpec::Hmac {
                secret_key: local_secret,
                ..
            },
        ) if *secret_key == PLACEHOLDER && !local_secret.is_empty() => {
            *secret_key = local_secret.clone();
        }
        _ => {}
    }
}

/// 值形态启发式：像 JWT / 长随机串时即便键名不敏感也脱敏。
fn looks_like_secret(value: &str) -> bool {
    // JWT：三段 base64url 以 . 分隔。
    let jwt_like = value.matches('.').count() == 2
        && value.len() > 40
        && value.split('.').all(|p| {
            !p.is_empty()
                && p.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        });
    jwt_like
}

fn report_unique(log: &mut RedactionLog) {
    log.entries.sort();
    log.entries.dedup();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests_support::sample_snapshot;
    use miyo_core::model::{Environment, EnvironmentVariable, KeyValue};

    #[test]
    fn redacts_sensitive_headers_and_auth() {
        let mut snapshot = sample_snapshot("/login");
        snapshot.endpoints[0].request.headers = vec![
            KeyValue::new("Authorization", "Bearer abc123"),
            KeyValue::new("X-Trace-Id", "keep-me"),
        ];
        snapshot.endpoints[0].request.auth = AuthSpec::Bearer {
            token: "secret-token".into(),
        };

        let report = redact_snapshot(&snapshot);
        let ep = &report.snapshot.endpoints[0];
        assert_eq!(ep.request.headers[0].value, PLACEHOLDER);
        assert_eq!(ep.request.headers[1].value, "keep-me");
        assert_eq!(
            ep.request.auth,
            AuthSpec::Bearer {
                token: PLACEHOLDER.into()
            }
        );
        assert_eq!(report.log.entries.len(), 2);
    }

    #[test]
    fn redacts_env_secrets_and_drops_local_overrides() {
        let mut snapshot = sample_snapshot("/x");
        snapshot.environments = vec![Environment {
            id: uuid::Uuid::new_v4(),
            project_id: snapshot.project.id,
            name: "prod".into(),
            base_url: "https://api.example.com".into(),
            variables: vec![
                EnvironmentVariable {
                    key: "api_token".into(),
                    remote_value: "tok-123".into(),
                    local_value: "".into(),
                    enabled: true,
                    local_override: None,
                    sensitive: None,
                    description: None,
                },
                EnvironmentVariable {
                    key: "region".into(),
                    remote_value: "cn-north".into(),
                    local_value: "local-only".into(),
                    enabled: true,
                    local_override: None,
                    sensitive: None,
                    description: None,
                },
            ],
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }];

        let report = redact_snapshot(&snapshot);
        let vars = &report.snapshot.environments[0].variables;
        assert_eq!(vars[0].remote_value, PLACEHOLDER);
        // 非敏感键保留；本地覆盖清空。
        assert_eq!(vars[1].remote_value, "cn-north");
        assert_eq!(vars[1].local_value, "");
    }

    #[test]
    fn restore_keeps_local_secrets_and_fills_placeholder() {
        let mut remote = sample_snapshot("/login");
        remote.endpoints[0].request.headers = vec![KeyValue::new("Authorization", PLACEHOLDER)];
        remote.endpoints[0].request.auth = AuthSpec::Bearer {
            token: PLACEHOLDER.into(),
        };

        let mut local = sample_snapshot("/login");
        local.endpoints[0].request.headers = vec![KeyValue::new("Authorization", "Bearer mine")];
        local.endpoints[0].request.auth = AuthSpec::Bearer {
            token: "my-token".into(),
        };

        restore_local_values(&mut remote, &local);
        assert_eq!(remote.endpoints[0].request.headers[0].value, "Bearer mine");
        assert_eq!(
            remote.endpoints[0].request.auth,
            AuthSpec::Bearer {
                token: "my-token".into()
            }
        );
    }

    #[test]
    fn looks_like_secret_detects_jwt() {
        let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U";
        assert!(looks_like_secret(jwt));
        assert!(!looks_like_secret("application/json"));
        assert!(!looks_like_secret("1.2.3"));
    }

    /// 禁用参数同样脱敏（值随快照外发，禁用只影响发送）。
    #[test]
    fn redacts_disabled_params() {
        let mut snapshot = sample_snapshot("/x");
        snapshot.endpoints[0].request.params = vec![KeyValue {
            key: "access_token".into(),
            value: "tok-999".into(),
            enabled: false,
            description: String::new(),
            field_type: miyo_core::model::FieldType::default(),
            required: true,
            example: String::new(),
        }];
        let report = redact_snapshot(&snapshot);
        let param = &report.snapshot.endpoints[0].request.params[0];
        assert_eq!(param.value, PLACEHOLDER);
        assert!(!param.enabled, "禁用状态不得被改动");
        assert!(report.log.entries.iter().any(|e| e.contains("已禁用")));
    }

    /// JSON Body 敏感字段脱敏 + UrlEncoded / Multipart 字段脱敏。
    #[test]
    fn redacts_body_secrets() {
        let mut snapshot = sample_snapshot("/x");
        snapshot.endpoints[0].request.body = miyo_core::model::BodySpec::Json {
            raw: r#"{"username":"u","password":"pw-1","note":{"api_key":"ak-2"},"keep":"plain"}"#
                .into(),
        };
        let report = redact_snapshot(&snapshot);
        let raw = match &report.snapshot.endpoints[0].request.body {
            miyo_core::model::BodySpec::Json { raw } => raw.clone(),
            other => panic!("body 应为 JSON，实际 {other:?}"),
        };
        assert!(!raw.contains("pw-1"), "密码应被脱敏: {raw}");
        assert!(!raw.contains("ak-2"), "嵌套 api_key 应被脱敏: {raw}");
        assert!(raw.contains("plain"), "非敏感字段保留");
        assert!(raw.contains("u"), "用户名保留");
        assert!(report.log.entries.iter().any(|e| e.contains("JSON Body")));

        // Multipart 文本字段按键名脱敏；文件字段不动。
        snapshot.endpoints[0].request.body = miyo_core::model::BodySpec::Multipart {
            fields: vec![
                miyo_core::model::MultipartField {
                    key: "password".into(),
                    value_type: miyo_core::model::MultipartValueType::Text,
                    value: "mp-pw".into(),
                    enabled: true,
                },
                miyo_core::model::MultipartField {
                    key: "avatar".into(),
                    value_type: miyo_core::model::MultipartValueType::FilePath,
                    value: "/tmp/a.png".into(),
                    enabled: true,
                },
            ],
        };
        let report = redact_snapshot(&snapshot);
        match &report.snapshot.endpoints[0].request.body {
            miyo_core::model::BodySpec::Multipart { fields } => {
                assert_eq!(fields[0].value, PLACEHOLDER);
                assert_eq!(fields[1].value, "/tmp/a.png", "文件字段保留");
            }
            other => panic!("body 应为 Multipart，实际 {other:?}"),
        }
    }

    /// 请求用例与响应示例纳入脱敏范围。
    #[test]
    fn redacts_examples() {
        use miyo_core::model::{BodySpec, RequestExample};
        let mut snapshot = sample_snapshot("/x");
        let ep_id = snapshot.endpoints[0].id;
        snapshot.request_examples = vec![RequestExample {
            id: uuid::Uuid::new_v4(),
            endpoint_id: ep_id,
            name: "带令牌的用例".into(),
            request: miyo_core::model::RequestSpec {
                auth: AuthSpec::Bearer {
                    token: "ex-tok".into(),
                },
                body: BodySpec::Json {
                    raw: r#"{"token":"bt-1"}"#.into(),
                },
                ..Default::default()
            },
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }];
        // sample_snapshot 无响应示例，构造一条带敏感响应头的。
        snapshot.response_examples = vec![miyo_core::model::ResponseExample {
            id: uuid::Uuid::new_v4(),
            endpoint_id: ep_id,
            name: "成功".into(),
            status: 200,
            headers: std::collections::HashMap::from([(
                "Set-Cookie".to_string(),
                "sid=secret-session".to_string(),
            )]),
            body: "{}".into(),
            content_type: "application/json".into(),
            docs: Default::default(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }];
        let report = redact_snapshot(&snapshot);
        let ex = &report.snapshot.request_examples[0];
        assert!(
            matches!(&ex.request.auth, AuthSpec::Bearer { token } if token == PLACEHOLDER),
            "用例 auth 应脱敏"
        );
        match &ex.request.body {
            BodySpec::Json { raw } => assert!(!raw.contains("bt-1")),
            other => panic!("用例 body 应为 JSON，实际 {other:?}"),
        }
        assert_eq!(
            report.snapshot.response_examples[0]
                .headers
                .get("Set-Cookie"),
            Some(&PLACEHOLDER.to_string())
        );
        assert_eq!(report.snapshot.response_examples[0].body, "{}");
    }

    /// 脚本类内容无法自动判断：登记人工审查提示而非静默放过。
    #[test]
    fn flags_scripts_for_manual_review() {
        use miyo_core::model::{Scenario, ScenarioStep, ScriptsSpec};
        let mut snapshot = sample_snapshot("/x");
        snapshot.endpoints[0].request.scripts = Some(ScriptsSpec {
            pre_request: "pm.request.headers.add({key:'X-Sign', value: sign()})".into(),
            post_response: String::new(),
        });
        let ep_id = snapshot.endpoints[0].id;
        snapshot.scenarios = vec![Scenario {
            id: uuid::Uuid::new_v4(),
            project_id: snapshot.project.id,
            name: "登录链路".into(),
            description: String::new(),
            category: String::new(),
            sort_order: 0,
            steps: vec![ScenarioStep {
                endpoint_id: Some(ep_id),
                enabled: true,
                extract: serde_json::json!([]),
                assertions: serde_json::json!([]),
                name: None,
                request: None,
            }],
            script: Some("fetch(url, {headers:{authorization: 'Bearer ' + REAL_TOKEN}})".into()),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }];
        let report = redact_snapshot(&snapshot);
        assert!(report
            .log
            .entries
            .iter()
            .any(|e| e.contains("登录链路") && e.contains("人工确认")));
        assert!(report
            .log
            .entries
            .iter()
            .any(|e| e.contains("前置/后置脚本")));
        // 脚本内容本身不替换（无法可靠判定），靠提示驱动人工检查。
        assert!(report.snapshot.scenarios[0]
            .script
            .as_deref()
            .unwrap()
            .contains("REAL_TOKEN"));
    }

    /// 拉取还原：查询参数占位符同样从本地取回。
    #[test]
    fn restore_refills_param_placeholders() {
        let mut remote = sample_snapshot("/login");
        remote.endpoints[0].request.params = vec![KeyValue::new("sig", PLACEHOLDER)];
        let mut local = sample_snapshot("/login");
        local.endpoints[0].request.params = vec![KeyValue::new("sig", "local-sig")];
        restore_local_values(&mut remote, &local);
        assert_eq!(remote.endpoints[0].request.params[0].value, "local-sig");
    }
}
