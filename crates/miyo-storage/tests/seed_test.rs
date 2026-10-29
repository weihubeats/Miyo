//! 开发种子数据测试（仅 debug 构建运行）：内存库上完整跑一遍 seed_dev_data，
//! 断言项目 / 接口 / 环境 / Mock 规则 / 测试用例 / 请求用例 / 全局参数 / 激活项齐全。
#![cfg(debug_assertions)]

use miyo_storage::db::memory_pool;
use miyo_storage::repository as repo;
use miyo_storage::seed::seed_dev_data;

#[tokio::test]
async fn seeds_full_fixture_set() {
    let db = memory_pool().await.unwrap();
    seed_dev_data(&db).await.unwrap();

    // 项目 10 个（3 个完整演示 + 6 个轻量补位 + 1 个边界场景），全部以「小奏技术」品牌命名
    let projects = repo::list_projects(&db).await.unwrap();
    assert_eq!(projects.len(), 10);
    assert!(
        projects.iter().all(|p| p.name.contains("小奏技术")),
        "所有种子项目名都应包含品牌「小奏技术」"
    );

    // 团队：默认团队（迁移 0015）+ 平台组 + 业务组；项目按团队分组展示
    let teams = repo::list_teams(&db).await.unwrap();
    assert_eq!(teams.len(), 3);
    let platform = teams
        .iter()
        .find(|t| t.name == "小奏技术 · 平台组")
        .expect("平台组存在");
    let business = teams
        .iter()
        .find(|t| t.name == "小奏技术 · 业务组")
        .expect("业务组存在");
    let count_in = |team_id: uuid::Uuid| -> usize {
        projects
            .iter()
            .filter(|p| p.team_id == Some(team_id))
            .count()
    };
    assert_eq!(count_in(platform.id), 3, "平台组 3 个项目");
    assert_eq!(count_in(business.id), 3, "业务组 3 个项目");
    assert_eq!(
        projects
            .iter()
            .filter(|p| p.team_id == Some(miyo_core::model::DEFAULT_TEAM_ID))
            .count(),
        4,
        "默认团队 4 个项目（GraphQL 网关 + 边界场景）"
    );

    let users = projects
        .iter()
        .find(|p| p.name == "小奏技术 · 用户服务")
        .expect("用户服务项目存在");

    // 用户服务：7 个接口（5 账号 + 2 鉴权），2 个文件夹，Mock 规则 5 基础 + 3 高级
    let endpoints = repo::list_endpoints(&db, users.id).await.unwrap();
    assert_eq!(endpoints.len(), 7);
    let folders = repo::list_folders(&db, users.id).await.unwrap();
    assert_eq!(folders.len(), 2);
    let rules = repo::list_mock_rules(&db, users.id).await.unwrap();
    assert_eq!(
        rules.len(),
        8,
        "5 条基础规则 + 3 条高级规则（query 精确匹配/故障注入/当前用户）"
    );
    assert!(
        rules.iter().any(|r| r.fault_rate_pct > 0),
        "应含故障注入演示规则"
    );
    assert!(
        rules.iter().any(|r| !r.match_query.is_empty()),
        "应含 query 精确匹配演示规则"
    );
    let list_users = endpoints
        .iter()
        .find(|e| e.path == "/users" && e.method == miyo_core::model::HttpMethod::GET)
        .expect("用户列表接口存在");
    let examples = repo::list_response_examples(&db, list_users.id)
        .await
        .unwrap();
    assert_eq!(examples.len(), 1);
    assert_eq!(examples[0].status, 200);

    // 测试用例：用户服务 8 条（正向/负向/边界值/安全性），全部挂在小奏技术端点上
    let mut users_case_total = 0usize;
    for e in &endpoints {
        users_case_total += repo::list_test_cases(&db, e.id).await.unwrap().len();
    }
    assert_eq!(users_case_total, 8);
    let login_cases = repo::list_test_cases(
        &db,
        endpoints
            .iter()
            .find(|e| e.path == "/auth/login")
            .expect("登录接口存在")
            .id,
    )
    .await
    .unwrap();
    assert_eq!(login_cases.len(), 2);
    assert!(login_cases.iter().any(|c| c.category == "正向"));
    assert!(login_cases.iter().any(|c| c.category == "负向"));

    // 请求用例：用户列表 2 条（分页快照），登录 1 条
    let list_req_examples = repo::list_request_examples(&db, list_users.id)
        .await
        .unwrap();
    assert_eq!(list_req_examples.len(), 2);
    assert!(list_req_examples
        .iter()
        .all(|ex| ex.name.contains("小奏技术")));

    // 开放演示：6 个接口；GraphQL 网关：2 个接口
    let open_demo = projects
        .iter()
        .find(|p| p.name == "小奏技术 · 开放演示")
        .expect("开放演示项目存在");
    let open_endpoints = repo::list_endpoints(&db, open_demo.id).await.unwrap();
    // 7 个 HTTP（JSONPlaceholder + 链路演示端点）+ 2 个 gRPC（grpcb.in 反射演示）
    assert_eq!(open_endpoints.len(), 9);
    assert_eq!(
        open_endpoints
            .iter()
            .filter(|e| e.method == miyo_core::model::HttpMethod::Grpc)
            .count(),
        2,
        "gRPC 演示端点（unary + 服务端流）应存在"
    );
    // 集成场景：链路演示（3 步，引用真实公网接口）
    let scenarios = repo::list_scenarios(&db, open_demo.id).await.unwrap();
    assert_eq!(scenarios.len(), 1, "开放演示应含 1 条链路演示场景");
    assert_eq!(scenarios[0].steps.len(), 3);

    // 开放演示含 3 条测试用例（文章分页 / 发布周报 / 不存在文章负向）+ 1 条请求用例
    let mut open_case_total = 0usize;
    for e in &open_endpoints {
        open_case_total += repo::list_test_cases(&db, e.id).await.unwrap().len();
    }
    assert_eq!(open_case_total, 3);
    // 开放演示文档素材：文章列表挂响应示例（含字段注释）
    let list_posts = open_endpoints
        .iter()
        .find(|e| e.path == "/posts" && e.method == miyo_core::model::HttpMethod::GET)
        .expect("文章列表接口存在");
    assert!(!repo::list_response_examples(&db, list_posts.id)
        .await
        .unwrap()
        .is_empty());
    let graphql = projects
        .iter()
        .find(|p| p.name == "小奏技术 · GraphQL 网关")
        .expect("GraphQL 项目存在");
    let gql_eps = repo::list_endpoints(&db, graphql.id).await.unwrap();
    assert_eq!(gql_eps.len(), 2);
    // GraphQL：每接口响应示例（文档素材）+ 正向/负向 2 条用例 + 1 条请求用例
    let mut gql_cases = 0usize;
    let mut gql_reqs = 0usize;
    for e in &gql_eps {
        assert!(
            !repo::list_response_examples(&db, e.id)
                .await
                .unwrap()
                .is_empty(),
            "GraphQL 接口应有响应示例"
        );
        gql_cases += repo::list_test_cases(&db, e.id).await.unwrap().len();
        gql_reqs += repo::list_request_examples(&db, e.id).await.unwrap().len();
    }
    assert_eq!(gql_cases, 2, "国家详情：正向 + 负向");
    assert_eq!(gql_reqs, 1, "带变量请求用例 1 条");

    // 轻量项目：每个都铺齐 —— 2 接口（带请求参数）+ 每接口响应示例
    //（文档素材）+ ≥2 测试用例 + ≥1 请求用例 + 开发 / 测试双环境
    for name in [
        "小奏技术 · 网关服务",
        "小奏技术 · 监控告警",
        "小奏技术 · 支付中心",
        "小奏技术 · 营销活动",
        "小奏技术 · 数据大屏",
        "小奏技术 · 文档中心",
    ] {
        let p = projects
            .iter()
            .find(|proj| proj.name == name)
            .unwrap_or_else(|| panic!("{name} 存在"));
        let eps = repo::list_endpoints(&db, p.id).await.unwrap();
        assert_eq!(eps.len(), 2, "{name} 2 个接口");
        assert!(
            eps.iter().any(|e| {
                !e.request.params.is_empty()
                    || !matches!(e.request.body, miyo_core::model::BodySpec::None)
            }),
            "{name} 至少一个接口带请求参数或 Body"
        );
        let mut cases = 0usize;
        let mut reqs = 0usize;
        for e in &eps {
            assert!(
                !repo::list_response_examples(&db, e.id)
                    .await
                    .unwrap()
                    .is_empty(),
                "{name}/{} 应有响应示例（文档素材）",
                e.name
            );
            cases += repo::list_test_cases(&db, e.id).await.unwrap().len();
            reqs += repo::list_request_examples(&db, e.id).await.unwrap().len();
        }
        assert!(cases >= 2, "{name} ≥2 条测试用例");
        assert!(reqs >= 1, "{name} ≥1 条请求用例");
        let envs = repo::list_environments(&db, p.id).await.unwrap();
        assert_eq!(envs.len(), 2, "{name} 开发 / 测试双环境");
    }

    // 边界场景：5 个大数据量端点 —— Body 规模 / 大文档阈值 / 长表单 / 长请求头 / 大响应
    let stress = projects
        .iter()
        .find(|p| p.name == "小奏技术 · 边界场景")
        .expect("边界场景项目存在");
    let stress_eps = repo::list_endpoints(&db, stress.id).await.unwrap();
    assert_eq!(stress_eps.len(), 5, "边界场景 5 个接口");
    let stress_ep = |name: &str| {
        stress_eps
            .iter()
            .find(|e| e.name == name)
            .unwrap_or_else(|| panic!("边界场景端点 {name} 存在"))
    };
    // 大 JSON Body ≈200 行；超大 Body ≥200K 字符（触发前端 LARGE_DOC_CHARS 降级）
    for (name, min_bytes, min_lines) in [
        ("批量导入用户（大 Body）", 3_000usize, 150usize),
        ("重载报表（超大 Body）", 200_000, 2_000),
    ] {
        let raw = match &stress_ep(name).request.body {
            miyo_core::model::BodySpec::Json { raw } => raw.clone(),
            other => panic!("{name} 应为 JSON Body，实际 {other:?}"),
        };
        assert!(raw.len() >= min_bytes, "{name} Body 应 ≥{min_bytes} 字节");
        assert!(
            raw.matches('\n').count() >= min_lines,
            "{name} Body 应 ≥{min_lines} 行"
        );
    }
    match &stress_ep("批量打标签（长表单）").request.body {
        miyo_core::model::BodySpec::UrlEncoded { fields } => {
            assert_eq!(fields.len(), 20, "长表单 20 个字段");
        }
        other => panic!("长表单端点应为 urlencoded Body，实际 {other:?}"),
    }
    let echo = stress_ep("长请求头回显");
    assert_eq!(echo.request.headers.len(), 30, "长请求头 30 条");
    assert_eq!(echo.request.params.len(), 12, "长 Query 参数 12 条");
    // 大响应示例 ≥1000 行（响应区滚动 / 树视图素材）
    let export = stress_ep("导出用户（大响应）");
    let export_examples = repo::list_response_examples(&db, export.id).await.unwrap();
    assert!(!export_examples.is_empty(), "大响应端点应有响应示例");
    assert!(
        export_examples[0].body.matches('\n').count() >= 1_000,
        "大响应示例应 ≥1000 行"
    );
    assert_eq!(
        repo::list_environments(&db, stress.id).await.unwrap().len(),
        2,
        "边界场景 开发 / 测试双环境"
    );

    // 环境：每个项目各自的开发 / 测试环境（单一 Base URL + 变量）
    let envs = repo::list_environments(&db, users.id).await.unwrap();
    assert_eq!(envs.len(), 2);
    let dev = envs
        .iter()
        .find(|e| e.name == "开发环境")
        .expect("开发环境存在");
    assert_eq!(dev.base_url, "http://127.0.0.1:4010");
    assert_eq!(dev.variables.len(), 3);
    // 开放演示 / GraphQL 网关项目同样各有开发 + 测试环境
    let open_demo = projects
        .iter()
        .find(|p| p.name == "小奏技术 · 开放演示")
        .expect("开放演示项目存在");
    let open_envs = repo::list_environments(&db, open_demo.id).await.unwrap();
    assert_eq!(open_envs.len(), 2);
    let open_dev = open_envs
        .iter()
        .find(|e| e.name == "开发环境")
        .expect("开放演示开发环境存在");
    assert_eq!(open_dev.base_url, "https://jsonplaceholder.typicode.com");

    // 全局参数 2 个；激活项 settings 已写入（项目 + 该项目的激活环境）
    let params = repo::get_global_params(&db).await.unwrap();
    assert_eq!(params.len(), 2);
    let active_project = repo::get_setting(&db, "active_project_id")
        .await
        .unwrap()
        .expect("active_project_id 已写入");
    // dev 默认激活公网项目（jsonplaceholder）而非本地 Mock，开箱即可直接发送请求
    assert!(active_project.contains(&open_demo.id.to_string()));
    let active_env = repo::get_setting(&db, &format!("active_environment_id:{}", open_demo.id))
        .await
        .unwrap()
        .expect("激活环境已写入");
    assert!(active_env.contains(&open_dev.id.to_string()));
}

/// 幂等性：同一内存库重复 seed（模拟手动重复调用）不应报错。
#[tokio::test]
async fn seed_is_idempotent_per_fresh_db() {
    let db = memory_pool().await.unwrap();
    seed_dev_data(&db).await.unwrap();
    // 注意：设计上只对「每次启动清库后」的全新库调用一次；重复调用会因
    // upsert（save_*）而成功，但会追加第二份 Mock 规则 —— 这里仅验证不 panic。
    seed_dev_data(&db).await.unwrap();
}
