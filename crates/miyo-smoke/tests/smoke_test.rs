//! Miyo 端到端冒烟测试（无 UI，纯逻辑联调）。
//!
//! 覆盖四个里程碑链路：
//! 1. 创建项目 → 创建环境 → 创建接口 → 发送请求 → 查看响应 → 保存历史 → 运行测试；
//! 2. 导出 OpenAPI → 导入 → 验证数据一致；
//! 3. 启动 Mock → HTTP 请求验证 → 停止 Mock；
//! 4. 备份项目 → 恢复项目 → 验证数据一致。

use std::collections::HashMap;

use chrono::Utc;
use miyo_backup::{build_backup, restore_backup, BackupFile, BackupInput};
use miyo_core::model::{
    BodySpec, HttpMethod, KeyValue, RequestExample, RequestHistory, ResponseExample, TestCase,
    TestCaseStatus, TestRun,
};
use miyo_core::variable::{resolve_variables_with, ResolveOptions};
use miyo_http::client::send_request;
use miyo_mock::server::{self, MockDefinition, MockStore};
use miyo_openapi::export::export_project;
use miyo_openapi::import::{import_any, ImportFormat};
use miyo_storage::db::memory_pool;
use miyo_storage::repository as repo;
use miyo_test::runner::run_endpoint;
use serde_json::json;
use sqlx::SqlitePool;

async fn setup_pool() -> SqlitePool {
    memory_pool().await.expect("创建内存数据库")
}

// ---------- 链路 1：完整用户流程（含链路 3 的 Mock 验证） ----------

#[tokio::test]
async fn full_user_flow() {
    let db = setup_pool().await;

    // 1. 创建项目。
    let project = repo::create_project(&db, "演示项目", "冒烟测试")
        .await
        .unwrap();

    // 2. 创建环境（变量稍后通过 update_environment 写入）。
    let mut env = repo::create_environment(&db, project.id, "本地", "", &[])
        .await
        .unwrap();

    // 3. 创建接口（GET /api/hello + 测试断言：状态码 200、body 包含 hello）。
    let mut ep = repo::create_endpoint(&db, project.id, None, "打招呼")
        .await
        .unwrap();
    ep.path = "/api/hello".into();
    ep.request.tests = Some(json!({
        "assertions": [
            {"name": "状态码应为 200", "type": "status", "op": "eq", "expected": 200},
            {"type": "jsonpath", "path": "$.message", "op": "contains", "expected": "hello"}
        ]
    }));
    let ep = repo::update_endpoint(&db, &ep).await.unwrap();

    // ---- 启动 Mock（链路 3 第一段）----
    let mut def = MockDefinition::from_endpoint(ep.method.as_str(), &ep.path, None);
    def.body_template = "{\"message\":\"hello from mock\",\"code\":0}".into();
    let store = MockStore::new();
    store.set_definitions(vec![def]);
    let server_mock = server::start(store).await.expect("Mock 服务启动失败");
    let base_url = server_mock.address();

    // 环境：Base URL 指向 Mock。
    env.base_url = base_url.clone();
    repo::update_environment(&db, &env).await.unwrap();
    // 后端环境解析应命中基址。
    assert_eq!(env.base_url, base_url);

    // 合并变量（模拟工作区 merged_vars：项目变量 < 环境变量）。
    let mut vars = HashMap::<String, String>::new();
    vars.insert("base_url".into(), base_url);
    let url = resolve_variables_with(
        "{{base_url}}/api/hello",
        &vars,
        120,
        ResolveOptions::default(),
    );
    assert!(url.starts_with("http://127.0.0.1"), "url: {url}");

    // 4. 发送请求。
    let spec = ep.request.clone();
    let res = send_request(ep.method, &url, &spec, None)
        .await
        .expect("发送请求失败");
    assert_eq!(res.status, 200);
    assert!(
        res.body_text().contains("hello from mock"),
        "响应体: {}",
        res.body_text()
    );

    // 5. 保存历史。
    let history = RequestHistory {
        id: uuid::Uuid::new_v4(),
        project_id: project.id,
        endpoint_id: Some(ep.id),
        method: ep.method.to_string(),
        url: url.clone(),
        status: Some(res.status),
        duration_ms: Some(res.duration_ms.round() as u64),
        request_summary_json: json!({"method": ep.method.to_string(), "path": ep.path}).to_string(),
        response_summary_json: json!({
            "status": res.status,
            "duration_ms": res.duration_ms,
            "content_type": res.content_type(),
        })
        .to_string(),
        created_at: Utc::now(),
    };
    repo::save_request_history(&db, &history).await.unwrap();
    let rows = repo::list_request_histories(&db, project.id, None, 10)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1, "历史应已保存");
    assert_eq!(rows[0].url, url);

    // 6. 运行测试（配置有 2 条断言）。
    let mut runtime_vars = HashMap::<String, String>::new();
    let (result, _resp) = run_endpoint(&ep, &url, &spec, &mut runtime_vars, None).await;
    assert!(result.ok, "测试应通过: {:?}", result.request_error);
    assert_eq!(result.status, Some(200));
    assert_eq!(result.outcomes.len(), 2, "应有 2 条断言明细");

    // 运行结果入库（测试历史）。
    let run = TestRun {
        id: uuid::Uuid::new_v4(),
        project_id: project.id,
        environment_id: Some(env.id),
        name: "接口测试".into(),
        result_json: serde_json::to_string(&json!({
            "total": 1, "passed": 1, "failed": 0, "skipped": 0,
            "rows": [{"name": "打招呼", "ok": true}]
        }))
        .unwrap(),
        started_at: Utc::now() - chrono::Duration::seconds(30),
        finished_at: Some(Utc::now()),
    };
    repo::save_test_run(&db, &run).await.unwrap();
    let runs = repo::list_test_runs(&db, project.id, 20).await.unwrap();
    assert_eq!(runs.len(), 1, "测试历史应已保存");

    // ---- 停止 Mock（链路 3 收尾）----
    server_mock.stop().await;
}

// ---------- 链路 2 + 4：OpenAPI 导出/导入、备份/恢复 ----------

#[tokio::test]
async fn openapi_roundtrip_and_backup() {
    let db = setup_pool().await;
    let project = repo::create_project(&db, "接口示例", "").await.unwrap();

    let folder = repo::create_folder(&db, project.id, None, "用户")
        .await
        .unwrap();
    let mut ep = repo::create_endpoint(&db, project.id, Some(folder.id), "创建用户")
        .await
        .unwrap();
    ep.method = HttpMethod::POST;
    ep.path = "/users".into();
    ep.description = "创建用户".into();
    ep.request.params.push(KeyValue::new("debug", "1"));
    ep.request.body = BodySpec::Json {
        raw: "{\"name\":\"{{name}}\"}".into(),
    };
    let ep = repo::update_endpoint(&db, &ep).await.unwrap();

    let example = ResponseExample {
        id: uuid::Uuid::new_v4(),
        endpoint_id: ep.id,
        name: "成功".into(),
        status: 201,
        headers: HashMap::from([("x-id".into(), "7".into())]),
        body: "{\"id\":7,\"name\":\"tom\"}".into(),
        content_type: "application/json".into(),
        docs: HashMap::new(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    repo::create_response_example(&db, ep.id, &example)
        .await
        .unwrap();

    // ---- 请求用例：保存快照 → 列表（最新在前）→ 删除 ----
    let mut req_example_request = ep.request.clone();
    req_example_request.params.push(KeyValue::new("page", "2"));
    let req_example = RequestExample {
        id: uuid::Uuid::new_v4(),
        endpoint_id: ep.id,
        name: "分页查询".into(),
        request: req_example_request,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    repo::create_request_example(&db, &req_example)
        .await
        .unwrap();
    let mut req_example_request_2 = ep.request.clone();
    req_example_request_2
        .headers
        .push(KeyValue::new("X-Trace", "1"));
    let req_example_2 = RequestExample {
        id: uuid::Uuid::new_v4(),
        endpoint_id: ep.id,
        name: "带追踪头".into(),
        request: req_example_request_2,
        created_at: Utc::now() + chrono::Duration::seconds(5),
        updated_at: Utc::now() + chrono::Duration::seconds(5),
    };
    repo::create_request_example(&db, &req_example_2)
        .await
        .unwrap();
    let req_examples = repo::list_request_examples(&db, ep.id).await.unwrap();
    assert_eq!(req_examples.len(), 2);
    assert_eq!(req_examples[0].name, "带追踪头", "最新保存的应排在最前");
    assert_eq!(req_examples[0].request.headers[0].key, "X-Trace");
    assert_eq!(
        req_examples[1].request.params[1].value, "2",
        "请求快照应完整保留"
    );
    repo::delete_request_example(&db, req_example.id)
        .await
        .unwrap();
    let req_examples = repo::list_request_examples(&db, ep.id).await.unwrap();
    assert_eq!(req_examples.len(), 1);
    assert_eq!(req_examples[0].name, "带追踪头");

    // ---- 导出 → 导入 → 验证数据一致 ----
    let eps = repo::list_endpoints(&db, project.id).await.unwrap();
    let mut examples_map: HashMap<uuid::Uuid, Vec<ResponseExample>> = HashMap::new();
    for e in &eps {
        examples_map.insert(e.id, repo::list_response_examples(&db, e.id).await.unwrap());
    }
    let json = export_project(&project.name, &eps, &examples_map).expect("导出 OpenAPI");

    let (imported, format) = import_any(&json).unwrap();
    assert_eq!(
        format,
        ImportFormat::OpenApi30,
        "导出文档应识别为 OpenAPI 3.0"
    );
    assert_eq!(imported.len(), 1, "导出 1 个接口应导入 1 个");
    assert_eq!(imported[0].method, HttpMethod::POST);
    assert_eq!(imported[0].path, "/users");
    assert_eq!(imported[0].request.params[0].key, "debug");
    assert!(imported[0].examples.iter().any(|ex| ex.status == 201));

    // ---- 备份 → 恢复 → 验证数据一致 ----
    let folders = repo::list_folders(&db, project.id).await.unwrap();
    let envs = repo::list_environments(&db, project.id).await.unwrap();
    let rules = repo::list_mock_rules(&db, project.id).await.unwrap();
    let all_examples: Vec<ResponseExample> = examples_map.values().flatten().cloned().collect();
    let all_req_examples: Vec<RequestExample> =
        repo::list_request_examples(&db, ep.id).await.unwrap();

    let file = build_backup(&BackupInput {
        project: &project,
        folders: &folders,
        endpoints: &eps,
        environments: &envs,
        mock_rules: &rules,
        response_examples: &all_examples,
        request_examples: &all_req_examples,
        proto_files: &[],
        settings: &std::collections::HashMap::new(),
        global_variables: &[],
        global_params: &[],
        team: &None,
    });
    let text = file.serialize().expect("序列化备份");
    assert!(text.contains("miyo-project-backup"));

    let parsed = BackupFile::parse(&text).expect("解析备份");
    let restored = restore_backup(&parsed);
    assert_eq!(restored.project.name, project.name);
    assert_ne!(restored.project.id, project.id, "恢复应重映射为新项目 id");
    assert_eq!(restored.folders.len(), 1);
    assert_eq!(restored.endpoints.len(), 1);
    assert_eq!(restored.response_examples.len(), 1);
    assert_eq!(restored.request_examples.len(), 1, "请求用例应随备份恢复");

    // 恢复链路可直接落库并运行（验证引用关系正确）。
    repo::save_project(&db, &restored.project).await.unwrap();
    for f in &restored.folders {
        repo::save_folder(&db, f).await.unwrap();
    }
    for e in &restored.endpoints {
        repo::save_endpoint(&db, e).await.unwrap();
    }
    let saved_ep = &restored.endpoints[0];
    assert_eq!(saved_ep.method, HttpMethod::POST);
    assert_eq!(saved_ep.path, "/users");
    assert_eq!(saved_ep.folder_id, Some(restored.folders[0].id));

    let projects = repo::list_projects(&db).await.unwrap();
    assert_eq!(projects.len(), 2, "恢复的项目应已入库");
}

// ---------- 链路 5：cURL 导入（复现 bug 报告：导入后 URL/路径/请求头丢失） ----------

/// 镜像前端 `createFromCurl` 的 URL 拆分：query → params，pathname → path。
fn split_imported_url(url: &str) -> (String, Vec<KeyValue>) {
    let (path_part, query_part) = match url.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (url, None),
    };
    let params = query_part
        .map(|q| {
            q.split('&')
                .filter_map(|kv| kv.split_once('='))
                .map(|(k, v)| KeyValue::new(k.to_string(), v.to_string()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    // 去掉 `scheme://host`（镜像前端 `new URL(url).pathname`）。
    let path = match path_part.split_once("://") {
        Some((_, rest)) => rest.split_once('/').map(|(_, p)| p).unwrap_or(""),
        None => path_part,
    };
    let path = path.split('/').collect::<Vec<_>>().join("/");
    let path = if path.starts_with('/') {
        path
    } else {
        format!("/{path}")
    };
    (path, params)
}

#[tokio::test]
async fn curl_import_roundtrip_keeps_url_headers_body() {
    use chrono::Utc;
    use miyo_core::curl_parser::parse_curl;
    use miyo_core::model::{BodySpec, Endpoint};
    use uuid::Uuid;

    let db = setup_pool().await;
    let project = repo::create_project(&db, "导入测试", "cURL 导入链路")
        .await
        .unwrap();

    // 用户报告中的命令（含续行、中文 JSON body）。
    let cmd = "curl -X POST https://jsonplaceholder.typicode.com/posts?userId=1 \
        -H 'Content-Type: application/json' -d '{\"title\":\"测试标题\",\"userId\":1}'";
    let parsed = parse_curl(cmd).expect("解析用户命令");
    assert_eq!(parsed.method, HttpMethod::POST);
    assert_eq!(
        parsed.url,
        "https://jsonplaceholder.typicode.com/posts?userId=1"
    );
    assert_eq!(parsed.headers.len(), 1);
    assert_eq!(parsed.headers[0].key, "Content-Type");

    // 镜像前端 createFromCurl：拆分 URL → path + params，组装 Endpoint。
    let (path, params) = split_imported_url(&parsed.url);
    let now = Utc::now();
    let endpoint = Endpoint {
        id: Uuid::new_v4(),
        project_id: project.id,
        folder_id: None,
        name: "posts".into(),
        method: parsed.method,
        path,
        description: String::new(),
        status: Default::default(),
        sort_order: 0,
        request: miyo_core::model::RequestSpec {
            params,
            headers: parsed.headers,
            path_variables: vec![],
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

    repo::save_endpoint(&db, &endpoint).await.expect("落库");
    let listed = repo::list_endpoints(&db, project.id).await.unwrap();
    assert_eq!(listed.len(), 1);
    let saved = &listed[0];

    // 核心断言：路径、方法、查询参数、请求头、JSON body 全部保留。
    assert_eq!(saved.path, "/posts");
    assert_eq!(saved.method, HttpMethod::POST);
    assert_eq!(saved.request.params.len(), 1, "查询参数应保留");
    assert_eq!(saved.request.params[0].key, "userId");
    assert_eq!(saved.request.params[0].value, "1");
    assert_eq!(saved.request.headers.len(), 1);
    assert_eq!(saved.request.headers[0].key, "Content-Type");
    assert_eq!(saved.request.headers[0].value, "application/json");
    match &saved.request.body {
        BodySpec::Json { raw } => assert!(raw.contains("测试标题"), "JSON body 应保留，实际 {raw}"),
        other => panic!("期望 JSON body，实际 {other:?}"),
    }

    // 复现 bug 报告：编辑后再次保存（同 id upsert）不应报主键冲突。
    let mut updated = endpoint.clone();
    updated.path = "/posts/1".into();
    updated
        .request
        .headers
        .push(KeyValue::new("X-Custom".to_string(), "v2".to_string()));
    repo::save_endpoint(&db, &updated)
        .await
        .expect("同 id 重复保存应成功（upsert）");
    let relisted = repo::list_endpoints(&db, project.id).await.unwrap();
    assert_eq!(relisted.len(), 1, "upsert 不应产生新行");
    assert_eq!(relisted[0].path, "/posts/1", "upsert 应更新路径");
    assert_eq!(relisted[0].request.headers.len(), 2, "upsert 应更新请求头");
}

// ---------- 链路 6：测试用例管理（Apifox 风格 CRUD + 运行状态 + 级联删除） ----------

#[tokio::test]
async fn test_case_management_flow() {
    use miyo_core::model::{Endpoint, RequestSpec};
    use uuid::Uuid;

    let db = setup_pool().await;
    let project = repo::create_project(&db, "用例管理", "测试用例链路")
        .await
        .unwrap();
    let new_endpoint: Endpoint = Endpoint {
        id: Uuid::new_v4(),
        project_id: project.id,
        folder_id: None,
        name: "资金调拨".into(),
        method: HttpMethod::POST,
        path: "/funds/transfer".into(),
        description: String::new(),
        status: Default::default(),
        sort_order: 0,
        request: RequestSpec {
            params: vec![KeyValue::new("env".to_string(), "prod".to_string())],
            headers: vec![],
            path_variables: vec![],
            auth: Default::default(),
            body: BodySpec::Json {
                raw: "{\"amount\":100}".into(),
            },
            active_tab: None,
            timeout_ms: None,
            follow_redirects: true,
            tests: None,
            disable_cookies: false,
            body_docs: std::collections::HashMap::new(),
            scripts: None,
        },
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    repo::save_endpoint(&db, &new_endpoint)
        .await
        .expect("落库接口");
    let endpoint = repo::list_endpoints(&db, project.id)
        .await
        .unwrap()
        .into_iter()
        .next()
        .expect("应能查到接口");

    let now = Utc::now();
    let case = TestCase {
        id: Uuid::new_v4(),
        request_id: endpoint.id,
        name: "正向-内部划转-SGB".into(),
        category: "正向".into(),
        method: HttpMethod::POST,
        url_path: "/funds/transfer".into(),
        path_variables: vec![],
        params: vec![KeyValue::new("env".to_string(), "prod".to_string())],
        headers: vec![],
        body_type: "json".into(),
        body_content: "{\"amount\":100}".into(),
        last_run_status: TestCaseStatus::Untested,
        created_at: now,
    };

    let created = repo::create_test_case(&db, &case).await.expect("创建用例");
    assert_eq!(created.name, case.name);
    assert_eq!(created.category, "正向");

    let cases = repo::list_test_cases(&db, endpoint.id).await.unwrap();
    assert_eq!(cases.len(), 1);

    // 更新元信息（改名 + 换分组）与运行状态。
    repo::update_test_case_meta(&db, case.id, "负向-金额超限", "边界值")
        .await
        .unwrap();
    repo::update_test_case_status(&db, case.id, TestCaseStatus::Success)
        .await
        .unwrap();
    let updated = repo::list_test_cases(&db, endpoint.id).await.unwrap();
    assert_eq!(updated[0].name, "负向-金额超限");
    assert_eq!(updated[0].category, "边界值");
    assert_eq!(updated[0].last_run_status, TestCaseStatus::Success);

    // 删除用例。
    repo::delete_test_case(&db, case.id).await.unwrap();
    assert!(repo::list_test_cases(&db, endpoint.id)
        .await
        .unwrap()
        .is_empty());

    // 级联删除：接口删除后用例随之删除。
    let case2 = TestCase {
        id: Uuid::new_v4(),
        request_id: endpoint.id,
        name: "级联验证".into(),
        category: "其他".into(),
        method: HttpMethod::POST,
        url_path: "/funds/transfer".into(),
        path_variables: vec![],
        params: vec![],
        headers: vec![],
        body_type: "none".into(),
        body_content: String::new(),
        last_run_status: TestCaseStatus::Failed,
        created_at: Utc::now(),
    };
    repo::create_test_case(&db, &case2).await.unwrap();
    repo::delete_endpoint(&db, endpoint.id).await.unwrap();
    assert!(
        repo::list_test_cases(&db, endpoint.id)
            .await
            .unwrap()
            .is_empty(),
        "接口删除后用例应级联删除"
    );
}
