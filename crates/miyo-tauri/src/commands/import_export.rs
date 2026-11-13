//! 导入导出 Command：OpenAPI 3.0 / Swagger 2.0 / Postman v2.1 导入（解析预览），
//! 项目接口导出为 OpenAPI 3.0 JSON，以及多格式文档导出（文档预览页）。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use tauri::State;
use uuid::Uuid;

use miyo_core::model::{Endpoint, EndpointStatus, ResponseExample};
use miyo_openapi::import::{import_any, ImportFormat};
use miyo_storage::repository as repo;

use crate::error::{CommandError, CommandErrorCode, CommandResult};
use crate::state::AppState;

/// 导入结果（前端先预览，确认后才落库）。
#[derive(Debug, Clone, Serialize)]
pub struct ImportResult {
    pub format: ImportFormat,
    pub endpoints: Vec<miyo_openapi::import::ImportedEndpoint>,
}

/// 解析文档文本：自动识别格式并提取接口（不落库）。
#[tauri::command(rename_all = "camelCase")]
pub async fn import_document(
    _state: State<'_, AppState>,
    text: String,
) -> CommandResult<ImportResult> {
    // OpenAPI/Postman 文本可达数 MB，serde 解析为纯 CPU 密集：
    // 挪到阻塞线程池，避免卡住 IPC 执行线程。
    let (endpoints, format) = tokio::task::spawn_blocking(move || import_any(&text))
        .await
        .map_err(|e| {
            CommandError::with_code(CommandErrorCode::Internal, format!("文档解析任务失败：{e}"))
        })?
        .map_err(CommandError::from)?;
    if endpoints.is_empty() {
        return Err(CommandError::validation("文档中没有可导入的接口"));
    }
    Ok(ImportResult { format, endpoints })
}

/// 读取本地文本文件内容（仪表板拖拽导入用：Tauri 文件拖放只提供路径）。
/// 上限 2MB；非 UTF-8 内容报 VALIDATION，提示改用粘贴导入。
#[tauri::command(rename_all = "camelCase")]
pub async fn read_text_file(path: String) -> CommandResult<String> {
    const MAX_LEN: u64 = 2 * 1024 * 1024;
    // 异步读：不在 IPC 线程上做同步文件 IO。
    let bytes = tokio::fs::read(&path).await.map_err(|e| {
        CommandError::with_code(CommandErrorCode::Io, format!("无法读取文件 {path}：{e}"))
    })?;
    if bytes.len() as u64 > MAX_LEN {
        return Err(CommandError::validation("文件超过 2MB，请改用粘贴文本导入"));
    }
    String::from_utf8(bytes)
        .map_err(|_| CommandError::validation("文件不是有效的 UTF-8 文本，请改用粘贴导入"))
}

/// 导出项目接口为 OpenAPI 3.0 JSON 文本（含响应示例）。
#[tauri::command(rename_all = "camelCase")]
pub async fn export_openapi(state: State<'_, AppState>, project_id: Uuid) -> CommandResult<String> {
    let project = repo::get_project(&state.db, project_id).await?;
    let mut endpoints = repo::list_endpoints(&state.db, project_id).await?;
    // 导出脱敏：auth / 敏感头 / body 凭证替换占位符（文档会离开本机）。
    for ep in &mut endpoints {
        let mut log = miyo_sync::redact::RedactionLog::default();
        miyo_sync::redact::redact_request_spec(&mut ep.request, &mut log, &ep.name);
    }

    // 批量一次查询（去 N+1）。
    let active_ids: Vec<Uuid> = endpoints
        .iter()
        .filter(|e| e.status != EndpointStatus::Deprecated)
        .map(|e| e.id)
        .collect();
    let examples_by_endpoint: HashMap<Uuid, Vec<miyo_core::model::ResponseExample>> =
        repo::list_response_examples_by_endpoints(&state.db, &active_ids)
            .await
            .unwrap_or_default();

    miyo_openapi::export::export_project(&project.name, &endpoints, &examples_by_endpoint)
        .map_err(CommandError::from)
}

// ---------- 文档导出（文档预览页「导出文档」） ----------

/// 文档导出格式（前端 ExportDocsDialog 卡片单选）。
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    /// OpenAPI 3.0 JSON
    OpenapiJson,
    /// OpenAPI 3.0 YAML
    OpenapiYaml,
    /// Postman Collection v2.1
    Postman,
    /// Markdown 文档
    Markdown,
    /// HTML 离线单页
    Html,
    /// cURL 命令脚本 (.sh)
    CurlScript,
    /// 冒烟测试文档 (Markdown)
    Smoke,
}

impl ExportFormat {
    /// 默认文件扩展名。
    fn ext(self) -> &'static str {
        match self {
            ExportFormat::OpenapiJson | ExportFormat::Postman => "json",
            ExportFormat::OpenapiYaml => "yaml",
            ExportFormat::Markdown | ExportFormat::Smoke => "md",
            ExportFormat::Html => "html",
            ExportFormat::CurlScript => "sh",
        }
    }

    /// 文件名种类前缀：扩展名无法区分语义的格式才加（md / html 靠后缀已可辨识）。
    fn slug(self) -> Option<&'static str> {
        match self {
            ExportFormat::OpenapiJson | ExportFormat::OpenapiYaml => Some("openapi"),
            ExportFormat::Postman => Some("postman"),
            ExportFormat::CurlScript => Some("curl"),
            ExportFormat::Smoke => Some("smoke"),
            ExportFormat::Markdown | ExportFormat::Html => None,
        }
    }
}

/// 导出结果：文档内容 + 建议文件名（前端据此唤起原生保存框）+
/// 被脱敏字段清单（导出前预览：用户据此知道哪些值需要自行回填）。
#[derive(Debug, Clone, Serialize)]
pub struct ExportedDoc {
    pub content: String,
    pub suggested_name: String,
    /// 导出中被脱敏的项（如「接口 登录 的 Bearer Token」）；空 = 无敏感值。
    #[serde(default)]
    pub redacted: Vec<String>,
}

/// 文件名安全化：系统保留字符替换为 `-`、去空白与首尾点、收敛连续 `-`；
/// 超长按字符截断（中文安全）；清空后回退 `export`。
fn sanitize_filename(input: &str) -> String {
    const FORBIDDEN: [char; 9] = ['/', '\\', ':', '*', '?', '"', '<', '>', '|'];
    let mut out = String::with_capacity(input.len());
    let mut prev_dash = false;
    for ch in input.trim().chars() {
        let c = if FORBIDDEN.contains(&ch) || ch.is_whitespace() {
            '-'
        } else {
            ch
        };
        if c == '-' && prev_dash {
            continue;
        }
        prev_dash = c == '-';
        out.push(c);
    }
    // 去掉首尾连字符（trim 后可能因替换产生）
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        return "export".into();
    }
    // 保守长度上限（60 个字符，含中文按字符计）
    let mut capped: String = trimmed.chars().take(60).collect();
    while capped.ends_with('-') || capped.ends_with('.') {
        capped.pop();
    }
    if capped.is_empty() {
        return "export".into();
    }
    capped
}

/// 建议文件名：`[kind-]{名称}-{YYYY-MM-DD}.{ext}`。
///
/// - 项目范围 base 取项目名；单接口范围取接口名（为空回退 `method-path`）；
/// - 例：`openapi-演示项目-2026-08-23.json`、`postman-创建订单-2026-08-23.json`、
///   `创建订单-2026-08-23.md`、`curl-演示项目-2026-08-23.sh`。
fn build_suggested_name(
    format: ExportFormat,
    base: &str,
    now: chrono::DateTime<chrono::Local>,
) -> String {
    let date = now.format("%Y-%m-%d");
    match format.slug() {
        Some(kind) => format!("{kind}-{}-{date}.{}", sanitize_filename(base), format.ext()),
        None => format!("{}-{date}.{}", sanitize_filename(base), format.ext()),
    }
}

/// 多格式文档导出。
///
/// - `endpoint_id` 为 Some 时仅导出该接口，None 时导出整个项目；
/// - 已废弃接口与 `export_openapi` 保持一致被排除；
/// - 响应示例随接口一并导出（OpenAPI / Postman / HTML 消费）。
#[tauri::command(rename_all = "camelCase")]
pub async fn export_docs(
    state: State<'_, AppState>,
    project_id: Uuid,
    endpoint_id: Option<Uuid>,
    format: ExportFormat,
) -> CommandResult<ExportedDoc> {
    let project = repo::get_project(&state.db, project_id).await?;
    let mut endpoints: Vec<Endpoint> = repo::list_endpoints(&state.db, project_id)
        .await?
        .into_iter()
        .filter(|e| e.status != EndpointStatus::Deprecated)
        .collect();
    if let Some(id) = endpoint_id {
        endpoints.retain(|e| e.id == id);
        if endpoints.is_empty() {
            return Err(CommandError::validation("当前接口不存在或已被删除"));
        }
    }

    // 导出脱敏：文档 / 脚本会离开本机，auth / 敏感头 / body 凭证一律占位符；
    // 被脱敏的字段清单随 ExportedDoc 返回（导出预览提示）。
    let mut redacted = miyo_sync::redact::RedactionLog::default();
    for ep in &mut endpoints {
        miyo_sync::redact::redact_request_spec(&mut ep.request, &mut redacted, &ep.name);
    }
    // 批量一次查询（去 N+1）；空列表不上 map，保持原有"无示例不占位"语义。
    let all_ids: Vec<Uuid> = endpoints.iter().map(|e| e.id).collect();
    let mut examples_by_endpoint: HashMap<Uuid, Vec<ResponseExample>> =
        repo::list_response_examples_by_endpoints(&state.db, &all_ids)
            .await
            .unwrap_or_default();
    examples_by_endpoint.retain(|_, v| !v.is_empty());
    for examples in examples_by_endpoint.values_mut() {
        for ex in examples {
            for (k, v) in ex.headers.iter_mut() {
                if miyo_sync::redact::is_sensitive_key(k) && !v.is_empty() {
                    *v = miyo_sync::redact::PLACEHOLDER.to_string();
                    redacted
                        .entries
                        .push(format!("响应示例 {} 的响应头 {}", ex.name, k));
                }
            }
        }
    }

    let content = render_docs(format, &project.name, &endpoints, &examples_by_endpoint)?;

    // 命名基准：单接口 → 接口名（空名回退 method-path）；整个项目 → 项目名
    let base = match endpoint_id {
        Some(_) => {
            let ep = &endpoints[0];
            if ep.name.trim().is_empty() {
                format!("{}-{}", ep.method.as_str().to_lowercase(), ep.path)
            } else {
                ep.name.clone()
            }
        }
        None => project.name.clone(),
    };
    // 同名去重后排序，预览清单稳定。
    let mut redacted_entries = redacted.entries;
    redacted_entries.sort();
    redacted_entries.dedup();
    Ok(ExportedDoc {
        content,
        suggested_name: build_suggested_name(format, &base, chrono::Local::now()),
        redacted: redacted_entries,
    })
}

fn render_docs(
    format: ExportFormat,
    project_name: &str,
    endpoints: &[Endpoint],
    examples: &HashMap<Uuid, Vec<ResponseExample>>,
) -> CommandResult<String> {
    match format {
        ExportFormat::OpenapiJson => {
            miyo_openapi::export::export_project(project_name, endpoints, examples)
                .map_err(CommandError::from)
        }
        // 结构值直转 YAML（原来 JSON 文本 → Value → YAML 双重编解码）。
        ExportFormat::OpenapiYaml => {
            let value =
                miyo_openapi::export::export_project_value(project_name, endpoints, examples)
                    .map_err(CommandError::from)?;
            serde_norway::to_string(&value)
                .map_err(|e| CommandError::with_code(CommandErrorCode::Export, e.to_string()))
        }
        ExportFormat::Postman => Ok(miyo_openapi::postman_export::export_postman(
            project_name,
            endpoints,
            examples,
        )),
        ExportFormat::Markdown => Ok(miyo_openapi::markdown::export_markdown(
            project_name,
            endpoints,
            examples,
        )),
        ExportFormat::Html => Ok(miyo_openapi::html::export_html(
            project_name,
            endpoints,
            examples,
        )),
        ExportFormat::CurlScript => Ok(render_curl_script(endpoints)),
        // 冒烟文档不走 export_docs（走独立的 export_smoke_docs，需要测试用例数据）
        ExportFormat::Smoke => Err(CommandError::validation(
            "冒烟测试文档请使用测试用例导出入口",
        )),
    }
}

/// 全部接口 → 可直接执行的 cURL 脚本（每接口一段注释 + 一条命令）。
/// 凭证脱敏：真实 Token / 密码不写入脚本，敏感值以 `${MIYO_*}` 环境变量
/// 引用占位（导出前预览清单由 export_docs 的 `redacted` 字段返回）。
fn render_curl_script(endpoints: &[Endpoint]) -> String {
    use miyo_codegen::{render_export, GenRequest, Lang};

    let mut out = String::from("#!/bin/sh\n");
    out.push_str("# Miyo 导出的 cURL 脚本（敏感值已替换为环境变量引用）\n");
    out.push_str(&format!(
        "# 共 {} 个接口 · 生成于 {}\n\n",
        endpoints.len(),
        chrono::Local::now().format("%Y-%m-%d %H:%M")
    ));

    for (idx, ep) in endpoints.iter().enumerate() {
        out.push_str(&format!(
            "# {}. {} {}\n",
            idx + 1,
            ep.method.as_str(),
            ep.name
        ));
        let req = GenRequest {
            method: &ep.method,
            url: &ep.path,
            headers: &ep.request.headers,
            body: &ep.request.body,
            auth: &ep.request.auth,
        };
        out.push_str(&render_export(Lang::Curl, &req));
        out.push_str("\n\n");
    }
    out
}

/// 导出测试用例为冒烟测试文档（Markdown）。
///
/// - `endpoint_id` 为 Some 时仅导出该接口的用例，None 时导出整个项目；
/// - `include_results` 为 true 时在文档中附带用例最近一次运行结果；
/// - `run_results` 为前端内存态运行元信息（caseId → HTTP 状态码 / 耗时），
///   命中时优先展示状态码与耗时，缺失回退 `TestCase.last_run_status`；
/// - 已废弃接口与 `export_docs` 保持一致被排除；
/// - 文档结构：测试范围 / 按分组（正向·负向·边界值·安全性·其他）用例详情 / 验收清单。
#[tauri::command(rename_all = "camelCase")]
pub async fn export_smoke_docs(
    state: State<'_, AppState>,
    project_id: Uuid,
    endpoint_id: Option<Uuid>,
    include_results: bool,
    run_results: Option<HashMap<Uuid, miyo_openapi::smoke::SmokeRunResult>>,
) -> CommandResult<ExportedDoc> {
    let project = repo::get_project(&state.db, project_id).await?;
    let mut endpoints: Vec<Endpoint> = repo::list_endpoints(&state.db, project_id)
        .await?
        .into_iter()
        .filter(|e| e.status != EndpointStatus::Deprecated)
        .collect();
    if let Some(id) = endpoint_id {
        endpoints.retain(|e| e.id == id);
        if endpoints.is_empty() {
            return Err(CommandError::validation("当前接口不存在或已被删除"));
        }
    }

    // 导出脱敏（与 export_docs 同口径）。
    let mut redacted = miyo_sync::redact::RedactionLog::default();
    for ep in &mut endpoints {
        miyo_sync::redact::redact_request_spec(&mut ep.request, &mut redacted, &ep.name);
    }
    let mut cases_by_endpoint: HashMap<Uuid, Vec<miyo_core::model::TestCase>> = HashMap::new();
    // 批量一次查询（去 N+1：E 个接口原来 E 次往返）；错误退化为无用例，
    // 与原实现「单接口查询失败仅跳过该接口」保持一致的可用性。
    let all_case_ids: Vec<Uuid> = endpoints.iter().map(|e| e.id).collect();
    for mut case in repo::list_test_cases_by_endpoints(&state.db, &all_case_ids)
        .await
        .unwrap_or_default()
    {
        miyo_sync::redact::redact_test_case(&mut case);
        cases_by_endpoint
            .entry(case.request_id)
            .or_default()
            .push(case);
    }

    let run_results = run_results.unwrap_or_default();
    let content = miyo_openapi::smoke::render_smoke(
        &project.name,
        &endpoints,
        &cases_by_endpoint,
        include_results,
        &run_results,
    );

    // 命名基准：单接口 → 接口名（空名回退 method-path）；整个项目 → 项目名
    let base = match endpoint_id {
        Some(_) => {
            let ep = &endpoints[0];
            if ep.name.trim().is_empty() {
                format!("{}-{}", ep.method.as_str().to_lowercase(), ep.path)
            } else {
                ep.name.clone()
            }
        }
        None => project.name.clone(),
    };
    let mut redacted_entries = redacted.entries;
    redacted_entries.sort();
    redacted_entries.dedup();
    Ok(ExportedDoc {
        content,
        suggested_name: build_suggested_name(ExportFormat::Smoke, &base, chrono::Local::now()),
        redacted: redacted_entries,
    })
}

/// 把导出内容写入磁盘指定路径（路径来自前端原生保存框的选择结果）。
#[tauri::command(rename_all = "camelCase")]
pub async fn save_text_file(path: String, contents: String) -> CommandResult<()> {
    // 异步写入：不阻塞 IPC 执行线程（大文档导出可达数 MB）。
    tokio::fs::write(&path, contents)
        .await
        .map_err(|e| CommandError::with_code(CommandErrorCode::Io, format!("写入文件失败：{e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use miyo_core::model::{BodySpec, HttpMethod, KeyValue, RequestSpec};

    fn sample_endpoint() -> Endpoint {
        Endpoint {
            id: Uuid::new_v4(),
            project_id: Uuid::new_v4(),
            folder_id: None,
            name: "创建订单".into(),
            method: HttpMethod::POST,
            path: "/api/v1/orders".into(),
            description: "下单".into(),
            status: EndpointStatus::Released,
            sort_order: 0,
            request: RequestSpec {
                headers: vec![KeyValue {
                    key: "X-Trace".into(),
                    value: "t".into(),
                    enabled: true,
                    description: String::new(),
                    field_type: Default::default(),
                    required: true,
                    example: String::new(),
                }],
                body: BodySpec::Json {
                    raw: "{\"sku\":\"a\"}".into(),
                },
                ..RequestSpec::default()
            },
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn examples_of(ep: &Endpoint) -> HashMap<Uuid, Vec<ResponseExample>> {
        let mut map = HashMap::new();
        map.insert(
            ep.id,
            vec![ResponseExample {
                id: Uuid::new_v4(),
                endpoint_id: ep.id,
                name: "成功".into(),
                status: 200,
                headers: Default::default(),
                body: "{\"code\":0}".into(),
                content_type: "application/json".into(),
                docs: Default::default(),
                created_at: Utc::now(),
                updated_at: Utc::now(),
            }],
        );
        map
    }

    #[test]
    fn render_docs_all_formats() {
        let ep = sample_endpoint();
        let eps = vec![ep.clone()];
        let examples = examples_of(&ep);

        // OpenAPI JSON：可解析且为 3.x 版本
        let json = render_docs(ExportFormat::OpenapiJson, "P", &eps, &examples).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(v["openapi"].as_str().unwrap().starts_with('3'));

        // OpenAPI YAML：可解析回同一结构
        let yaml = render_docs(ExportFormat::OpenapiYaml, "P", &eps, &examples).unwrap();
        let yv: serde_norway::Value = serde_norway::from_str(&yaml).unwrap();
        assert!(yv.get("openapi").is_some());

        // Postman：Collection v2.1 schema
        let postman = render_docs(ExportFormat::Postman, "P", &eps, &examples).unwrap();
        assert!(postman.contains("collection/v2.1.0"));

        // Markdown：标题开头 + 接口名
        let md = render_docs(ExportFormat::Markdown, "演示项目", &eps, &examples).unwrap();
        assert!(md.starts_with("# 演示项目"));
        assert!(md.contains("创建订单"));

        // HTML：单页文档，含接口卡片，无脚本
        let html = render_docs(ExportFormat::Html, "P", &eps, &examples).unwrap();
        assert!(html.starts_with("<!DOCTYPE html>"));
        assert!(html.contains("/api/v1/orders"));

        // cURL 脚本：shebang + curl 命令 + body 数据
        let sh = render_docs(ExportFormat::CurlScript, "P", &eps, &examples).unwrap();
        assert!(sh.starts_with("#!/bin/sh"));
        assert!(sh.contains("curl -X POST '/api/v1/orders'"));
        assert!(sh.contains("--data"));
    }

    #[test]
    fn curl_script_contains_body_and_header() {
        let ep = sample_endpoint();
        let sh = render_curl_script(std::slice::from_ref(&ep));
        assert!(sh.contains("-H 'X-Trace: t'"));
        // sq() 只转义单引号：JSON 双引号原样保留
        assert!(sh.contains("--data '{\"sku\":\"a\"}'"));
    }

    /// 导出 curl 脚本：auth / 敏感头凭证必须以环境变量引用占位（默认脱敏）。
    #[test]
    fn curl_script_masks_credentials() {
        use miyo_core::model::{AuthSpec, KeyValue};
        let mut ep = sample_endpoint();
        ep.request.headers.push(KeyValue {
            key: "Authorization".into(),
            value: "Bearer real-tok-1".into(),
            enabled: true,
            description: String::new(),
            field_type: Default::default(),
            required: true,
            example: String::new(),
        });
        ep.request.auth = AuthSpec::Bearer {
            token: "real-tok-2".into(),
        };
        let sh = render_curl_script(std::slice::from_ref(&ep));
        assert!(
            !sh.contains("real-tok-1") && !sh.contains("real-tok-2"),
            "凭证不得明文导出：{sh}"
        );
        assert!(sh.contains("${MIYO_AUTHORIZATION}"));
        assert!(sh.contains("-H 'X-Trace: t'"), "非敏感头保留");
    }

    #[test]
    fn ext_mapping_covers_all_formats() {
        assert_eq!(ExportFormat::OpenapiJson.ext(), "json");
        assert_eq!(ExportFormat::OpenapiYaml.ext(), "yaml");
        assert_eq!(ExportFormat::Postman.ext(), "json");
        assert_eq!(ExportFormat::Markdown.ext(), "md");
        assert_eq!(ExportFormat::Html.ext(), "html");
        assert_eq!(ExportFormat::CurlScript.ext(), "sh");
        assert_eq!(ExportFormat::Smoke.ext(), "md");
    }

    #[test]
    fn sanitize_filename_replaces_forbidden_chars() {
        assert_eq!(sanitize_filename("订单/支付:v2"), "订单-支付-v2");
        assert_eq!(sanitize_filename("a*b?c\"d<e>f|g"), "a-b-c-d-e-f-g");
        assert_eq!(sanitize_filename("a\\b"), "a-b");
    }

    #[test]
    fn sanitize_filename_trims_and_collapses() {
        // 空白收敛为连字符并去重；首尾连字符 / 点被移除
        assert_eq!(sanitize_filename("  演示   项目  "), "演示-项目");
        assert_eq!(sanitize_filename("--name--.md.."), "name-.md");
        assert_eq!(sanitize_filename("-*-"), "export");
        assert_eq!(sanitize_filename("   "), "export");
        assert_eq!(sanitize_filename(""), "export");
    }

    #[test]
    fn sanitize_filename_caps_length_by_chars() {
        let long = "测".repeat(100);
        let out = sanitize_filename(&long);
        assert_eq!(out.chars().count(), 60);
        assert!(!out.ends_with('-'));
    }

    #[test]
    fn suggested_name_matches_scope_and_format() {
        use chrono::TimeZone;
        let now = chrono::Local
            .with_ymd_and_hms(2026, 8, 23, 12, 0, 0)
            .unwrap();

        // 项目范围：项目名为基准
        assert_eq!(
            build_suggested_name(ExportFormat::OpenapiJson, "演示项目", now),
            "openapi-演示项目-2026-08-23.json"
        );
        assert_eq!(
            build_suggested_name(ExportFormat::OpenapiYaml, "演示项目", now),
            "openapi-演示项目-2026-08-23.yaml"
        );
        assert_eq!(
            build_suggested_name(ExportFormat::Postman, "演示项目", now),
            "postman-演示项目-2026-08-23.json"
        );
        assert_eq!(
            build_suggested_name(ExportFormat::CurlScript, "演示项目", now),
            "curl-演示项目-2026-08-23.sh"
        );
        assert_eq!(
            build_suggested_name(ExportFormat::Smoke, "演示项目", now),
            "smoke-演示项目-2026-08-23.md"
        );

        // 单接口范围：接口名为基准，md/html 不加种类前缀（后缀已可辨识）
        assert_eq!(
            build_suggested_name(ExportFormat::Markdown, "创建订单", now),
            "创建订单-2026-08-23.md"
        );
        assert_eq!(
            build_suggested_name(ExportFormat::Html, "创建订单", now),
            "创建订单-2026-08-23.html"
        );

        // 名称含非法字符时被安全化
        assert_eq!(
            build_suggested_name(ExportFormat::Markdown, "A/B:测试", now),
            "A-B-测试-2026-08-23.md"
        );
    }

    #[tokio::test]
    async fn save_text_file_writes_content() {
        let dir = std::env::temp_dir().join(format!("miyo-export-test-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("out.md");
        let path_str = path.to_string_lossy().to_string();

        save_text_file(path_str.clone(), "# hello".into())
            .await
            .unwrap();

        let written = std::fs::read_to_string(&path).unwrap();
        assert_eq!(written, "# hello");

        std::fs::remove_dir_all(&dir).ok();
    }
}
