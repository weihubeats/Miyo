//! 请求执行 Command：变量渲染 → 参数校验 → 发送 HTTP 请求（成功后落历史）。

use serde::{Deserialize, Serialize};
use tauri::State;
use uuid::Uuid;

use miyo_core::model::{HttpMethod, KeyValue, RequestHistory, RequestSpec};
use miyo_core::render_spec;
use miyo_core::VariableMap;
use miyo_http::client::{HttpResponseData, RequestSnapshot};
use miyo_storage::repository as repo;

use crate::error::{CommandError, CommandResult};
use crate::state::AppState;

/// 执行请求的入参（前端结构体，与 `RequestSpec` 一致可 JSON 互传）。
#[derive(Debug, Clone, Deserialize)]
pub struct ExecuteRequestArgs {
    /// 请求 URL 模板，支持 `{{variable}}`。
    pub url: String,
    pub method: HttpMethod,
    pub spec: RequestSpec,
    /// 本次请求使用的环境（缺省使用当前激活环境）。
    pub environment_id: Option<Uuid>,
    /// 历史归属项目 / 接口（可选；提供时成功后记入请求历史）。
    pub project_id: Option<Uuid>,
    pub endpoint_id: Option<Uuid>,
    /// 本次请求的取消标识（由前端生成；提供后可通过 `cancel_request` 中止）。
    #[serde(default)]
    pub request_id: Option<String>,
}

/// 执行结果（`HttpResponseData` 含非序列化 `Bytes`，此处转成 JSON 安全结构）。
#[derive(Debug, Clone, Serialize)]
pub struct ExecuteResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
    pub content_type: String,
    /// 毫秒（浮点，保留亚毫秒精度）。
    pub duration_ms: f64,
    pub size_bytes: usize,
    pub truncated: bool,
    /// `body` 是否仅为 IPC 预览（超限时截断；全文经 `get_response_body` 按需取回）。
    pub body_omitted: bool,
    /// 本次请求实际发出的形态（临时随响应返回，不落历史）：
    /// 前端「请求」页签据此定位 URL / 头 / Cookie / 体的问题。
    pub request: Option<RequestSnapshot>,
    /// Pre / Post 脚本运行摘要（无脚本时为空、序列化省略）。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub scripts: Vec<ScriptRunSummary>,
}

/// 单条 `pm.test` 断言结果（序列化给前端展示）。
#[derive(Debug, Clone, Serialize)]
pub struct ScriptTestOutcome {
    pub name: String,
    pub passed: bool,
    pub error: Option<String>,
}

/// 一个阶段（pre_request / post_response）的脚本运行摘要。
#[derive(Debug, Clone, Serialize)]
pub struct ScriptRunSummary {
    /// 执行阶段：`pre_request` / `post_response`。
    pub phase: &'static str,
    /// 脚本本身执行失败（语法 / 运行时 / 超时）。post 阶段失败不吞掉已到达的响应。
    pub error: Option<String>,
    pub tests: Vec<ScriptTestOutcome>,
    pub logs: Vec<String>,
}

/// IPC 正文预览上限：webview 对 invoke 返回值做同步 `JSON.parse`，
/// 20MB 级字符串会冻结主线程（页面假死）；超限只回预览，
/// 全文留在 `AppState::response_bodies` 缓存按需取回。
const IPC_BODY_PREVIEW_BYTES: usize = 2 * 1024 * 1024;

/// 把响应全文切成 IPC 预览（按 UTF-8 字符边界截断，避免 panic）。
fn cap_ipc_body(full: &str) -> (String, bool) {
    if full.len() <= IPC_BODY_PREVIEW_BYTES {
        return (full.to_string(), false);
    }
    let mut end = IPC_BODY_PREVIEW_BYTES;
    while !full.is_char_boundary(end) {
        end -= 1;
    }
    (full[..end].to_string(), true)
}

/// 执行 HTTP 请求：加载变量 → 渲染 URL/Headers/Body → 参数校验 → 发送。
#[tauri::command(rename_all = "camelCase")]
pub async fn execute_request(
    state: State<'_, AppState>,
    args: ExecuteRequestArgs,
) -> CommandResult<ExecuteResponse> {
    // 1. 加载变量（环境 > 项目 > 全局），渲染 URL 与请求规格；再注入全局参数。
    //    全局设置（变量/参数/超时）走进程内缓存：命中时零 DB 往返（原来是
    //    每请求 2~3 次 settings SELECT + 2 次 AES-GCM 解密 + JSON 反序列化）。
    let (vars, settings, timeout_ms) = tokio::join!(
        state.variables_for(args.environment_id),
        state.global_settings(),
        resolve_timeout_ms(&args.spec, &state),
    );
    let vars = vars?;
    let settings = settings?;
    let timeout_ms = timeout_ms?;
    let url = miyo_core::resolve_variables(&args.url, &vars);
    let mut spec = render_spec(&args.spec, &vars);
    apply_global_params(&mut spec, &settings.global_params, &vars);

    // 1.5 脚本两段代码先取出（后续 spec.headers 会被 pre 脚本结果改写）。
    let (pre_code, post_code) = match &spec.scripts {
        Some(s) => (s.pre_request.clone(), s.post_response.clone()),
        None => (String::new(), String::new()),
    };

    // 2. 参数校验：URL 必填、必须是 http/https。
    if url.trim().is_empty() {
        return Err(CommandError::validation("URL 不能为空"));
    }
    let parsed =
        url::Url::parse(&url).map_err(|e| CommandError::validation(format!("URL 无效：{e}")))?;
    match parsed.scheme() {
        "http" | "https" => {}
        other => {
            return Err(CommandError::validation(format!(
                "不支持的协议：{other}（仅支持 http/https）"
            )));
        }
    }

    // 2.5 Pre-request 脚本：变量渲染后、发送前执行（放取消令牌注册之前，
    //     脚本失败早期返回不会泄漏令牌）。脚本可改头（回传完整列表并回规格）、
    //     读写变量（并入本次运行变量表，供 post-response 延续；不落库）。
    //     脚本执行出错则中止本次发送（与 Postman 语义一致）。
    let mut script_summaries: Vec<ScriptRunSummary> = Vec::new();
    let mut runtime_vars = vars;
    if !pre_code.trim().is_empty() {
        let sandbox = state.script_sandbox()?;
        let snapshot =
            miyo_http::script_sandbox::ScriptRequestData::from_spec(args.method, &url, &spec);
        match sandbox
            .run(miyo_http::script_sandbox::ScriptInput {
                code: pre_code,
                environment: runtime_vars.clone(),
                request: snapshot,
                response: None,
            })
            .await
        {
            Ok(r) => {
                apply_script_headers(&mut spec, &r.headers);
                for (k, v) in r.environment {
                    runtime_vars.insert(k, v);
                }
                script_summaries.push(ScriptRunSummary {
                    phase: "pre_request",
                    error: None,
                    tests: r.tests.into_iter().map(Into::into).collect(),
                    logs: r.logs,
                });
            }
            Err(e) => {
                return Err(CommandError::with_code(
                    "SCRIPT",
                    format!("Pre-request 脚本执行失败：{}", e.user_message()),
                ));
            }
        }
    }

    // 3. 注册取消令牌（前端「取消请求」→ `cancel_request` 触发中止）。
    let token = args.request_id.as_ref().map(|id| {
        let token = tokio_util::sync::CancellationToken::new();
        state
            .request_cancels
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(id.clone(), token.clone());
        (id.clone(), token)
    });

    // 4. 发送（超时已在第 1 步随变量同波次解析）。
    let result = async {
        let resp: HttpResponseData = match token.as_ref() {
            Some((_, t)) => {
                miyo_http::client::send_request_cancel(
                    args.method,
                    &url,
                    &spec,
                    Some(timeout_ms),
                    t,
                )
                .await?
            }
            None => {
                miyo_http::client::send_request(args.method, &url, &spec, Some(timeout_ms)).await?
            }
        };

        // 5. 映射为可序列化响应（IPC 只传预览，防大正文冻结 webview 主线程）。
        let full = resp.body_text();
        let (body, body_omitted) = cap_ipc_body(&full);
        let content_type = resp.content_type();

        // 5.05 Post-response 脚本：响应到达后执行，pm.test 断言与 console
        //     输出随响应返回；脚本失败不吞掉已到达的响应（记入 summary.error）。
        //     须在 `resp.request` 被 move 前执行（脚本快照要借用 resp）。
        if !post_code.trim().is_empty() {
            let summary = match run_post_response_script(
                state.inner(),
                args.method,
                &url,
                &spec,
                &resp,
                post_code,
                &runtime_vars,
            )
            .await
            {
                Ok((tests, logs)) => ScriptRunSummary {
                    phase: "post_response",
                    error: None,
                    tests,
                    logs,
                },
                Err(e) => ScriptRunSummary {
                    phase: "post_response",
                    error: Some(e.message),
                    tests: Vec::new(),
                    logs: Vec::new(),
                },
            };
            script_summaries.push(summary);
        }

        // 请求快照补上渲染前模板：前端据此对照变量替换结果（一致时不展示）。
        let mut request = resp.request;
        if let Some(snap) = request.as_mut() {
            snap.url_template = Some(args.url.clone());
        }

        let response = ExecuteResponse {
            status: resp.status,
            headers: resp.headers,
            body,
            content_type,
            duration_ms: resp.duration_ms,
            size_bytes: resp.size_bytes,
            truncated: resp.truncated,
            body_omitted,
            request,
            scripts: std::mem::take(&mut script_summaries),
        };

        // 5.1 被截断的全文入缓存：前端复制 / 保存示例按 request_id 取回。
        //     无 request_id（少数直调路径）则跳过，前端相应降级为预览。
        if body_omitted {
            if let Some(rid) = &args.request_id {
                state
                    .response_bodies
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .insert(rid.clone(), full);
            }
        }

        // 6. 写入请求历史（尽力而为：失败仅告警，不阻断发送）。
        //    后台任务执行，磁盘写入慢时不拖慢响应返回的感知耗时。
        if let Some(project_id) = args.project_id {
            let history = build_history(
                project_id,
                args.endpoint_id,
                args.method,
                &url,
                &args.url,
                &spec,
                &response,
            );
            let db = state.db.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = repo::save_request_history(&db, &history).await {
                    tracing::warn!("[execute_request] 保存历史失败：{}", e.user_message());
                }
            });
        }

        Ok(response)
    }
    .await;

    // 7. 无论成功 / 取消 / 失败，都从注册表移除，避免泄漏。
    if let Some((id, _)) = &token {
        state
            .request_cancels
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(id);
    }

    // 8. 自增序列若被本次请求推进，回写磁盘（尽力而为，失败仅告警不阻断）。
    //    后台执行：一次 settings 读改写不再加在响应返回路径上（历史保存同法）。
    let db = state.db.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = super::seq::sync_seq_counters_if_dirty(&db).await {
            tracing::warn!("[execute_request] 同步自增序列失败：{e}");
        }
    });

    result
}

/// Post-response 脚本执行：返回断言与 console 输出（执行失败由调用方记入 summary）。
async fn run_post_response_script(
    state: &AppState,
    method: HttpMethod,
    url: &str,
    spec: &RequestSpec,
    resp: &HttpResponseData,
    code: String,
    vars: &VariableMap,
) -> CommandResult<(Vec<ScriptTestOutcome>, Vec<String>)> {
    let sandbox = state.script_sandbox()?;
    let snapshot = miyo_http::script_sandbox::ScriptRequestData::from_spec(method, url, spec);
    let response = miyo_http::script_sandbox::ScriptResponseData::from_response(resp);
    let r = sandbox
        .run(miyo_http::script_sandbox::ScriptInput {
            code,
            environment: vars.clone(),
            request: snapshot,
            response: Some(response),
        })
        .await?;
    Ok((r.tests.into_iter().map(Into::into).collect(), r.logs))
}

impl From<miyo_http::script_sandbox::TestResult> for ScriptTestOutcome {
    fn from(t: miyo_http::script_sandbox::TestResult) -> Self {
        ScriptTestOutcome {
            name: t.name,
            passed: t.passed,
            error: t.error,
        }
    }
}

/// 把脚本回传的头列表并回规格：
/// - 启用行按脚本结果更新（大小写不敏感匹配，含脚本改写后的新 key 拼写）；
/// - 脚本删除的头（结果列表缺失）从启用集中移除；
/// - 脚本新增的头追加（description 标记「脚本」）；
/// - 禁用行原样保留（脚本头列表本就不含禁用行）。
fn apply_script_headers(spec: &mut RequestSpec, headers: &[(String, String)]) {
    let mut used = std::collections::HashSet::new();
    let mut out: Vec<KeyValue> = Vec::with_capacity(spec.headers.len() + 4);
    for kv in std::mem::take(&mut spec.headers) {
        if !kv.enabled {
            out.push(kv);
            continue;
        }
        if let Some((k, v)) = headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(kv.key.trim()))
        {
            used.insert(k.to_ascii_lowercase());
            out.push(KeyValue {
                key: k.clone(),
                value: v.clone(),
                enabled: true,
                ..kv
            });
        }
    }
    for (k, v) in headers {
        if !used.contains(&k.to_ascii_lowercase()) {
            let mut kv = KeyValue::new(k.clone(), v.clone());
            kv.description = "脚本".into();
            out.push(kv);
        }
    }
    spec.headers = out;
}

/// 取消一个在途请求（`request_id` 不存在或已完成时返回 `false`）。
///
/// 通过 reqwest `AbortHandle` + 取消令牌双重机制中止底层连接，
/// `execute_request` 随即以 `CANCELLED` 错误码返回。
#[tauri::command(rename_all = "camelCase")]
pub fn cancel_request(state: State<'_, AppState>, request_id: String) -> CommandResult<bool> {
    let token = state
        .request_cancels
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .remove(&request_id);
    if let Some(token) = token {
        token.cancel();
        Ok(true)
    } else {
        Ok(false)
    }
}

/// 按 `request_id` 取被 IPC 预览截断的完整响应正文。
///
/// 全文由 `execute_request` 在 `body_omitted` 时写入 `AppState::response_bodies`
/// （容量 4 条 + 10 分钟 TTL）；缓存未命中 / 过期返回 `None`，
/// 前端据此降级（复制 / 保存示例退回截断预览并提示）。
#[tauri::command(rename_all = "camelCase")]
pub fn get_response_body(
    state: State<'_, AppState>,
    request_id: String,
) -> CommandResult<Option<String>> {
    Ok(state
        .response_bodies
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(&request_id))
}

/// 构建历史记录。
///
/// - `url`：变量渲染后的实际发送地址——历史列表展示与关键字搜索的数据源
///   （存模板会让 `{{base_url}}/...` 字面量进入列表，且域名搜索失效）；
/// - `url_template`：渲染前模板——写入 `request_summary_json`，供前端
///   「恢复到编辑器」保留环境变量语义（换环境重发不锁死域名）；
/// - `request_summary_json` 另存完整请求规格（变量已渲染），认证字段统一
///   置空——凭据不落历史库，恢复时保留接口自身配置的认证。
fn build_history(
    project_id: Uuid,
    endpoint_id: Option<Uuid>,
    method: HttpMethod,
    url: &str,
    url_template: &str,
    spec: &RequestSpec,
    data: &ExecuteResponse,
) -> RequestHistory {
    // 响应预览按字节截断（字符边界安全）：`chars().take(n)` 会对 20MB
    // 大响应做全量字符迭代，耗时与 body 成正比；字节截断为 O(截断长度)。
    // 返回切片而非 String：截断与否都不产生中间拷贝——json! 内部
    // `to_value(&x)` 本来就会克隆一次进 Value，先前的 `to_string()` 属于白拷。
    let body_preview: &str = byte_truncate(&data.body, 2000);
    let mut spec_value = serde_json::to_value(spec).unwrap_or(serde_json::Value::Null);
    if let Some(obj) = spec_value.as_object_mut() {
        obj.insert("auth".into(), serde_json::json!({ "type": "none" }));
    }
    RequestHistory {
        id: Uuid::new_v4(),
        project_id,
        endpoint_id,
        method: method.to_string(),
        url: url.to_string(),
        status: Some(data.status),
        duration_ms: Some(data.duration_ms.round() as u64),
        request_summary_json: serde_json::json!({
            "method": method.to_string(),
            "url": url_template,
            "spec": spec_value,
        })
        .to_string(),
        response_summary_json: serde_json::json!({
            "status": data.status,
            "duration_ms": data.duration_ms,
            "size_bytes": data.size_bytes,
            "truncated": data.truncated,
            "content_type": data.content_type,
            "body": body_preview,
        })
        .to_string(),
        created_at: chrono::Utc::now(),
    }
}

/// 按字节上限截断字符串（保证字符边界），返回借用切片。
///
/// 上限内原样返回 `&s`（零分配）；超限时返回前缀切片——同样是借用，
/// 不再像返回 `String` 那样在 `<= max` 分支多做一次整体拷贝。
fn byte_truncate(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// 解析请求超时（毫秒）：接口级 `spec.timeout_ms` 优先，其次全局设置（走缓存），最后默认 300 秒。
pub(crate) async fn resolve_timeout_ms(spec: &RequestSpec, state: &AppState) -> CommandResult<u64> {
    if let Some(ms) = spec.timeout_ms {
        return Ok(ms);
    }
    Ok(state
        .global_settings()
        .await?
        .http_timeout_ms
        .unwrap_or(miyo_http::client::DEFAULT_TIMEOUT_MS))
}

/// 全局参数注入：将启用的全局参数并入请求规格（query → params，header → headers）。
///
/// 规则：请求自身已存在的同名键优先（全局参数只做补缺，不覆盖显式配置）。
/// key 大小写不敏感比较（Header 常见 `x-request-id` / `X-Request-Id` 混写）。
pub(crate) fn apply_global_params(
    spec: &mut RequestSpec,
    params: &[miyo_core::model::GlobalParam],
    vars: &VariableMap,
) {
    use miyo_core::model::GlobalParamLocation;

    for gp in params.iter().filter(|p| p.enabled) {
        if gp.key.trim().is_empty() {
            continue;
        }
        let key_lower = gp.key.trim().to_ascii_lowercase();
        let value = miyo_core::resolve_variables(&gp.value, vars);
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
    use miyo_core::model::{AuthSpec, BodySpec, GlobalParamLocation};
    use std::collections::HashMap;

    /// 脚本头回写：改写（大小写不敏感）、删除、新增、禁用行保留四路齐全。
    #[test]
    fn script_headers_reconcile_with_spec() {
        let mut disabled = KeyValue::new("X-Off", "0");
        disabled.enabled = false;
        let mut spec = RequestSpec {
            headers: vec![
                KeyValue::new("Authorization", "old"),
                KeyValue::new("X-Drop", "gone"),
                disabled,
            ],
            ..Default::default()
        };
        apply_script_headers(
            &mut spec,
            &[
                ("authorization".to_string(), "Bearer fresh".to_string()),
                ("X-Script".to_string(), "1".to_string()),
            ],
        );
        // 脚本改写（大小写不敏感匹配）、删除、新增、禁用行原样保留。
        let enabled: Vec<(String, String)> = spec
            .headers
            .iter()
            .filter(|kv| kv.enabled)
            .map(|kv| (kv.key.clone(), kv.value.clone()))
            .collect();
        assert_eq!(
            enabled,
            vec![
                ("authorization".to_string(), "Bearer fresh".to_string()),
                ("X-Script".to_string(), "1".to_string()),
            ]
        );
        assert!(spec
            .headers
            .iter()
            .any(|kv| !kv.enabled && kv.key == "X-Off"));
        let added = spec
            .headers
            .iter()
            .find(|kv| kv.key == "X-Script")
            .expect("脚本新增头应存在");
        assert_eq!(added.description, "脚本");
    }

    /// IPC 预览截断：小正文原样；超限截到 2MB 内；截断点落多字节字符时退字符边界。
    #[test]
    fn ipc_body_capped_at_preview_limit() {
        // 未超限：原样返回，无截断标记。
        let (body, omitted) = cap_ipc_body("hello");
        assert_eq!(body, "hello");
        assert!(!omitted);

        // 超限：截到上限并标记。
        let full = "a".repeat(IPC_BODY_PREVIEW_BYTES + 1024);
        let (body, omitted) = cap_ipc_body(&full);
        assert!(omitted);
        assert_eq!(body.len(), IPC_BODY_PREVIEW_BYTES);

        // 截断点横跨多字节字符：退回字符边界，不 panic、结果仍是合法 UTF-8。
        let mut big = "b".repeat(IPC_BODY_PREVIEW_BYTES - 1);
        big.push('中'); // 该字符横跨 2MB 边界（3 字节）
        big.push_str(&"c".repeat(64));
        let (body, omitted) = cap_ipc_body(&big);
        assert!(omitted);
        assert_eq!(body.len(), IPC_BODY_PREVIEW_BYTES - 1);
        assert!(body.ends_with('b'));
    }

    #[test]
    fn global_params_inject_query_and_header_fill_gaps() {
        use miyo_core::model::GlobalParam;

        let mut spec = RequestSpec {
            params: vec![KeyValue::new("keep", "1")],
            headers: vec![],
            ..Default::default()
        };
        let params = vec![
            GlobalParam {
                key: "debug".into(),
                value: "1".into(),
                enabled: true,
                location: GlobalParamLocation::Query,
            },
            // 已存在的同名（大小写不敏感）不覆盖
            GlobalParam {
                key: "KEEP".into(),
                value: "override".into(),
                enabled: true,
                location: GlobalParamLocation::Query,
            },
            GlobalParam {
                key: "X-Request-Id".into(),
                value: "{{rid}}".into(),
                enabled: true,
                location: GlobalParamLocation::Header,
            },
            // 禁用不注入
            GlobalParam {
                key: "off".into(),
                value: "x".into(),
                enabled: false,
                location: GlobalParamLocation::Header,
            },
        ];
        let mut vars = HashMap::new();
        vars.insert("rid".to_string(), "trace-9".to_string());
        apply_global_params(&mut spec, &params, &vars);

        assert_eq!(spec.params.len(), 2, "注入 debug；KEEP 已有同名不覆盖");
        assert!(spec
            .params
            .iter()
            .any(|kv| kv.key == "debug" && kv.value == "1"));
        assert_eq!(spec.params[0].value, "1", "请求显式值优先");
        assert_eq!(spec.headers.len(), 1);
        assert_eq!(spec.headers[0].key, "X-Request-Id");
        assert_eq!(
            spec.headers[0].value, "trace-9",
            "全局参数值支持 {{变量}} 解析"
        );
    }

    /// 历史列表字段与恢复模板分离：`url` 为渲染后地址（展示/搜索），
    /// `request_summary_json.url` 为渲染前模板（恢复保留环境变量），
    /// 且认证字段不得入库。
    #[test]
    fn history_summary_contains_spec_with_auth_stripped() {
        let spec = RequestSpec {
            params: vec![KeyValue::new("page", "1")],
            headers: vec![KeyValue::new("X-Token", "tok")],
            path_variables: vec![],
            auth: AuthSpec::Bearer {
                token: "secret-token".into(),
            },
            body: BodySpec::Json {
                raw: "{\"a\":1}".into(),
            },
            active_tab: None,
            timeout_ms: None,
            follow_redirects: true,
            tests: None,
            disable_cookies: false,
            body_docs: HashMap::new(),
            scripts: None,
        };
        let response = ExecuteResponse {
            status: 200,
            headers: vec![],
            body: "{}".into(),
            content_type: "application/json".into(),
            duration_ms: 12.5,
            size_bytes: 2,
            truncated: false,
            body_omitted: false,
            request: None,
            scripts: Vec::new(),
        };
        let history = build_history(
            Uuid::new_v4(),
            Some(Uuid::new_v4()),
            HttpMethod::POST,
            "https://api.example.com/users?page=1",
            "{{base_url}}/users?page=1",
            &spec,
            &response,
        );
        // 列表字段存渲染后真实地址；恢复编辑器走 summary 模板（保留环境语义）
        assert_eq!(history.url, "https://api.example.com/users?page=1");
        let summary: serde_json::Value =
            serde_json::from_str(&history.request_summary_json).unwrap();
        assert_eq!(summary["method"], "POST");
        assert_eq!(summary["url"], "{{base_url}}/users?page=1");
        assert_eq!(summary["spec"]["headers"][0]["key"], "X-Token");
        assert_eq!(summary["spec"]["body"]["mode"], "json");
        assert_eq!(summary["spec"]["params"][0]["value"], "1");
        // 认证凭据不得入库：auth 统一置空
        assert_eq!(summary["spec"]["auth"]["type"], "none");
        assert!(summary["spec"]["auth"].get("token").is_none());
    }
}
