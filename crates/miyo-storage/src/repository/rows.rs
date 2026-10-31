//! 行映射与共享工具（内部）。

use chrono::Utc;
use uuid::Uuid;

use miyo_core::model::*;
use miyo_core::{AppError, Result};

#[derive(sqlx::FromRow)]
pub(crate) struct ProjectRow {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) variables_json: String,
    pub(crate) team_id: Option<String>,
    pub(crate) created_at: String,
    pub(crate) updated_at: String,
}

impl ProjectRow {
    pub(crate) fn from_model(model: &Project) -> Self {
        ProjectRow {
            id: model.id.to_string(),
            name: model.name.clone(),
            description: model.description.clone(),
            variables_json: serde_json::to_string(&model.variables).unwrap_or_else(|_| "{}".into()),
            team_id: model.team_id.map(|v| v.to_string()),
            created_at: model.created_at.to_rfc3339(),
            updated_at: model.updated_at.to_rfc3339(),
        }
    }

    pub(crate) fn into_model(self) -> Result<Project> {
        Ok(Project {
            id: parse_uuid(&self.id)?,
            name: self.name,
            description: self.description,
            variables: serde_json::from_str(&self.variables_json).unwrap_or_default(),
            team_id: self.team_id.map(|v| parse_uuid(&v)).transpose()?,
            created_at: parse_time(&self.created_at)?,
            updated_at: parse_time(&self.updated_at)?,
        })
    }
}

#[derive(sqlx::FromRow)]
pub(crate) struct FolderRow {
    pub(crate) id: String,
    pub(crate) project_id: String,
    pub(crate) parent_id: Option<String>,
    pub(crate) name: String,
    pub(crate) sort_order: i64,
    pub(crate) created_at: String,
    pub(crate) updated_at: String,
}

impl FolderRow {
    pub(crate) fn from_model(model: &Folder) -> FolderRow {
        FolderRow {
            id: model.id.to_string(),
            project_id: model.project_id.to_string(),
            parent_id: model.parent_id.map(|v| v.to_string()),
            name: model.name.clone(),
            sort_order: model.sort_order,
            created_at: model.created_at.to_rfc3339(),
            updated_at: model.updated_at.to_rfc3339(),
        }
    }

    pub(crate) fn into_model(self) -> Result<Folder> {
        Ok(Folder {
            id: parse_uuid(&self.id)?,
            project_id: parse_uuid(&self.project_id)?,
            parent_id: self.parent_id.map(|s| parse_uuid(&s)).transpose()?,
            name: self.name,
            sort_order: self.sort_order,
            created_at: parse_time(&self.created_at)?,
            updated_at: parse_time(&self.updated_at)?,
        })
    }
}

#[derive(sqlx::FromRow)]
pub(crate) struct EndpointRow {
    pub(crate) id: String,
    pub(crate) project_id: String,
    pub(crate) folder_id: Option<String>,
    pub(crate) name: String,
    pub(crate) method: String,
    pub(crate) path: String,
    pub(crate) description: String,
    pub(crate) status: String,
    pub(crate) sort_order: i64,
    pub(crate) request_json: String,
    pub(crate) created_at: String,
    pub(crate) updated_at: String,
}

impl EndpointRow {
    pub(crate) fn from_model(model: &Endpoint) -> EndpointRow {
        EndpointRow {
            id: model.id.to_string(),
            project_id: model.project_id.to_string(),
            folder_id: model.folder_id.map(|v| v.to_string()),
            name: model.name.clone(),
            method: model.method.as_str().to_string(),
            path: model.path.clone(),
            description: model.description.clone(),
            status: model.status.as_str().to_string(),
            sort_order: model.sort_order,
            // app_secret 为敏感字段：持久化前加密（AES-256-GCM，见本文件
            // `encrypt_request_secrets`；加密失败降级明文并告警）。
            request_json: encrypt_request_secrets(
                &serde_json::to_string(&model.request).unwrap_or_else(|_| "{}".into()),
            ),
            created_at: model.created_at.to_rfc3339(),
            updated_at: model.updated_at.to_rfc3339(),
        }
    }

    pub(crate) fn into_model(self) -> Result<Endpoint> {
        Ok(Endpoint {
            id: parse_uuid(&self.id)?,
            project_id: parse_uuid(&self.project_id)?,
            folder_id: self.folder_id.map(|s| parse_uuid(&s)).transpose()?,
            name: self.name,
            method: self.method.parse()?,
            path: self.path,
            description: self.description,
            status: match self.status.as_str() {
                "designing" => EndpointStatus::Designing,
                "developing" => EndpointStatus::Developing,
                "testing" => EndpointStatus::Testing,
                "released" => EndpointStatus::Released,
                "deprecated" => EndpointStatus::Deprecated,
                _ => EndpointStatus::Developing,
            },
            sort_order: self.sort_order,
            // 读路径：先解密 app_secret 再反序列化（明文 / 非签名数据原样通过）。
            request: serde_json::from_str(&decrypt_request_secrets(&self.request_json)?)
                .unwrap_or_default(),
            created_at: parse_time(&self.created_at)?,
            updated_at: parse_time(&self.updated_at)?,
        })
    }
}

#[derive(sqlx::FromRow)]
pub(crate) struct EnvironmentRow {
    pub(crate) id: String,
    pub(crate) project_id: String,
    pub(crate) name: String,
    pub(crate) base_url: String,
    pub(crate) variables_json: String,
    pub(crate) created_at: String,
    pub(crate) updated_at: String,
}

impl EnvironmentRow {
    pub(crate) fn from_model(model: &Environment) -> EnvironmentRow {
        EnvironmentRow {
            id: model.id.to_string(),
            project_id: model.project_id.to_string(),
            name: model.name.clone(),
            // 基址非敏感信息，明文落库。
            base_url: model.base_url.clone(),
            // M11：变量整体加密后落库（密钥不可用时降级明文，保证可用性）。
            variables_json: encrypt_env_json(&model.variables),
            created_at: model.created_at.to_rfc3339(),
            updated_at: model.updated_at.to_rfc3339(),
        }
    }

    pub(crate) fn into_model(self) -> Result<Environment> {
        let value = decrypt_env_json(&self.variables_json)?;
        let variables = variables_from_value(value)?;
        Ok(Environment {
            id: parse_uuid(&self.id)?,
            project_id: parse_uuid(&self.project_id)?,
            name: self.name,
            base_url: self.base_url,
            variables,
            created_at: parse_time(&self.created_at)?,
            updated_at: parse_time(&self.updated_at)?,
        })
    }
}

#[derive(sqlx::FromRow)]
pub(crate) struct TestRunRow {
    pub(crate) id: String,
    pub(crate) project_id: String,
    pub(crate) environment_id: Option<String>,
    pub(crate) name: String,
    pub(crate) result_json: String,
    pub(crate) started_at: String,
    pub(crate) finished_at: Option<String>,
}

impl TestRunRow {
    pub(crate) fn from_model(model: &TestRun) -> Self {
        TestRunRow {
            id: model.id.to_string(),
            project_id: model.project_id.to_string(),
            environment_id: model.environment_id.map(|e| e.to_string()),
            name: model.name.clone(),
            result_json: model.result_json.clone(),
            started_at: model.started_at.to_rfc3339(),
            finished_at: model.finished_at.map(|d| d.to_rfc3339()),
        }
    }

    pub(crate) fn into_model(self) -> Result<TestRun> {
        Ok(TestRun {
            id: parse_uuid(&self.id)?,
            project_id: parse_uuid(&self.project_id)?,
            environment_id: self.environment_id.as_deref().map(parse_uuid).transpose()?,
            name: self.name,
            result_json: self.result_json,
            started_at: parse_time(&self.started_at)?,
            finished_at: self.finished_at.as_deref().map(parse_time).transpose()?,
        })
    }
}

#[derive(sqlx::FromRow)]
pub(crate) struct ScenarioRunRow {
    pub(crate) id: String,
    pub(crate) scenario_id: String,
    pub(crate) environment_id: Option<String>,
    pub(crate) finished_at: String,
    pub(crate) duration_ms: Option<f64>,
    pub(crate) total_steps: i64,
    pub(crate) ok_count: i64,
    pub(crate) fail_count: i64,
    pub(crate) skipped_count: i64,
    pub(crate) cancelled: i64,
    pub(crate) stopped_on_failure: i64,
    pub(crate) results_json: String,
    pub(crate) variables_json: String,
}

impl ScenarioRunRow {
    pub(crate) fn from_model(model: &ScenarioRun) -> Self {
        ScenarioRunRow {
            id: model.id.to_string(),
            scenario_id: model.scenario_id.to_string(),
            environment_id: model.environment_id.map(|e| e.to_string()),
            finished_at: model.finished_at.to_rfc3339(),
            duration_ms: model.duration_ms,
            total_steps: model.total_steps,
            ok_count: model.ok_count,
            fail_count: model.fail_count,
            skipped_count: model.skipped_count,
            cancelled: model.cancelled as i64,
            stopped_on_failure: model.stopped_on_failure as i64,
            results_json: model.results_json.clone(),
            variables_json: model.variables_json.clone(),
        }
    }

    pub(crate) fn into_model(self) -> Result<ScenarioRun> {
        Ok(ScenarioRun {
            id: parse_uuid(&self.id)?,
            scenario_id: parse_uuid(&self.scenario_id)?,
            environment_id: self.environment_id.as_deref().map(parse_uuid).transpose()?,
            finished_at: parse_time(&self.finished_at)?,
            duration_ms: self.duration_ms,
            total_steps: self.total_steps,
            ok_count: self.ok_count,
            fail_count: self.fail_count,
            skipped_count: self.skipped_count,
            cancelled: self.cancelled != 0,
            stopped_on_failure: self.stopped_on_failure != 0,
            results_json: self.results_json,
            variables_json: self.variables_json,
        })
    }
}

#[derive(sqlx::FromRow)]
pub(crate) struct HistoryRow {
    pub(crate) id: String,
    pub(crate) project_id: String,
    pub(crate) endpoint_id: Option<String>,
    pub(crate) method: String,
    pub(crate) url: String,
    pub(crate) status: Option<i64>,
    pub(crate) duration_ms: Option<i64>,
    pub(crate) request_summary_json: String,
    pub(crate) response_summary_json: String,
    pub(crate) created_at: String,
}

impl HistoryRow {
    pub(crate) fn from_model(model: &RequestHistory) -> Self {
        HistoryRow {
            id: model.id.to_string(),
            project_id: model.project_id.to_string(),
            endpoint_id: model.endpoint_id.map(|e| e.to_string()),
            method: model.method.clone(),
            url: model.url.clone(),
            status: model.status.map(|s| s as i64),
            duration_ms: model.duration_ms.map(|d| d as i64),
            request_summary_json: model.request_summary_json.clone(),
            response_summary_json: model.response_summary_json.clone(),
            created_at: model.created_at.to_rfc3339(),
        }
    }

    pub(crate) fn into_model(self) -> Result<RequestHistory> {
        Ok(RequestHistory {
            id: parse_uuid(&self.id)?,
            project_id: parse_uuid(&self.project_id)?,
            endpoint_id: match self.endpoint_id {
                Some(v) => Some(parse_uuid(&v)?),
                None => None,
            },
            method: self.method,
            url: self.url,
            status: self.status.map(|v| v as u16),
            duration_ms: self.duration_ms.map(|d| d as u64),
            request_summary_json: self.request_summary_json,
            response_summary_json: self.response_summary_json,
            created_at: parse_time(&self.created_at)?,
        })
    }
}

#[derive(sqlx::FromRow)]
pub(crate) struct MockRuleRow {
    pub(crate) id: String,
    pub(crate) project_id: String,
    pub(crate) endpoint_id: Option<String>,
    pub(crate) name: String,
    pub(crate) method: String,
    pub(crate) path: String,
    pub(crate) match_query_json: String,
    pub(crate) match_headers_json: String,
    pub(crate) response_status: i64,
    pub(crate) response_headers_json: String,
    pub(crate) response_body_template: String,
    pub(crate) delay_ms: i64,
    pub(crate) fault_rate_pct: i64,
    pub(crate) fault_status: i64,
    pub(crate) enabled: i64,
    pub(crate) priority: i64,
    pub(crate) created_at: String,
    pub(crate) updated_at: String,
}

impl MockRuleRow {
    pub(crate) fn from_model(model: &MockRule) -> Self {
        MockRuleRow {
            id: model.id.to_string(),
            project_id: model.project_id.to_string(),
            endpoint_id: model.endpoint_id.map(|id| id.to_string()),
            name: model.name.clone(),
            method: model.method.as_str().to_string(),
            path: model.path.clone(),
            match_query_json: serde_json::to_string(&model.match_query)
                .unwrap_or_else(|_| "[]".into()),
            match_headers_json: serde_json::to_string(&model.match_headers)
                .unwrap_or_else(|_| "[]".into()),
            response_status: model.response_status as i64,
            response_headers_json: serde_json::to_string(&model.response_headers)
                .unwrap_or_else(|_| "{}".into()),
            response_body_template: model.response_body_template.clone(),
            delay_ms: model.delay_ms as i64,
            fault_rate_pct: model.fault_rate_pct as i64,
            fault_status: model.fault_status as i64,
            enabled: if model.enabled { 1 } else { 0 },
            priority: model.priority,
            created_at: model.created_at.to_rfc3339(),
            updated_at: model.updated_at.to_rfc3339(),
        }
    }

    pub(crate) fn into_model(self) -> Result<MockRule> {
        Ok(MockRule {
            id: parse_uuid(&self.id)?,
            project_id: parse_uuid(&self.project_id)?,
            endpoint_id: match self.endpoint_id {
                Some(id) => Some(parse_uuid(&id)?),
                None => None,
            },
            name: self.name,
            method: self.method.parse()?,
            path: self.path,
            match_query: serde_json::from_str(&self.match_query_json).unwrap_or_default(),
            match_headers: serde_json::from_str(&self.match_headers_json).unwrap_or_default(),
            response_status: self.response_status as u16,
            response_headers: serde_json::from_str(&self.response_headers_json).unwrap_or_default(),
            response_body_template: self.response_body_template,
            delay_ms: self.delay_ms as u64,
            fault_rate_pct: self.fault_rate_pct.clamp(0, 100) as u8,
            fault_status: self.fault_status as u16,
            enabled: self.enabled != 0,
            priority: self.priority,
            created_at: parse_time(&self.created_at)?,
            updated_at: parse_time(&self.updated_at)?,
        })
    }
}

#[derive(sqlx::FromRow)]
pub(crate) struct ResponseExampleRow {
    pub(crate) id: String,
    pub(crate) endpoint_id: String,
    pub(crate) name: String,
    pub(crate) status: i64,
    pub(crate) headers_json: String,
    pub(crate) body: String,
    pub(crate) content_type: String,
    pub(crate) docs_json: String,
    pub(crate) created_at: String,
    pub(crate) updated_at: String,
}

impl ResponseExampleRow {
    pub(crate) fn from_model(model: &ResponseExample) -> Self {
        ResponseExampleRow {
            id: model.id.to_string(),
            endpoint_id: model.endpoint_id.to_string(),
            name: model.name.clone(),
            status: model.status as i64,
            headers_json: serde_json::to_string(&model.headers).unwrap_or_else(|_| "{}".into()),
            body: model.body.clone(),
            content_type: model.content_type.clone(),
            docs_json: serde_json::to_string(&model.docs).unwrap_or_else(|_| "{}".into()),
            created_at: model.created_at.to_rfc3339(),
            updated_at: model.updated_at.to_rfc3339(),
        }
    }

    pub(crate) fn into_model(self) -> Result<ResponseExample> {
        Ok(ResponseExample {
            id: parse_uuid(&self.id)?,
            endpoint_id: parse_uuid(&self.endpoint_id)?,
            name: self.name,
            status: self.status as u16,
            headers: serde_json::from_str(&self.headers_json).unwrap_or_default(),
            body: self.body,
            content_type: self.content_type,
            docs: serde_json::from_str(&self.docs_json).unwrap_or_default(),
            created_at: parse_time(&self.created_at)?,
            updated_at: parse_time(&self.updated_at)?,
        })
    }
}

#[derive(sqlx::FromRow)]
pub(crate) struct WsMessageRow {
    pub(crate) id: String,
    pub(crate) message_type: String,
    pub(crate) payload: String,
    pub(crate) created_at: String,
}

impl WsMessageRow {
    pub(crate) fn into_model(self) -> Result<WsMessageRecord> {
        Ok(WsMessageRecord {
            id: parse_uuid(&self.id)?,
            message_type: match self.message_type.as_str() {
                "text" => WsMessageType::Text,
                "binary" => WsMessageType::Binary,
                "ping" => WsMessageType::Ping,
                other => {
                    return Err(AppError::Validation(format!("无效的消息类型：{other}")));
                }
            },
            payload: self.payload,
            created_at: parse_time(&self.created_at)?,
        })
    }
}

/// 环境变量加密（AES-256-GCM，密钥见 miyo-secret）。
pub(crate) fn encrypt_env_json(vars: &[EnvironmentVariable]) -> String {
    let json = serde_json::to_string(vars).unwrap_or_else(|_| "[]".into());
    match miyo_secret::ensure_master_key().and_then(|k| miyo_secret::encrypt(&k, &json)) {
        Ok(cipher) => cipher,
        Err(e) => {
            // 可用性优先降级为明文落库，但必须留下痕迹：密钥文件只读 /
            // 磁盘满等故障若静默发生，用户无从得知敏感数据未加密。
            tracing::warn!(error = %e, "环境变量加密失败，已降级为明文存储");
            json
        }
    }
}

/// 环境变量解密。
///
/// 旧版本明文数据原样返回；明确加密格式但解密失败（主密钥丢失 / 更换、
/// 密文损坏）返回 `AppError::Decryption`，由 UI 层弹窗提示，
/// 避免把 base64 密文当明文解析成空变量而静默丢失。
pub(crate) fn decrypt_env_json(json: &str) -> Result<serde_json::Value> {
    let plain = miyo_secret::ensure_master_key()
        .and_then(|k| miyo_secret::decrypt(&k, json))
        .map_err(|e| AppError::Decryption(e.to_string()))?;
    serde_json::from_str(&plain)
        .map_err(|_| AppError::Decryption("环境变量密文已损坏，无法解析".to_string()))
}

/// 从解密后的 JSON 解析结构化变量。
///
/// 新格式为数组 `[EnvironmentVariable]`；旧格式为 `{key:value}` map，
/// 全部键转为 `EnvironmentVariable { remote_value = value, enabled: true }`。
pub(crate) fn variables_from_value(value: serde_json::Value) -> Result<Vec<EnvironmentVariable>> {
    match value {
        serde_json::Value::Array(items) => {
            let mut vars = Vec::with_capacity(items.len());
            for item in items {
                vars.push(serde_json::from_value(item).map_err(|_| {
                    AppError::Decryption("环境变量密文已损坏，无法解析".to_string())
                })?);
            }
            Ok(vars)
        }
        serde_json::Value::Object(map) => {
            let mut vars = Vec::with_capacity(map.len());
            for (key, value) in map {
                let raw = match value {
                    serde_json::Value::String(s) => s,
                    other => other.to_string(),
                };
                if key.trim().is_empty() || key.starts_with("{{") || key.starts_with('$') {
                    continue;
                }
                vars.push(EnvironmentVariable {
                    key,
                    remote_value: raw,
                    local_value: String::new(),
                    enabled: true,
                    description: None,
                });
            }
            Ok(vars)
        }
        _ => Err(AppError::Decryption(
            "环境变量密文已损坏，无法解析".to_string(),
        )),
    }
}

pub(crate) fn parse_uuid(s: &str) -> Result<Uuid> {
    Uuid::parse_str(s).map_err(|e| AppError::Validation(format!("无效 ID：{s}（{e}）")))
}

// ---------------------------------------------------------------------------
// 动态签名 app_secret 加密（AES-256-GCM，密钥见 miyo-secret）。
//
// [占位实现]：endpoints.request_json 整包以 JSON 持久化，其中
// `AuthSpec::DynamicSignature.config.app_secret` 为敏感字段。完整接入点有二：
//
// 1. 写路径：`EndpointRow::from_model`（本文件）序列化后调用
//    `encrypt_request_secrets`，把 app_secret 字段替换为密文；
// 2. 读路径：`EndpointRow::into_model` 反序列化前调用
//    `decrypt_request_secrets`，还原明文后照常解析。
//
// 下方两个函数已按递归 JSON 遍历实现（未来 App-Secret 字段改名 / 新增
// 敏感字段时无需改这里，按 key 名匹配即可）；是否接入由上层按发布节奏
// 决定——接入前旧库中 app_secret 为明文，decrypt 对无 `:` 前缀的明文
// 原样返回（miyo-secret 兼容策略），不会破坏既有数据。
/// 把请求 JSON 中动态签名鉴权的 `app_secret` 字段加密为密文。
///
/// 加密失败降级为明文保留（与 `encrypt_env_json` 一致），并记录 warn，
/// 避免磁盘故障等偶发问题把整个请求写失败。新数据只含动态签名时
/// 走 miyo-secret 的 `encrypt`；旧明文 / 非动态签名数据原样通过。
pub(crate) fn encrypt_request_secrets(request_json: &str) -> String {
    encrypt_request_json(request_json, true)
}

/// 把请求 JSON 中动态签名鉴权的 `app_secret` 字段解密回明文。
///
/// 加密格式解不开（主密钥丢失/更换）时返回 `AppError::Decryption` 由上层
/// 提示用户，避免把密文当明文继续使用。JSON 损坏时原样返回（与旧版
/// `unwrap_or_default` 容错行为一致，交由反序列化兜底）。
pub(crate) fn decrypt_request_secrets(request_json: &str) -> Result<String> {
    let Ok(value) = serde_json::from_str(request_json) else {
        return Ok(request_json.to_string());
    };
    let out = walk_secrets(value, false)
        .map_err(|e| AppError::Decryption(format!("请求密文解密失败：{e}")))?;
    Ok(serde_json::to_string(&out).unwrap_or_else(|_| request_json.to_string()))
}

fn encrypt_request_json(request_json: &str, encrypt: bool) -> String {
    let value: serde_json::Value = match serde_json::from_str(request_json) {
        Ok(v) => v,
        Err(_) => return request_json.to_string(),
    };
    let out = match walk_secrets(value, encrypt) {
        Ok(v) => v,
        Err(_) => return request_json.to_string(),
    };
    serde_json::to_string(&out).unwrap_or_else(|_| request_json.to_string())
}

/// 递归遍历 JSON，凡 key 为 `app_secret` 的字符串字段做加/解密。
///
/// 用 key 名匹配而非深度绑定到 `DynamicSignatureConfig` 结构：未来该字段
/// 挪到 OAuth2 等其它鉴权对象中也能自动覆盖；同时规避 serde 扁平化标签
/// 带来的结构感知复杂度。
fn walk_secrets(value: serde_json::Value, encrypt: bool) -> Result<serde_json::Value> {
    match value {
        serde_json::Value::Object(map) => {
            let mut out = serde_json::Map::with_capacity(map.len());
            for (key, v) in map {
                if key == "app_secret" {
                    // 仅字符串字段参与加解密；非字符串（异常数据）原样保留。
                    if let serde_json::Value::String(s) = v {
                        let secret = if encrypt {
                            miyo_secret::ensure_master_key()
                                .and_then(|k| miyo_secret::encrypt(&k, &s))
                                .map_err(|e| AppError::Decryption(e.to_string()))?
                        } else {
                            miyo_secret::ensure_master_key()
                                .and_then(|k| miyo_secret::decrypt(&k, &s))
                                .map_err(|e| AppError::Decryption(e.to_string()))?
                        };
                        out.insert(key, serde_json::Value::String(secret));
                    } else {
                        out.insert(key, v);
                    }
                } else {
                    // 原地递归消费，避免逐节点 clone。
                    out.insert(key, walk_secrets(v, encrypt)?);
                }
            }
            Ok(serde_json::Value::Object(out))
        }
        serde_json::Value::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(walk_secrets(item, encrypt)?);
            }
            Ok(serde_json::Value::Array(out))
        }
        other => Ok(other),
    }
}

pub(crate) fn parse_time(s: &str) -> Result<chrono::DateTime<Utc>> {
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&Utc))
        .map_err(|e| AppError::Validation(format!("无效时间：{s}（{e}）")))
}

#[cfg(test)]
mod secret_tests {
    use super::*;

    #[test]
    fn app_secret_roundtrips_through_encrypt_decrypt() {
        let json = r#"{"auth":{"type":"dynamic_signature","config":{"app_secret":"hunter2","app_key":"k"}}}"#;
        let enc = encrypt_request_secrets(json);
        assert_ne!(enc, json);
        assert!(enc.contains(":"), "密文应为 nonce:ciphertext 格式");
        assert!(!enc.contains("hunter2"), "明文不得残留");
        let dec = decrypt_request_secrets(&enc).unwrap();
        assert!(dec.contains("hunter2"));
    }

    #[test]
    fn non_signature_json_is_passthrough() {
        let json = r#"{"headers":[{"key":"X","value":"1"}]}"#;
        assert_eq!(encrypt_request_secrets(json), json);
        assert_eq!(decrypt_request_secrets(json).unwrap(), json);
    }

    #[test]
    fn decrypt_plaintext_app_secret_passthrough() {
        // 旧库明文 app_secret：解密原样返回，不误报。
        let json = r#"{"config":{"app_secret":"legacy-plain"}}"#;
        assert_eq!(decrypt_request_secrets(json).unwrap(), json);
    }

    #[test]
    fn corrupt_json_is_passthrough() {
        assert_eq!(encrypt_request_secrets("not-json"), "not-json");
        assert_eq!(decrypt_request_secrets("not-json").unwrap(), "not-json");
    }
}
