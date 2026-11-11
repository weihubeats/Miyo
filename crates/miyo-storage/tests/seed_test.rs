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

    // 项目 11 个（4 个完整演示 + 6 个轻量补位 + 1 个边界场景），全部以「小奏技术」品牌命名
    let projects = repo::list_projects(&db).await.unwrap();
    assert_eq!(projects.len(), 11);
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
    assert_eq!(
        count_in(platform.id),
        4,
        "平台组 4 个项目（含网关 Mock 链路演示）"
    );
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

    // 开放演示：分文件夹的 HTTP 接口 + gRPC 反射演示；GraphQL 网关：2 个接口
    let open_demo = projects
        .iter()
        .find(|p| p.name == "小奏技术 · 开放演示")
        .expect("开放演示项目存在");
    let open_endpoints = repo::list_endpoints(&db, open_demo.id).await.unwrap();
    // 22 个 HTTP（JSONPlaceholder 六类资源 + 跨域回显 + 链路演示端点）+ 2 个 gRPC（grpcb.in 反射演示）
    assert_eq!(open_endpoints.len(), 24);
    assert_eq!(
        open_endpoints
            .iter()
            .filter(|e| e.method == miyo_core::model::HttpMethod::Grpc)
            .count(),
        2,
        "gRPC 演示端点（unary + 服务端流）应存在"
    );
    // 开放演示按业务域分 5 个文件夹（文章 / 评论 / 用户 / 相册 / 杂项）
    let open_folders = repo::list_folders(&db, open_demo.id).await.unwrap();
    assert_eq!(open_folders.len(), 5, "开放演示应有 5 个业务文件夹");
    let folder_names: Vec<&str> = open_folders.iter().map(|f| f.name.as_str()).collect();
    for expected in ["文章", "评论", "用户", "相册", "杂项"] {
        assert!(
            folder_names.contains(&expected),
            "开放演示缺少文件夹「{expected}」"
        );
    }
    // 每个业务文件夹都有接口（避免空文件夹）；无文件夹归属的只有链路演示 + gRPC
    for f in &open_folders {
        let count = open_endpoints
            .iter()
            .filter(|e| e.folder_id == Some(f.id))
            .count();
        assert!(count >= 2, "文件夹「{}」至少应有 2 个接口", f.name);
    }
    let loose = open_endpoints
        .iter()
        .filter(|e| e.folder_id.is_none())
        .count();
    assert_eq!(loose, 3, "无文件夹归属：链路演示端点 1 + gRPC 2");
    // 集成场景：链路演示（3 步引用式）+ 内联请求演示（2 步内联，免接口集合）
    let scenarios = repo::list_scenarios(&db, open_demo.id).await.unwrap();
    assert_eq!(scenarios.len(), 2, "开放演示应含 2 条链路演示场景");
    let inline = scenarios
        .iter()
        .find(|s| s.name.starts_with("内联请求链路"))
        .expect("内联请求演示场景存在");
    assert_eq!(inline.steps.len(), 2);
    assert!(
        inline.steps.iter().all(|st| st.request.is_some()),
        "内联场景每步应自持请求定义"
    );

    // 开放演示含 15 条测试用例（文章 3 + 评论 4 + 用户 4 + 相册 2 + 杂项 2）
    let mut open_case_total = 0usize;
    for e in &open_endpoints {
        open_case_total += repo::list_test_cases(&db, e.id).await.unwrap().len();
    }
    assert_eq!(open_case_total, 15, "开放演示用例数");
    // 至少覆盖四类分类（正向 / 负向 / 边界值），便于用例管理页分组演示
    let mut categories = std::collections::HashSet::new();
    for e in &open_endpoints {
        for c in repo::list_test_cases(&db, e.id).await.unwrap() {
            categories.insert(c.category);
        }
    }
    for expected in ["正向", "负向", "边界值"] {
        assert!(
            categories.contains(expected),
            "开放演示缺少分类「{expected}」"
        );
    }
    // 开放演示文档素材：文章列表 / 用户详情挂响应示例（含字段注释）
    let list_posts = open_endpoints
        .iter()
        .find(|e| e.path == "/posts" && e.method == miyo_core::model::HttpMethod::GET)
        .expect("文章列表接口存在");
    assert!(!repo::list_response_examples(&db, list_posts.id)
        .await
        .unwrap()
        .is_empty());
    for name in ["用户详情", "文章评论", "评论列表", "相册照片", "待办列表"] {
        let ep = open_endpoints
            .iter()
            .find(|e| e.name == name)
            .unwrap_or_else(|| panic!("开放演示缺少接口「{name}」"));
        assert!(
            !repo::list_response_examples(&db, ep.id)
                .await
                .unwrap()
                .is_empty(),
            "接口「{name}」应有响应示例（文档页素材）"
        );
    }
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

    // 电商下单 Mock 链路：POST 下单 → GET 预览 → POST 支付，上一步结果喂下一步
    let shop = projects
        .iter()
        .find(|p| p.name == "小奏技术 · 电商下单（Mock 链路）")
        .expect("电商下单 Mock 链路项目存在");
    let shop_envs = repo::list_environments(&db, shop.id).await.unwrap();
    assert_eq!(shop_envs.len(), 1);
    assert_eq!(
        shop_envs[0].base_url, "http://127.0.0.1:4010",
        "链路演示走本地 Mock"
    );

    let shop_eps = repo::list_endpoints(&db, shop.id).await.unwrap();
    assert_eq!(shop_eps.len(), 3);
    let create_ep = shop_eps
        .iter()
        .find(|e| e.method == miyo_core::model::HttpMethod::POST && e.path == "/shop/orders")
        .expect("POST 下单接口存在");
    assert!(
        matches!(&create_ep.request.body, miyo_core::model::BodySpec::Json { raw } if raw.contains("sku")),
        "下单接口应带 JSON 请求体（请求参数演示素材）"
    );
    let preview_ep = shop_eps
        .iter()
        .find(|e| e.path == "/shop/orders/{orderNo}/pay-preview")
        .expect("GET 支付预览接口存在");
    assert!(
        !preview_ep.request.params.is_empty() && !preview_ep.request.path_variables.is_empty(),
        "预览接口应同时带 query 参数与路径变量"
    );
    assert!(
        preview_ep
            .request
            .params
            .iter()
            .any(|p| p.value.contains("{{payToken}}")),
        "预览接口的 query 应引用第 1 步提取的 payToken"
    );
    let pay_ep = shop_eps
        .iter()
        .find(|e| e.path.ends_with("/pay"))
        .expect("POST 支付接口存在");
    assert!(
        matches!(&pay_ep.request.body, miyo_core::model::BodySpec::Json { raw } if raw.contains("{{payableAmount}}")),
        "支付接口 Body 应使用预览步骤算出的实付金额"
    );

    let shop_rules = repo::list_mock_rules(&db, shop.id).await.unwrap();
    assert_eq!(
        shop_rules.len(),
        5,
        "下单 / 预览 / 支付 / 订单状态 / 发票各一条 Mock 规则"
    );
    assert!(
        shop_rules
            .iter()
            .any(|r| r.response_body_template.contains("{{params.")),
        "Mock 响应应回显路径参数（便于核对实际入参）"
    );

    let shop_scenarios = repo::list_scenarios(&db, shop.id).await.unwrap();
    let chain = shop_scenarios
        .iter()
        .find(|s| s.name.starts_with("电商主流程"))
        .expect("电商主流程场景存在");
    assert_eq!(chain.steps.len(), 3);
    let extract0 = serde_json::to_string(&chain.steps[0].extract).unwrap();
    assert!(
        extract0.contains("orderNo") && extract0.contains("payToken"),
        "第 1 步应提取 orderNo / payToken 供后续步骤使用"
    );
    assert!(
        serde_json::to_string(&chain.steps[1].extract)
            .unwrap()
            .contains("payableAmount"),
        "第 2 步应提取 payableAmount 供支付步骤使用"
    );
    // 新增：while 轮询场景 + 发票条件场景
    assert_eq!(shop_scenarios.len(), 3, "电商项目应有 3 条场景");
    let poll = shop_scenarios
        .iter()
        .find(|s| s.name.contains("轮询"))
        .expect("订单状态轮询场景存在");
    let poll_loop = poll.steps[2].r#loop.as_ref().expect("第 3 步应有循环配置");
    assert_eq!(poll_loop["mode"], "while");
    assert!(poll.steps[2].condition.is_none(), "轮询步用循环不用条件");
    let invoice = shop_scenarios
        .iter()
        .find(|s| s.name.contains("发票"))
        .expect("发票条件场景存在");
    let invoice_cond = invoice.steps[2]
        .condition
        .as_ref()
        .expect("第 3 步应有条件");
    assert_eq!(invoice_cond["op"], "gt");
    assert!(invoice.steps[2].request.is_some(), "发票步为内联请求");
}

#[tokio::test]
async fn seed_users_auth_and_condition_scenarios() {
    let db = memory_pool().await.unwrap();
    seed_dev_data(&db).await.unwrap();
    let projects = repo::list_projects(&db).await.unwrap();
    let users = projects
        .iter()
        .find(|p| p.name == "小奏技术 · 用户服务")
        .expect("用户服务项目存在");
    let scenarios = repo::list_scenarios(&db, users.id).await.unwrap();
    assert_eq!(scenarios.len(), 2, "用户服务应有 2 条集成场景");
    // 鉴权链路：第 2 步引用「当前用户」接口（Bearer {{token}} 由第 1 步提取）。
    let auth = scenarios
        .iter()
        .find(|s| s.name.contains("鉴权"))
        .expect("鉴权链路场景存在");
    assert_eq!(auth.steps.len(), 2);
    let token_extract = serde_json::to_string(&auth.steps[0].extract).unwrap();
    assert!(token_extract.contains("token"), "第 1 步应提取 token");
    // 条件演示：第 3 步 condition role eq admin。
    let condition = scenarios
        .iter()
        .find(|s| s.name.contains("角色校验"))
        .expect("角色校验场景存在");
    let cond = condition.steps[2]
        .condition
        .as_ref()
        .expect("第 3 步应有条件");
    assert_eq!(cond["name"], "role");
    assert_eq!(cond["value"], "admin");
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
