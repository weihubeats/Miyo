//! miyo-storage 集成测试：Project / Folder / Endpoint / Environment CRUD。

use sqlx::SqlitePool;

use miyo_storage::db::memory_pool;
use miyo_storage::repository as repo;
use uuid::Uuid;

use miyo_core::model::{
    EnvironmentVariable, HttpMethod, KeyValue, MockRule, RequestExample, RequestHistory,
    RequestSpec, ResponseExample, TestCase, TestCaseStatus, WsMessageType,
};

async fn pool() -> SqlitePool {
    memory_pool().await.unwrap()
}

#[tokio::test]
async fn project_crud() {
    let db = pool().await;

    let created = repo::create_project(&db, "Demo API", "描述").await.unwrap();
    assert_eq!(created.name, "Demo API");
    assert!(!created.id.to_string().is_empty());

    let listed = repo::list_projects(&db).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, created.id);
    assert_eq!(listed[0].description, "描述");

    let fetched = repo::get_project(&db, created.id).await.unwrap();
    assert_eq!(fetched.id, created.id);
    assert!(created.variables.is_empty());

    let mut updated = fetched.clone();
    updated.name = "改名".into();
    updated
        .variables
        .insert("base_url".into(), "https://x.com".into());
    repo::update_project(&db, &updated).await.unwrap();
    let refetched = repo::get_project(&db, created.id).await.unwrap();
    assert_eq!(refetched.name, "改名");
    assert_eq!(refetched.variables["base_url"], "https://x.com");

    repo::delete_project(&db, created.id).await.unwrap();
    assert!(repo::list_projects(&db).await.unwrap().is_empty());
    assert!(repo::get_project(&db, created.id).await.is_err());
}

#[tokio::test]
async fn folder_crud() {
    let db = pool().await;
    let project = repo::create_project(&db, "P", "").await.unwrap();

    let root = repo::create_folder(&db, project.id, None, "根目录")
        .await
        .unwrap();
    assert_eq!(root.name, "根目录");
    assert!(root.parent_id.is_none());

    let child = repo::create_folder(&db, project.id, Some(root.id), "子目录")
        .await
        .unwrap();
    assert_eq!(child.parent_id, Some(root.id));

    let listed = repo::list_folders(&db, project.id).await.unwrap();
    assert_eq!(listed.len(), 2);

    let fetched = repo::get_folder(&db, child.id).await.unwrap();
    assert_eq!(fetched.name, "子目录");

    // 重命名文件夹。
    let mut renamed = fetched.clone();
    renamed.name = "改名字目录".into();
    let updated = repo::update_folder(&db, &renamed).await.unwrap();
    assert_eq!(updated.name, "改名字目录");
    let fetched = repo::get_folder(&db, child.id).await.unwrap();
    assert_eq!(fetched.name, "改名字目录");

    repo::delete_folder(&db, root.id).await.unwrap();
    // 删除父文件夹后，子文件夹（及整个子树）应一并级联删除。
    assert!(repo::get_folder(&db, child.id).await.is_err());
}

#[tokio::test]
async fn delete_folder_cascades_subtree() {
    let db = pool().await;
    let project = repo::create_project(&db, "P", "").await.unwrap();

    let root = repo::create_folder(&db, project.id, None, "根")
        .await
        .unwrap();
    let child = repo::create_folder(&db, project.id, Some(root.id), "子")
        .await
        .unwrap();
    let grand = repo::create_folder(&db, project.id, Some(child.id), "孙")
        .await
        .unwrap();

    let ep_root = repo::create_endpoint(&db, project.id, Some(root.id), "R")
        .await
        .unwrap();
    let ep_child = repo::create_endpoint(&db, project.id, Some(child.id), "C")
        .await
        .unwrap();
    let ep_grand = repo::create_endpoint(&db, project.id, Some(grand.id), "G")
        .await
        .unwrap();
    let ep_free = repo::create_endpoint(&db, project.id, None, "F")
        .await
        .unwrap();

    repo::delete_folder(&db, root.id).await.unwrap();

    // 子孙文件夹全部删除，不再有孤儿记录。
    assert!(repo::get_folder(&db, root.id).await.is_err());
    assert!(repo::get_folder(&db, child.id).await.is_err());
    assert!(repo::get_folder(&db, grand.id).await.is_err());
    // 子树下接口全部删除。
    assert!(repo::get_endpoint(&db, ep_root.id).await.is_err());
    assert!(repo::get_endpoint(&db, ep_child.id).await.is_err());
    assert!(repo::get_endpoint(&db, ep_grand.id).await.is_err());
    // 子树外接口不受影响。
    assert!(repo::get_endpoint(&db, ep_free.id).await.is_ok());
    // 删除不存在的文件夹返回 NotFound。
    assert!(repo::delete_folder(&db, uuid::Uuid::new_v4())
        .await
        .is_err());
}

#[tokio::test]
async fn endpoint_crud() {
    let db = pool().await;
    let project = repo::create_project(&db, "P", "").await.unwrap();
    let folder = repo::create_folder(&db, project.id, None, "F")
        .await
        .unwrap();

    let created = repo::create_endpoint(&db, project.id, Some(folder.id), "查询用户")
        .await
        .unwrap();
    assert_eq!(created.folder_id, Some(folder.id));
    assert_eq!(created.method.to_string(), "GET");
    assert_eq!(created.status.as_str(), "developing");

    let fetched = repo::get_endpoint(&db, created.id).await.unwrap();
    assert_eq!(fetched.name, "查询用户");
    assert_eq!(fetched.request.params.len(), 0);

    let mut updated = fetched.clone();
    updated.method = "POST".parse().unwrap();
    updated.path = "/users".into();
    updated
        .request
        .params
        .push(miyo_core::model::KeyValue::new("page", "1"));
    updated.request.body = miyo_core::model::BodySpec::Json {
        raw: "{\"a\":1}".into(),
    };
    let saved = repo::update_endpoint(&db, &updated).await.unwrap();
    assert_eq!(saved.method.to_string(), "POST");

    let refetched = repo::get_endpoint(&db, created.id).await.unwrap();
    assert_eq!(refetched.path, "/users");
    assert_eq!(refetched.request.params[0].key, "page");
    assert_eq!(refetched.request.body.mode_name(), "json");

    let dup = repo::duplicate_endpoint(&db, created.id).await.unwrap();
    assert_ne!(dup.id, created.id);
    assert_eq!(dup.name, format!("{}（副本）", created.name));
    assert_eq!(dup.path, "/users");

    let listed = repo::list_endpoints(&db, project.id).await.unwrap();
    assert_eq!(listed.len(), 2);

    repo::delete_endpoint(&db, created.id).await.unwrap();
    assert!(repo::get_endpoint(&db, created.id).await.is_err());
    assert_eq!(
        repo::list_endpoints(&db, project.id).await.unwrap().len(),
        1
    );
}

/// 接口增删改触摸父项目更新时间（仪表板最近活动排序 / 卡片更新时间）。
/// 直接 SQL 回拨项目时间到过去，保证断言不受毫秒级同值影响。
/// 回拨哨兵：远古时间（RFC3339 字典序即时间序，可直接字符串比较）。
const ANCIENT: &str = "2000-01-01T00:00:00+00:00";

async fn backdate_project(db: &SqlitePool, id: Uuid) {
    sqlx::query("UPDATE projects SET updated_at = ? WHERE id = ?")
        .bind(ANCIENT)
        .bind(id.to_string())
        .execute(db)
        .await
        .unwrap();
}

async fn project_updated_at(db: &SqlitePool, id: Uuid) -> String {
    repo::get_project(db, id)
        .await
        .unwrap()
        .updated_at
        .to_rfc3339()
}

#[tokio::test]
async fn endpoint_mutation_touches_project_updated_at() {
    let db = pool().await;
    let project = repo::create_project(&db, "P", "").await.unwrap();

    backdate_project(&db, project.id).await;
    let created = repo::create_endpoint(&db, project.id, None, "查询用户")
        .await
        .unwrap();
    assert!(
        project_updated_at(&db, project.id).await.as_str() > ANCIENT,
        "新建接口应触摸项目更新时间"
    );

    backdate_project(&db, project.id).await;
    let mut updated = created.clone();
    updated.path = "/users".into();
    repo::update_endpoint(&db, &updated).await.unwrap();
    assert!(
        project_updated_at(&db, project.id).await.as_str() > ANCIENT,
        "更新接口应触摸项目更新时间"
    );

    backdate_project(&db, project.id).await;
    repo::delete_endpoint(&db, created.id).await.unwrap();
    assert!(
        project_updated_at(&db, project.id).await.as_str() > ANCIENT,
        "删除接口应触摸项目更新时间"
    );
}

#[tokio::test]
async fn environment_crud() {
    let db = pool().await;
    let project = repo::create_project(&db, "P", "").await.unwrap();

    let env = repo::create_environment(&db, project.id, "local", "", &[])
        .await
        .unwrap();
    assert_eq!(env.name, "local");
    assert_eq!(env.project_id, project.id);

    let mut updated = env.clone();
    updated.base_url = "https://pay.example.com".into();
    updated.variables.push(EnvironmentVariable {
        key: "token".into(),
        remote_value: "abc".into(),
        local_value: String::new(),
        enabled: true,
        description: None,
    });
    repo::update_environment(&db, &updated).await.unwrap();

    let fetched = repo::get_environment(&db, env.id).await.unwrap();
    assert_eq!(fetched.base_url, "https://pay.example.com");
    assert_eq!(fetched.variables.len(), 1);
    assert_eq!(fetched.variables[0].effective_value(), "abc");

    // 环境按项目隔离：仅列出本项目环境
    let other = repo::create_project(&db, "Q", "").await.unwrap();
    repo::create_environment(&db, other.id, "other", "https://other.example.com", &[])
        .await
        .unwrap();
    let listed = repo::list_environments(&db, project.id).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, env.id);

    // M11：落库应为密文（不包含明文 token），且不含加密格式前缀（明文容错路径）
    let raw: (String,) = sqlx::query_as("SELECT variables_json FROM environments WHERE id = ?")
        .bind(env.id.to_string())
        .fetch_one(&db)
        .await
        .unwrap();
    assert!(
        !raw.0.contains("abc"),
        "变量应加密存储，明文出现在库中： {}",
        raw.0
    );
    assert!(raw.0.contains(':'), "密文应为 base64:base64 格式");
    // 基址非敏感信息，明文落库。
    let base: (String,) = sqlx::query_as("SELECT base_url FROM environments WHERE id = ?")
        .bind(env.id.to_string())
        .fetch_one(&db)
        .await
        .unwrap();
    assert!(base.0.contains("https://pay.example.com"));

    repo::delete_environment(&db, env.id).await.unwrap();
    assert!(repo::list_environments(&db, project.id)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn cascade_delete_project() {
    let db = pool().await;
    let project = repo::create_project(&db, "P", "").await.unwrap();
    let folder = repo::create_folder(&db, project.id, None, "F")
        .await
        .unwrap();
    let ep = repo::create_endpoint(&db, project.id, Some(folder.id), "E")
        .await
        .unwrap();
    let env = repo::create_environment(&db, project.id, "E", "", &[])
        .await
        .unwrap();
    assert_eq!(
        repo::list_endpoints(&db, project.id).await.unwrap().len(),
        1
    );

    let other = repo::create_project(&db, "Q", "").await.unwrap();
    repo::create_endpoint(&db, other.id, None, "X")
        .await
        .unwrap();

    repo::delete_project(&db, project.id).await.unwrap();

    assert!(repo::get_endpoint(&db, ep.id).await.is_err());
    assert!(repo::get_folder(&db, folder.id).await.is_err());
    // 环境随项目级联删除。
    assert!(repo::get_environment(&db, env.id).await.is_err());
    assert_eq!(repo::list_projects(&db).await.unwrap().len(), 1);
}

#[tokio::test]
async fn settings_roundtrip() {
    let db = pool().await;
    assert!(repo::get_setting(&db, "k").await.unwrap().is_none());
    repo::set_setting(&db, "port", "4010").await.unwrap();
    assert_eq!(
        repo::get_setting(&db, "port").await.unwrap(),
        Some("4010".into())
    );
    repo::set_setting(&db, "port", "4011").await.unwrap();
    assert_eq!(
        repo::get_setting(&db, "port").await.unwrap(),
        Some("4011".into())
    );
}

#[tokio::test]
async fn ws_message_enqueue_list_delete() {
    let db = pool().await;

    repo::enqueue_ws_message(&db, "ws://a", WsMessageType::Text, "hello")
        .await
        .unwrap();
    repo::enqueue_ws_message(&db, "ws://a", WsMessageType::Binary, "AQID")
        .await
        .unwrap();
    // 其它目标地址互不影响。
    repo::enqueue_ws_message(&db, "ws://b", WsMessageType::Ping, "p1")
        .await
        .unwrap();

    let list = repo::list_pending_ws_messages(&db, "ws://a", 100)
        .await
        .unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].message_type, WsMessageType::Text);
    assert_eq!(list[0].payload, "hello");
    assert_eq!(list[1].message_type, WsMessageType::Binary);
    assert_eq!(list[1].payload, "AQID");

    repo::delete_ws_messages(&db, &[list[0].id]).await.unwrap();
    let after = repo::list_pending_ws_messages(&db, "ws://a", 100)
        .await
        .unwrap();
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].id, list[1].id);
    assert_eq!(
        repo::list_pending_ws_messages(&db, "ws://b", 100)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn ws_message_purges_expired() {
    let db = pool().await;
    let record = repo::enqueue_ws_message(&db, "ws://a", WsMessageType::Text, "old")
        .await
        .unwrap();
    // 把记录改到 48 小时前，模拟过期消息。
    let old = (chrono::Utc::now() - chrono::Duration::hours(48)).to_rfc3339();
    sqlx::query("UPDATE ws_messages SET created_at = ? WHERE id = ?")
        .bind(old)
        .bind(record.id.to_string())
        .execute(&db)
        .await
        .unwrap();

    // 24 小时内的新消息不受影响。
    repo::enqueue_ws_message(&db, "ws://a", WsMessageType::Text, "fresh")
        .await
        .unwrap();

    let removed = repo::purge_expired_ws_messages(&db, "ws://a", chrono::Duration::hours(24))
        .await
        .unwrap();
    assert_eq!(removed, 1);
    let list = repo::list_pending_ws_messages(&db, "ws://a", 100)
        .await
        .unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].payload, "fresh");
}

#[tokio::test]
async fn save_folder_repeated_id_updates_not_conflicts() {
    let db = pool().await;
    let project = repo::create_project(&db, "P", "").await.unwrap();

    let created = repo::create_folder(&db, project.id, None, "原名字")
        .await
        .unwrap();
    let mut renamed = created.clone();
    renamed.name = "新名字".into();
    renamed.updated_at = chrono::Utc::now();
    // 重命名走 save_*（带 id 再次保存），此前因主键冲突失败，回归此问题。
    repo::save_folder(&db, &renamed).await.unwrap();

    let fetched = repo::get_folder(&db, created.id).await.unwrap();
    assert_eq!(fetched.name, "新名字");
}

#[tokio::test]
async fn save_project_repeated_id_updates_not_conflicts() {
    let db = pool().await;
    let created = repo::create_project(&db, "原名", "").await.unwrap();
    let mut renamed = created.clone();
    renamed.name = "改名".into();
    renamed.updated_at = chrono::Utc::now();
    repo::save_project(&db, &renamed).await.unwrap();

    let fetched = repo::get_project(&db, created.id).await.unwrap();
    assert_eq!(fetched.name, "改名");
}

#[tokio::test]
async fn save_environment_repeated_id_updates_not_conflicts() {
    let db = pool().await;
    let project = repo::create_project(&db, "P", "").await.unwrap();
    let created = repo::create_environment(&db, project.id, "开发", "", &[])
        .await
        .unwrap();
    let mut edited = created.clone();
    edited.name = "生产".into();
    edited.updated_at = chrono::Utc::now();
    let saved = repo::save_environment(&db, &edited).await.unwrap();
    assert_eq!(saved.name, "生产");

    let fetched = repo::get_environment(&db, created.id).await.unwrap();
    assert_eq!(fetched.name, "生产");
}

/// 用户场景回归：编辑预置环境输入 Base URL（如 http://localhost:8093）后保存——
/// save_environment 走 upsert 路径，base_url 必须持久，读回（单查 + 列表）不丢。
#[tokio::test]
async fn save_environment_upsert_persists_base_url() {
    let db = pool().await;
    let project = repo::create_project(&db, "P", "").await.unwrap();
    let created = repo::create_environment(&db, project.id, "开发环境", "", &[])
        .await
        .unwrap();

    let mut edited = created.clone();
    edited.base_url = "http://localhost:8093".into();
    repo::save_environment(&db, &edited).await.unwrap();

    let fetched = repo::get_environment(&db, created.id).await.unwrap();
    assert_eq!(fetched.base_url, "http://localhost:8093");
    let listed = repo::list_environments(&db, project.id).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].base_url, "http://localhost:8093");

    // 重复 upsert 改基址：覆盖更新而非残留旧值
    let mut again = fetched;
    again.base_url = "http://localhost:9094".into();
    repo::save_environment(&db, &again).await.unwrap();
    let refetched = repo::get_environment(&db, created.id).await.unwrap();
    assert_eq!(refetched.base_url, "http://localhost:9094");
}

#[tokio::test]
async fn save_response_example_repeated_id_updates_not_conflicts() {
    let db = pool().await;
    let project = repo::create_project(&db, "P", "").await.unwrap();
    let ep = repo::create_endpoint(&db, project.id, None, "E")
        .await
        .unwrap();

    let now = chrono::Utc::now();
    let ex = ResponseExample {
        id: Uuid::new_v4(),
        endpoint_id: ep.id,
        name: "200 响应".into(),
        status: 200,
        headers: std::collections::HashMap::new(),
        body: "{}".into(),
        content_type: "application/json".into(),
        docs: std::collections::HashMap::new(),
        created_at: now,
        updated_at: now,
    };
    repo::save_response_example(&db, &ex).await.unwrap();

    // 同 id 二次保存 = 覆盖更新（设计页「保存修改」路径），不得主键冲突。
    let mut edited = ex.clone();
    edited.body = "{\"code\":0}".into();
    edited.updated_at = chrono::Utc::now();
    repo::save_response_example(&db, &edited).await.unwrap();

    let list = repo::list_response_examples(&db, ep.id).await.unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].body, "{\"code\":0}");
}

/// 备份恢复的批量多值 INSERT：六张表的 SQL（含 ON CONFLICT upsert）在真库上
/// 可执行，且与逐条 save_* 同语义——同 id 重复批量写入是覆盖更新而非冲突。
#[tokio::test]
async fn bulk_save_matches_per_row_save_semantics() {
    let db = pool().await;
    let project = repo::create_project(&db, "P", "").await.unwrap();
    let folder = repo::create_folder(&db, project.id, None, "F")
        .await
        .unwrap();
    let ep = repo::create_endpoint(&db, project.id, Some(folder.id), "E")
        .await
        .unwrap();
    let env = repo::create_environment(&db, project.id, "dev", "", &[])
        .await
        .unwrap();

    let now = chrono::Utc::now();
    let rule = MockRule {
        id: Uuid::new_v4(),
        project_id: project.id,
        endpoint_id: Some(ep.id),
        name: "命中用户列表".into(),
        method: HttpMethod::GET,
        path: "/users".into(),
        match_query: Vec::new(),
        match_headers: Vec::new(),
        response_status: 200,
        response_headers: Default::default(),
        response_body_template: "{}".into(),
        delay_ms: 0,
        fault_rate_pct: 0,
        fault_status: 500,
        enabled: true,
        priority: 1,
        created_at: now,
        updated_at: now,
    };
    repo::create_mock_rule(&db, project.id, &rule)
        .await
        .unwrap();

    let example = ResponseExample {
        id: Uuid::new_v4(),
        endpoint_id: ep.id,
        name: "200".into(),
        status: 200,
        headers: Default::default(),
        body: "{}".into(),
        content_type: "application/json".into(),
        docs: Default::default(),
        created_at: now,
        updated_at: now,
    };
    repo::save_response_example(&db, &example).await.unwrap();

    let snapshot = RequestExample {
        id: Uuid::new_v4(),
        endpoint_id: ep.id,
        name: "下单快照".into(),
        request: RequestSpec::default(),
        created_at: now,
        updated_at: now,
    };
    repo::create_request_example(&db, &snapshot).await.unwrap();

    // 走与备份导出相同的读路径拿模型，再整批回写。
    let folders = repo::list_folders(&db, project.id).await.unwrap();
    let endpoints = repo::list_endpoints(&db, project.id).await.unwrap();
    let environments = repo::list_environments(&db, project.id).await.unwrap();
    let rules = repo::list_mock_rules(&db, project.id).await.unwrap();
    let examples = repo::list_response_examples(&db, ep.id).await.unwrap();

    // request_examples 是普通 INSERT（与逐条 create_* 一致）：批量写入一条新 id。
    let mut fresh = snapshot.clone();
    fresh.id = Uuid::new_v4();

    // 内存库单连接：批量写入持连接期间不得再走 pool 查询（否则自锁）。
    {
        let mut conn = db.acquire().await.unwrap();
        repo::save_folders_bulk(&mut conn, &folders).await.unwrap();
        repo::save_endpoints_bulk(&mut conn, &endpoints)
            .await
            .unwrap();
        repo::save_environments_bulk(&mut conn, &environments)
            .await
            .unwrap();
        repo::save_mock_rules_bulk(&mut conn, &rules).await.unwrap();
        repo::save_response_examples_bulk(&mut conn, &examples)
            .await
            .unwrap();
        repo::save_request_examples_bulk(&mut conn, std::slice::from_ref(&fresh))
            .await
            .unwrap();
    }

    // upsert 类表：同 id 重复批量写入行数不变。
    assert_eq!(repo::list_folders(&db, project.id).await.unwrap().len(), 1);
    assert_eq!(
        repo::list_endpoints(&db, project.id).await.unwrap().len(),
        1
    );
    assert_eq!(
        repo::list_environments(&db, project.id)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        repo::list_mock_rules(&db, project.id).await.unwrap().len(),
        1
    );
    assert_eq!(
        repo::list_response_examples(&db, ep.id)
            .await
            .unwrap()
            .len(),
        1
    );
    // 普通 INSERT 类表：新增一行。
    assert_eq!(
        repo::list_request_examples(&db, ep.id).await.unwrap().len(),
        2
    );

    // 批量回写同样覆盖更新。
    let mut folders = repo::list_folders(&db, project.id).await.unwrap();
    folders[0].name = "改名".into();
    {
        let mut conn = db.acquire().await.unwrap();
        repo::save_folders_bulk(&mut conn, &folders).await.unwrap();
    }
    assert_eq!(
        repo::list_folders(&db, project.id).await.unwrap()[0].name,
        "改名"
    );
    // 环境批量写入后仍可读出（变量解密路径未被破坏）。
    let fetched_env = repo::get_environment(&db, env.id).await.unwrap();
    assert_eq!(fetched_env.name, "dev");
    assert_eq!(fetched_env.project_id, project.id);
}

/// 设置批量读取：一次 IN 查询取回全部命中键，缺失键不进结果。
#[tokio::test]
async fn get_settings_returns_only_requested_keys() {
    let db = pool().await;
    repo::set_setting(&db, "http_proxy", "\"http://127.0.0.1:7890\"")
        .await
        .unwrap();
    repo::set_setting(&db, "http_timeout_ms", "30000")
        .await
        .unwrap();
    repo::set_setting(&db, "log_retention_days", "7")
        .await
        .unwrap();

    let got = repo::get_settings(&db, &["http_proxy", "http_timeout_ms", "missing"])
        .await
        .unwrap();
    assert_eq!(got.len(), 2, "缺失键不返回");
    assert_eq!(got["http_timeout_ms"], "30000");
    assert!(got.contains_key("http_proxy"));
    assert!(!got.contains_key("log_retention_days"), "未请求的键不返回");

    let empty = repo::get_settings(&db, &[]).await.unwrap();
    assert!(empty.is_empty());
}

/// 批量测试用例查询（冒烟文档导出去 N+1）：按 endpoint 分组正确，
/// 组内顺序与单接口查询一致。
#[tokio::test]
async fn list_test_cases_by_endpoints_groups_and_orders() {
    let db = pool().await;
    let project = repo::create_project(&db, "P", "").await.unwrap();
    let ep_a = repo::create_endpoint(&db, project.id, None, "A")
        .await
        .unwrap();
    let ep_b = repo::create_endpoint(&db, project.id, None, "B")
        .await
        .unwrap();

    let mk = |ep: Uuid, name: &str, created: chrono::DateTime<chrono::Utc>| TestCase {
        id: Uuid::new_v4(),
        request_id: ep,
        name: name.into(),
        category: "正向".into(),
        folder: String::new(),
        method: HttpMethod::GET,
        url_path: "/users".into(),
        path_variables: Vec::new(),
        params: Vec::new(),
        headers: Vec::new(),
        body_type: "none".into(),
        body_content: String::new(),
        last_run_status: TestCaseStatus::Untested,
        created_at: created,
    };
    let base = chrono::Utc::now();
    let older = base - chrono::Duration::seconds(5);
    // A 组两条（老→新），B 组一条。
    repo::create_test_case(&db, &mk(ep_a.id, "a1", older))
        .await
        .unwrap();
    repo::create_test_case(&db, &mk(ep_a.id, "a2", base))
        .await
        .unwrap();
    repo::create_test_case(&db, &mk(ep_b.id, "b1", base))
        .await
        .unwrap();

    let ids = [ep_a.id, ep_b.id];
    let all = repo::list_test_cases_by_endpoints(&db, &ids).await.unwrap();
    assert_eq!(all.len(), 3);
    let a: Vec<&TestCase> = all.iter().filter(|c| c.request_id == ep_a.id).collect();
    let b: Vec<&TestCase> = all.iter().filter(|c| c.request_id == ep_b.id).collect();
    assert_eq!(a.len(), 2);
    assert_eq!(b.len(), 1);
    assert_eq!(a[0].name, "a1", "组内按 created_at 升序");
    assert_eq!(a[1].name, "a2");

    // 与单接口查询结果一致（顺序 + 内容）。
    let single = repo::list_test_cases(&db, ep_a.id).await.unwrap();
    let names: Vec<&str> = single.iter().map(|c| c.name.as_str()).collect();
    let batch_names: Vec<&str> = a.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, batch_names);

    // 空入参不查库。
    assert!(repo::list_test_cases_by_endpoints(&db, &[])
        .await
        .unwrap()
        .is_empty());
}

/// 测试用例路径变量落库与回读（回归）。
///
/// 旧 schema 没有 `path_variables` 列，用例里的 `/users/{id}` 取值无处安放，
/// 单跑只能把字面量 `{id}` 发出去。此测试覆盖 create / list / update 三条链路。
#[tokio::test]
async fn test_case_path_variables_persist_and_update() {
    let db = pool().await;
    let project = repo::create_project(&db, "P", "").await.unwrap();
    let ep = repo::create_endpoint(&db, project.id, None, "A")
        .await
        .unwrap();

    let vars = vec![KeyValue::new("id", "42")];
    let case = TestCase {
        id: Uuid::new_v4(),
        request_id: ep.id,
        name: "详情".into(),
        category: "正向".into(),
        folder: String::new(),
        method: HttpMethod::GET,
        url_path: "/users/{id}".into(),
        path_variables: vars.clone(),
        params: Vec::new(),
        headers: Vec::new(),
        body_type: "none".into(),
        body_content: String::new(),
        last_run_status: TestCaseStatus::Untested,
        created_at: chrono::Utc::now(),
    };
    repo::create_test_case(&db, &case).await.unwrap();

    // 单接口查询回读
    let listed = repo::list_test_cases(&db, ep.id).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].path_variables, vars, "创建后应能回读到路径变量");

    // 批量查询同样带上
    let batch = repo::list_test_cases_by_endpoints(&db, &[ep.id])
        .await
        .unwrap();
    assert_eq!(batch[0].path_variables, vars);

    // 「保存修改」更新路径变量
    let next = vec![KeyValue::new("id", "99")];
    repo::update_test_case_content(
        &db,
        case.id,
        HttpMethod::GET,
        "/users/{id}",
        &next,
        &[],
        &[],
        "none",
        "",
    )
    .await
    .unwrap();
    let after = repo::list_test_cases(&db, ep.id).await.unwrap();
    assert_eq!(after[0].path_variables, next, "更新后应回读到新取值");
}

fn history_row(project: Uuid, endpoint: Option<Uuid>, i: usize) -> RequestHistory {
    RequestHistory {
        id: Uuid::new_v4(),
        project_id: project,
        endpoint_id: endpoint,
        method: "GET".into(),
        url: format!("https://api.example.com/items/{i}"),
        status: Some(200),
        duration_ms: Some(12),
        request_summary_json: "{}".into(),
        response_summary_json: "{}".into(),
        created_at: chrono::Utc::now(),
    }
}

async fn history_count(db: &SqlitePool, project: Uuid, endpoint: Option<Uuid>) -> i64 {
    let row: (i64,) = match endpoint {
        Some(ep) => sqlx::query_as(
            "SELECT COUNT(*) FROM request_histories WHERE project_id = ? AND endpoint_id = ?",
        )
        .bind(project.to_string())
        .bind(ep.to_string())
        .fetch_one(db)
        .await
        .unwrap(),
        None => sqlx::query_as("SELECT COUNT(*) FROM request_histories WHERE project_id = ?")
            .bind(project.to_string())
            .fetch_one(db)
            .await
            .unwrap(),
    };
    row.0
}

/// 按接口清空历史只删该接口的记录（DELETE 按 endpoint 索引过滤）。
#[tokio::test]
async fn clear_request_histories_filters_by_endpoint() {
    let db = pool().await;
    let project = repo::create_project(&db, "P", "").await.unwrap();
    let ep_a = repo::create_endpoint(&db, project.id, None, "A")
        .await
        .unwrap();
    let ep_b = repo::create_endpoint(&db, project.id, None, "B")
        .await
        .unwrap();
    let project = project.id;
    let ep_a = ep_a.id;
    let ep_b = ep_b.id;
    repo::save_request_history(&db, &history_row(project, Some(ep_a), 1))
        .await
        .unwrap();
    repo::save_request_history(&db, &history_row(project, Some(ep_a), 2))
        .await
        .unwrap();
    repo::save_request_history(&db, &history_row(project, Some(ep_b), 3))
        .await
        .unwrap();
    repo::save_request_history(&db, &history_row(project, None, 4))
        .await
        .unwrap();

    let removed = repo::clear_request_histories(&db, project, Some(ep_a))
        .await
        .unwrap();
    assert_eq!(removed, 2, "只删 ep_a 的两条");
    assert_eq!(history_count(&db, project, Some(ep_a)).await, 0);
    assert_eq!(history_count(&db, project, Some(ep_b)).await, 1);
    assert_eq!(history_count(&db, project, None).await, 2);

    let removed = repo::clear_request_histories(&db, project, None)
        .await
        .unwrap();
    assert_eq!(removed, 2, "无 endpoint 条件清空整个项目");
    assert_eq!(history_count(&db, project, None).await, 0);
}

/// 多项目交替写入时各自的保留上限都生效：裁剪节流按项目计数
///（原全局计数在交替写入时只推进一个项目的裁剪，另一项目无上限增长）。
#[tokio::test]
async fn history_retention_applies_per_project_under_interleaved_writes() {
    let db = pool().await;
    let a = repo::create_project(&db, "A", "").await.unwrap();
    let b = repo::create_project(&db, "B", "").await.unwrap();
    let cap = repo::HISTORY_RETENTION_PER_PROJECT;
    // 超过上限 100 条：旧全局节流下 B 会原样涨到 600。
    let total = cap + 100;
    for i in 0..total as usize {
        for p in [&a, &b] {
            repo::save_request_history(&db, &history_row(p.id, None, i))
                .await
                .unwrap();
        }
    }
    let count_a = history_count(&db, a.id, None).await;
    let count_b = history_count(&db, b.id, None).await;
    // 裁剪节流：超额最多延迟一个节流周期（20 条）被清理。
    assert!(
        count_a <= cap + 20,
        "项目 A 应受保留上限约束，实际 {count_a}"
    );
    assert!(
        count_b <= cap + 20,
        "项目 B 应受保留上限约束，实际 {count_b}"
    );
}

/// 用例文件夹：meta 写入 folder → 整组重命名 → 批量移出分组，全链路往返。
#[tokio::test]
async fn test_case_folder_roundtrip() {
    let db = pool().await;
    let project = repo::create_project(&db, "链路", "").await.unwrap();
    let endpoint = repo::create_endpoint(&db, project.id, None, "汇款")
        .await
        .unwrap();
    let case = TestCase {
        id: Uuid::new_v4(),
        request_id: endpoint.id,
        name: "正向-汇 USD".into(),
        category: "正向".into(),
        folder: "Dlocal 渠道".into(),
        method: HttpMethod::POST,
        url_path: "/remmit".into(),
        path_variables: Vec::new(),
        params: Vec::new(),
        headers: Vec::new(),
        body_type: "json".into(),
        body_content: String::new(),
        last_run_status: TestCaseStatus::Untested,
        created_at: chrono::Utc::now(),
    };
    repo::create_test_case(&db, &case).await.unwrap();

    // meta 更新 folder 单项生效
    repo::update_test_case_meta(&db, case.id, "正向-汇 USD", "正向", "Stripe 渠道")
        .await
        .unwrap();
    let reloaded = repo::list_test_cases(&db, endpoint.id).await.unwrap();
    assert_eq!(
        reloaded.iter().find(|c| c.id == case.id).unwrap().folder,
        "Stripe 渠道"
    );

    // 整组重命名（目标组不存在 → 只改本组）
    let moved = repo::rename_test_case_folder(&db, endpoint.id, "Stripe 渠道", "Dlocal 渠道")
        .await
        .unwrap();
    assert_eq!(moved, 1);

    // 批量移出分组（空 = 未分组）
    let moved = repo::set_test_cases_folder(&db, &[case.id], "")
        .await
        .unwrap();
    assert_eq!(moved, 1);
    let reloaded = repo::list_test_cases(&db, endpoint.id).await.unwrap();
    assert_eq!(
        reloaded.iter().find(|c| c.id == case.id).unwrap().folder,
        ""
    );
}

// ---------- 场景运行历史（scenario_runs） ----------

fn scenario_run(scenario_id: Uuid, id: Uuid, offset_min: i64) -> miyo_core::model::ScenarioRun {
    use miyo_core::model::ScenarioRun;
    ScenarioRun {
        id,
        scenario_id,
        environment_id: None,
        finished_at: chrono::Utc::now() + chrono::Duration::minutes(offset_min),
        duration_ms: Some(120.5),
        total_steps: 3,
        ok_count: 2,
        fail_count: 1,
        skipped_count: 0,
        cancelled: false,
        stopped_on_failure: true,
        results_json: r#"[{"endpoint_id":"0b3f4a2e-6d1a-4c6e-9a6f-111111111111","endpoint_name":"登录","method":"POST","path":"/login","ok":false,"status":500,"duration_ms":60.0,"request_error":null,"outcomes":[]}]"#.into(),
        variables_json: r#"{"token":"abc"}"#.into(),
    }
}

#[tokio::test]
async fn scenario_run_history_roundtrip_and_prune() {
    let db = pool().await;

    let project = repo::create_project(&db, "场景历史", "").await.unwrap();
    let scenario = miyo_core::model::Scenario {
        id: Uuid::new_v4(),
        project_id: project.id,
        name: "登录链路".into(),
        description: String::new(),
        sort_order: 0,
        steps: vec![],
        script: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    repo::save_scenario(&db, &scenario).await.unwrap();

    // 保存 25 条 → 列表按时间倒序，修剪后仅剩最近 20 条。
    for i in 0..25 {
        let run = scenario_run(scenario.id, Uuid::new_v4(), i);
        repo::save_scenario_run(&db, &run).await.unwrap();
    }
    repo::prune_scenario_runs(&db, scenario.id, repo::SCENARIO_RUN_KEEP)
        .await
        .unwrap();

    let listed = repo::list_scenario_runs(&db, scenario.id, 50)
        .await
        .unwrap();
    assert_eq!(listed.len(), repo::SCENARIO_RUN_KEEP as usize);
    // 新→旧：第一条是 offset 最大的（最新）。
    assert_eq!(listed[0].ok_count, 2);
    assert!(listed[0].finished_at >= listed[1].finished_at);
    assert!(listed[0].stopped_on_failure);

    // 按 id 取回：JSON 透传字段一致。
    let got = repo::get_scenario_run(&db, listed[0].id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got.id, listed[0].id);
    assert_eq!(got.total_steps, 3);
    assert!(got.results_json.contains("登录"));
    assert_eq!(got.variables_json, r#"{"token":"abc"}"#);

    // 场景删除级联清理历史。
    repo::delete_scenario(&db, scenario.id).await.unwrap();
    let after = repo::list_scenario_runs(&db, scenario.id, 50)
        .await
        .unwrap();
    assert!(after.is_empty());
}
