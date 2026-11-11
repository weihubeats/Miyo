//! 开发环境种子数据（仅 debug 构建参与编译）。
//!
//! `npm run tauri dev` 每次启动：`db::reset_dev_database()` 先删库文件，
//! 迁移重建后由本模块写入一批开箱可测的项目 / 接口 / 环境 / Mock 规则，
//! 保证每次重启都是干净一致的测试数据集。release 构建不编译本模块，
//! 正式数据完全不受影响。
//!
//! 数据集以「小奏技术」为演示品牌，项目概览：
//! - 项目 1「小奏技术 · 用户服务」：账号 / 鉴权 REST 接口 + 配套 Mock 规则
//!   （启动 Mock 后基址 http://127.0.0.1:4010 可直接调试）；
//! - 项目 2「小奏技术 · 开放演示」：公网真实 API（JSONPlaceholder），无需 Mock 即可直接发送；
//! - 项目 3「小奏技术 · GraphQL 网关」：公共 GraphQL 服务，测试 GraphQL 工作台；
//! - 项目 2 内含 gRPC 端点：公共反射演示服务（grpcb.in），测试 gRPC 调试；
//! - 6 个轻量演示项目同样铺齐内容：带请求参数（query / 路径变量 / JSON Body）
//!   的接口 + 响应示例（文档素材）+ 测试用例（正向 / 负向 / 边界值）+
//!   请求用例 + 开发 / 测试双环境，每个项目打开都有完整内容可看；
//! - 「小奏技术 · 开放演示」含集成测试场景：文章详情 → 提取 userId →
//!   用户文章 / 用户详情（真实公网 API，激活场景页签即可直接运行链路）；
//! - 「小奏技术 · 边界场景」：大数据量 / 极端输入的 UI 压测素材——大 JSON Body
//!   （约 200 行）、超大 Body（>200KB 触发前端大文档降级）、长 urlencoded 表单、
//!   大响应示例、长请求头 / Query 列表；
//! - 环境：每个项目各自的「开发环境 / 测试环境」（单一 Base URL + 变量）、
//!   全局参数、激活项 settings。

use std::collections::HashMap;

use chrono::Utc;
use sqlx::SqlitePool;
use uuid::Uuid;

use miyo_core::model::{
    BodySpec, Endpoint, EndpointStatus, Environment, EnvironmentVariable, Folder, GlobalParam,
    GlobalParamLocation, GraphQLSpec, GrpcSpec, HttpMethod, KeyValue, MockMatchItem, MockRule,
    Project, RequestExample, RequestSpec, Scenario, ScenarioStep, TestCase, TestCaseStatus,
};
use miyo_core::Result;

use crate::repository as repo;

/// 与 miyo-tauri/src/state.rs 的 KEY_ACTIVE_PROJECT 一致；
/// state.rs 未导出常量，此处保持字面量同步。
/// 激活环境按项目记忆：键为 `active_environment_id:{project_id}`。
const KEY_ACTIVE_PROJECT: &str = "active_project_id";
const KEY_ACTIVE_ENVIRONMENT_PREFIX: &str = "active_environment_id:";

/// 开发启动时清库后的种子写入入口。
pub async fn seed_dev_data(db: &SqlitePool) -> Result<()> {
    // ---- 团队（默认团队由迁移 0015 创建，这里补两个演示团队） ----
    let team_platform = repo::create_team(db, "小奏技术 · 平台组").await?;
    let team_business = repo::create_team(db, "小奏技术 · 业务组").await?;

    // ---- 项目 ----
    let users = project(
        "小奏技术 · 用户服务",
        "小奏技术内部用户服务演示：账号与鉴权接口。启动 Mock 服务后，\
         开发环境下基址 http://127.0.0.1:4010 的接口可直接调试。",
        Some(team_platform.id),
    );
    let open_demo = project(
        "小奏技术 · 开放演示",
        "小奏技术开放 API 演示：后端指向公网 JSONPlaceholder（jsonplaceholder.typicode.com），\
         无需 Mock 即可直接发送体验。",
        Some(team_business.id),
    );
    let graphql = project(
        "小奏技术 · GraphQL 网关",
        "小奏技术 GraphQL 网关演示：接入公共 GraphQL 服务（countries.trevorblades.com），用于测试 GraphQL 工作台。",
        None, // 默认团队
    );
    for p in [&users, &open_demo, &graphql] {
        repo::save_project(db, p).await?;
    }

    // ---- 轻量演示项目（每团队补几个，凑足分组展示的数据量） ----
    // 每个项目铺齐内容：带请求参数（query / 路径变量 / JSON Body）的接口 →
    // 响应示例（文档页素材）→ 开发 / 测试双环境；测试用例与请求用例在
    // 下方统一挂载（light_endpoints 提供端点引用）。
    let mut light_endpoints: HashMap<String, HashMap<String, Endpoint>> = HashMap::new();
    for (name, desc, team_id, base_url, endpoints) in [
        (
            "小奏技术 · 网关服务",
            "API 网关配置演示：路由与限流接口（指向 JSONPlaceholder 占位）。",
            Some(team_platform.id),
            "https://jsonplaceholder.typicode.com",
            vec![
                (
                    "GET",
                    "/routes",
                    "路由列表",
                    "Query 参数 page / size 分页；返回网关路由配置数组",
                    EndpointStatus::Released,
                    RequestSpec {
                        params: vec![
                            kvd("page", "1", "页码，从 1 开始"),
                            kvd("size", "20", "每页条数，上限 100"),
                        ],
                        ..RequestSpec::default()
                    },
                    (
                        "200 成功",
                        200,
                        r#"[{"id": 1, "path": "/api/users", "upstream": "user-service", "enabled": true, "rate_limit": 100}, {"id": 2, "path": "/api/orders", "upstream": "order-service", "enabled": true, "rate_limit": 50}]"#,
                    ),
                ),
                (
                    "POST",
                    "/routes",
                    "新增路由",
                    "JSON Body：新增路由，path 需以 / 开头",
                    EndpointStatus::Testing,
                    json_body(
                        r#"{"path": "/api/notify", "upstream": "notify-service", "enabled": true, "rate_limit": 30}"#,
                    ),
                    (
                        "201 已创建",
                        201,
                        r#"{"id": 3, "path": "/api/notify", "upstream": "notify-service", "enabled": true, "rate_limit": 30}"#,
                    ),
                ),
            ],
        ),
        (
            "小奏技术 · 监控告警",
            "监控告警演示：告警规则 CRUD（指向 httpbin.org 公网调试服务）。",
            Some(team_platform.id),
            "https://httpbin.org",
            vec![
                (
                    "GET",
                    "/alerts",
                    "告警规则列表",
                    "Query 参数 status / level 过滤告警",
                    EndpointStatus::Released,
                    RequestSpec {
                        params: vec![
                            kvd("status", "all", "状态筛选：all / firing / resolved"),
                            kvd("level", "warn", "级别：info / warn / critical"),
                        ],
                        ..RequestSpec::default()
                    },
                    (
                        "200 成功",
                        200,
                        r#"[{"id": 1, "name": "CPU 使用率过高", "level": "critical", "status": "firing", "threshold": 90}, {"id": 2, "name": "磁盘水位告警", "level": "warn", "status": "resolved", "threshold": 85}]"#,
                    ),
                ),
                (
                    "PUT",
                    "/alerts/{id}",
                    "更新告警",
                    "路径变量 {id} + JSON Body：调整告警阈值",
                    EndpointStatus::Developing,
                    RequestSpec {
                        path_variables: vec![kvd("id", "1", "告警规则 ID")],
                        body: BodySpec::Json {
                            raw: r#"{"name": "CPU 使用率过高", "threshold": 95, "enabled": true}"#
                                .into(),
                        },
                        ..RequestSpec::default()
                    },
                    (
                        "200 成功",
                        200,
                        r#"{"id": 1, "name": "CPU 使用率过高", "level": "critical", "status": "firing", "threshold": 95, "enabled": true}"#,
                    ),
                ),
            ],
        ),
        (
            "小奏技术 · 支付中心",
            "支付中心演示：下单 / 退款接口（指向 jsonplaceholder 占位）。",
            Some(team_business.id),
            "https://jsonplaceholder.typicode.com",
            vec![
                (
                    "POST",
                    "/orders",
                    "创建支付订单",
                    "JSON Body：下单金额单位为元，currency 支持 CNY / USD",
                    EndpointStatus::Testing,
                    json_body(
                        r#"{"order_no": "XZ20261014001", "amount": 199.0, "currency": "CNY", "item": "Miyo 专业版"}"#,
                    ),
                    (
                        "201 已创建",
                        201,
                        r#"{"id": 10086, "order_no": "XZ20261014001", "amount": 199.0, "currency": "CNY", "status": "paid", "created_at": "2026-10-14T10:00:00Z"}"#,
                    ),
                ),
                (
                    "POST",
                    "/refunds",
                    "发起退款",
                    "JSON Body：按订单号全额或部分退款，reason 必填",
                    EndpointStatus::Developing,
                    json_body(
                        r#"{"order_no": "XZ20261014001", "amount": 199.0, "reason": "重复下单"}"#,
                    ),
                    (
                        "200 成功",
                        200,
                        r#"{"refund_no": "RF20261014001", "order_no": "XZ20261014001", "amount": 199.0, "status": "processing"}"#,
                    ),
                ),
            ],
        ),
        (
            "小奏技术 · 营销活动",
            "营销活动演示：活动投放接口（指向 httpbin.org 公网调试服务）。",
            Some(team_business.id),
            "https://httpbin.org",
            vec![
                (
                    "GET",
                    "/campaigns",
                    "活动列表",
                    "Query 参数 status / channel 过滤投放活动",
                    EndpointStatus::Released,
                    RequestSpec {
                        params: vec![
                            kvd("status", "active", "投放状态：active / paused / ended"),
                            kvd("channel", "wechat", "投放渠道：wechat / weibo / douyin"),
                        ],
                        ..RequestSpec::default()
                    },
                    (
                        "200 成功",
                        200,
                        r#"[{"id": 1, "name": "国庆大促", "status": "active", "channel": "wechat", "budget": 50000}, {"id": 2, "name": "会员日", "status": "paused", "channel": "weibo", "budget": 20000}]"#,
                    ),
                ),
                (
                    "POST",
                    "/campaigns",
                    "创建活动",
                    "JSON Body：创建投放活动，budget 单位为元",
                    EndpointStatus::Testing,
                    json_body(
                        r#"{"name": "双十一大促", "budget": 100000, "start_at": "2026-11-01T00:00:00Z", "channel": "wechat"}"#,
                    ),
                    (
                        "201 已创建",
                        201,
                        r#"{"id": 11, "name": "双十一大促", "status": "draft", "channel": "wechat", "budget": 100000}"#,
                    ),
                ),
            ],
        ),
        (
            "小奏技术 · 数据大屏",
            "数据大屏演示：指标查询接口（指向 JSONPlaceholder 占位）。",
            None, // 默认团队
            "https://jsonplaceholder.typicode.com",
            vec![
                (
                    "GET",
                    "/metrics",
                    "大屏指标聚合",
                    "Query 参数 range / unit：时间范围与展示单位",
                    EndpointStatus::Released,
                    RequestSpec {
                        params: vec![
                            kvd("range", "24h", "时间范围：1h / 24h / 7d"),
                            kvd("unit", "percent", "展示单位：percent / count"),
                        ],
                        ..RequestSpec::default()
                    },
                    (
                        "200 成功",
                        200,
                        r#"{"pv": 128734, "uv": 38210, "qps": 452, "error_rate": 0.003}"#,
                    ),
                ),
                (
                    "GET",
                    "/metrics/trend",
                    "趋势数据",
                    "Query 参数 days：回看天数（1-30）",
                    EndpointStatus::Released,
                    RequestSpec {
                        params: vec![kvd("days", "7", "回看天数，1-30")],
                        ..RequestSpec::default()
                    },
                    (
                        "200 成功",
                        200,
                        r#"{"days": ["10-08", "10-09", "10-10"], "pv": [98210, 102344, 128734], "uv": [30102, 31875, 38210]}"#,
                    ),
                ),
            ],
        ),
        (
            "小奏技术 · 文档中心",
            "文档中心演示：文档发布接口（指向 httpbin.org 公网调试服务）。",
            None, // 默认团队
            "https://httpbin.org",
            vec![
                (
                    "GET",
                    "/docs",
                    "文档列表",
                    "Query 参数 keyword / status 检索文档",
                    EndpointStatus::Released,
                    RequestSpec {
                        params: vec![
                            kvd("keyword", "", "标题 / 标签搜索关键词，可为空"),
                            kvd("status", "published", "状态：draft / published"),
                        ],
                        ..RequestSpec::default()
                    },
                    (
                        "200 成功",
                        200,
                        r#"[{"id": 1, "title": "Miyo 接入指南", "status": "published", "updated_at": "2026-10-10T09:30:00Z"}]"#,
                    ),
                ),
                (
                    "POST",
                    "/docs",
                    "发布文档",
                    "JSON Body：Markdown 正文，status 可为 draft / published",
                    EndpointStatus::Testing,
                    json_body(
                        r##"{"title": "Miyo 发版说明", "content": "# 新版本\n\n- 浅色主题对比度优化", "status": "published"}"##,
                    ),
                    (
                        "201 已创建",
                        201,
                        r#"{"id": 2, "title": "Miyo 发版说明", "status": "published", "author": "小奏技术"}"#,
                    ),
                ),
            ],
        ),
    ] {
        let p = project(name, desc, team_id);
        repo::save_project(db, &p).await?;
        let mut by_name: HashMap<String, Endpoint> = HashMap::new();
        for (
            i,
            (method, path, ep_name, ep_desc, status, request, (resp_name, resp_status, resp_body)),
        ) in endpoints.into_iter().enumerate()
        {
            let method = match method {
                "POST" => HttpMethod::POST,
                "PUT" => HttpMethod::PUT,
                _ => HttpMethod::GET,
            };
            let ep = endpoint(
                &p, None, ep_name, method, path, ep_desc, status, i as i64, request,
            );
            repo::save_endpoint(db, &ep).await?;
            repo::save_response_example(
                db,
                &response_example(&ep, resp_name, resp_status, resp_body),
            )
            .await?;
            by_name.insert(ep_name.to_string(), ep);
        }
        // 双环境：base_url 指向该项目端点所在的真实服务
        for (env_name, tag) in [("开发环境", "dev"), ("测试环境", "stg")] {
            repo::save_environment(
                db,
                &environment(
                    &p,
                    env_name,
                    base_url,
                    vec![
                        env_var(
                            "token",
                            &format!("xz-{tag}-token-123"),
                            "登录接口返回的 Bearer Token 示例",
                        ),
                        env_var(
                            "env_name",
                            if tag == "dev" {
                                "development"
                            } else {
                                "staging"
                            },
                            "当前环境标识",
                        ),
                        env_var(
                            "trace_id",
                            &format!("xz-trace-{tag}-001"),
                            "链路追踪 ID 示例",
                        ),
                    ],
                ),
            )
            .await?;
        }
        light_endpoints.insert(name.to_string(), by_name);
    }

    // ---- 边界场景项目：大数据量 / 极端输入的 UI 压测素材 ----
    // 每个端点对应一类压力场景：Body 编辑器滚动 / 行号对齐 / 拖拽缩放、
    // 大文档降级提示、长表单行编辑、大响应渲染、长请求头 / 参数列表。
    let stress = project(
        "小奏技术 · 边界场景",
        "边界场景演示：大数据量请求 / 响应的 UI 压测素材（Body 编辑器滚动与缩放、超大文档降级、\
         长表单 / 长请求头列表、大响应渲染）。基址指向 httpbin.org，路径为演示路径，\
         重点在编辑器与列表的极端输入表现，无需真实可达。",
        None,
    );
    repo::save_project(db, &stress).await?;
    let mut stress_endpoints: HashMap<String, Endpoint> = HashMap::new();
    for (i, (name, method, path, desc, request, (resp_name, resp_status, resp_body))) in [
        (
            "批量导入用户（大 Body）",
            HttpMethod::POST,
            "/batch/users",
            "JSON Body 约 200 行 / 15 个嵌套对象：验证 Body 编辑器滚动、行号对齐、拖拽缩放",
            RequestSpec {
                headers: vec![kvd("Content-Type", "application/json", "JSON 提交")],
                body: BodySpec::Json {
                    raw: big_batch_users_body(),
                },
                ..RequestSpec::default()
            },
            (
                "200 批量受理",
                200u16,
                r#"{"accepted": 15, "rejected": 0, "batch_no": "XZ-BATCH-20261015"}"#.to_string(),
            ),
        ),
        (
            "重载报表（超大 Body）",
            HttpMethod::POST,
            "/report/heavy",
            "JSON Body >200KB：触发前端大文档降级提示（isLargeDoc 只读态）",
            RequestSpec {
                headers: vec![kvd("Content-Type", "application/json", "JSON 提交")],
                body: BodySpec::Json {
                    raw: heavy_report_body(),
                },
                ..RequestSpec::default()
            },
            (
                "200 已受理",
                200,
                r#"{"task_id": "task-20261015-001", "status": "queued"}"#.to_string(),
            ),
        ),
        (
            "批量打标签（长表单）",
            HttpMethod::PUT,
            "/users/{id}/labels",
            "x-www-form-urlencoded 20 个字段：验证字段行编辑区滚动与增删行",
            RequestSpec {
                path_variables: vec![kvd("id", "1", "用户 ID")],
                body: BodySpec::UrlEncoded {
                    fields: (1..=20)
                        .map(|i| {
                            kvd(
                                &format!("label_{i:02}"),
                                &format!("标签{i:02}"),
                                &format!("表单字段 {i}"),
                            )
                        })
                        .collect(),
                },
                ..RequestSpec::default()
            },
            ("200 已更新", 200, r#"{"id": 1, "labels": 20}"#.to_string()),
        ),
        (
            "导出用户（大响应）",
            HttpMethod::GET,
            "/users/export",
            "响应示例约 1800 行大 JSON：验证响应区滚动 / 树视图展开",
            RequestSpec {
                params: vec![
                    kvd("format", "json", "导出格式"),
                    kvd("limit", "300", "导出行数"),
                ],
                ..RequestSpec::default()
            },
            ("200 大响应示例", 200, heavy_users_export_body()),
        ),
        (
            "长请求头回显",
            HttpMethod::GET,
            "/echo/headers",
            "30 个请求头 + 12 个 Query 参数：验证头部 / 参数表格滚动与全量勾选",
            RequestSpec {
                params: (1..=12)
                    .map(|i| {
                        kvd(
                            &format!("trace_{i:02}"),
                            &format!("t-{i:03}"),
                            &format!("链路字段 {i}"),
                        )
                    })
                    .collect(),
                headers: (1..=30)
                    .map(|i| {
                        kvd(
                            &format!("X-Demo-{i:02}"),
                            &format!("value-{i:03}"),
                            &format!("演示请求头 {i}"),
                        )
                    })
                    .collect(),
                ..RequestSpec::default()
            },
            (
                "200 回显",
                200,
                r#"{"headers": 30, "params": 12}"#.to_string(),
            ),
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let ep = endpoint(
            &stress,
            None,
            name,
            method,
            path,
            desc,
            EndpointStatus::Released,
            i as i64,
            request,
        );
        repo::save_endpoint(db, &ep).await?;
        repo::save_response_example(
            db,
            &response_example(&ep, resp_name, resp_status, &resp_body),
        )
        .await?;
        stress_endpoints.insert(name.to_string(), ep);
    }

    // ---- 边界场景：测试用例 / 请求用例 / 双环境 ----
    for (ep_name, case_name, category, body_content) in [
        (
            "批量导入用户（大 Body）",
            "正向-单条导入（紧凑 Body）",
            "正向",
            r#"{"batch_no": "XZ-BATCH-SINGLE", "users": [{"username": "user001", "name": "演示用户 01"}]}"#,
        ),
        (
            "重载报表（超大 Body）",
            "负向-空指标列表",
            "负向",
            r#"{"title": "", "metrics": []}"#,
        ),
    ] {
        let ep = stress_endpoints
            .get(ep_name)
            .unwrap_or_else(|| panic!("seed 边界场景端点 {ep_name}"));
        repo::create_test_case(
            db,
            &test_case(
                ep,
                case_name,
                category,
                "",
                ep.method,
                &ep.path,
                vec![],
                vec![],
                "json",
                body_content,
            ),
        )
        .await?;
    }
    let batch_users = stress_endpoints
        .get("批量导入用户（大 Body）")
        .expect("seed 边界场景批量导入端点");
    repo::create_request_example(
        db,
        &request_example_snap(
            batch_users,
            "紧凑批次（2 条）",
            json_body(
                r#"{"batch_no": "XZ-BATCH-002", "users": [{"username": "user001", "name": "演示用户 01"}, {"username": "user002", "name": "演示用户 02"}]}"#,
            ),
        ),
    )
    .await?;
    for (env_name, tag) in [("开发环境", "dev"), ("测试环境", "stg")] {
        repo::save_environment(
            db,
            &environment(
                &stress,
                env_name,
                "https://httpbin.org",
                vec![
                    env_var(
                        "token",
                        &format!("xz-{tag}-token-123"),
                        "登录接口返回的 Bearer Token 示例",
                    ),
                    env_var(
                        "env_name",
                        if tag == "dev" {
                            "development"
                        } else {
                            "staging"
                        },
                        "当前环境标识",
                    ),
                ],
            ),
        )
        .await?;
    }

    // ---- 用户服务：文件夹 + 接口 + 响应示例 + Mock 规则 ----
    let account_folder = folder(&users, None, "账号管理");
    let auth_folder = folder(&users, None, "鉴权");
    for f in [&account_folder, &auth_folder] {
        repo::save_folder(db, f).await?;
    }

    let list_users = endpoint(
        &users,
        Some(&account_folder),
        "用户列表",
        HttpMethod::GET,
        "/users",
        "分页查询小奏技术账号，Query 参数 page / limit",
        EndpointStatus::Released,
        0,
        RequestSpec {
            params: vec![
                kvd("page", "1", "页码，从 1 开始"),
                kvd("limit", "10", "每页条数，1-100"),
            ],
            ..RequestSpec::default()
        },
    );
    let get_user = endpoint(
        &users,
        Some(&account_folder),
        "用户详情",
        HttpMethod::GET,
        "/users/{id}",
        "路径变量 {id}；Mock 规则内置 /users/1 示例",
        EndpointStatus::Released,
        1,
        RequestSpec {
            path_variables: vec![kvd("id", "1", "目标用户 ID")],
            ..RequestSpec::default()
        },
    );
    let create_user = endpoint(
        &users,
        Some(&account_folder),
        "创建用户",
        HttpMethod::POST,
        "/users",
        "JSON Body：新增小奏技术账号",
        EndpointStatus::Testing,
        2,
        json_body(r#"{"name": "奏小新", "email": "xinxin@xiaozou.tech", "dept": "设计部"}"#),
    );
    let update_user = endpoint(
        &users,
        Some(&account_folder),
        "更新用户",
        HttpMethod::PUT,
        "/users/{id}",
        "JSON Body + 路径变量",
        EndpointStatus::Developing,
        3,
        json_body(r#"{"name": "奏小雪", "dept": "架构组"}"#),
    );
    let delete_user = endpoint(
        &users,
        Some(&account_folder),
        "删除用户",
        HttpMethod::DELETE,
        "/users/{id}",
        "路径变量 {id}",
        EndpointStatus::Developing,
        4,
        RequestSpec {
            path_variables: vec![kvd("id", "1", "目标用户 ID")],
            ..RequestSpec::default()
        },
    );
    let login = endpoint(
        &users,
        Some(&auth_folder),
        "登录",
        HttpMethod::POST,
        "/auth/login",
        "表单（urlencoded）提交 username / password，返回小奏技术 token",
        EndpointStatus::Testing,
        5,
        RequestSpec {
            body: BodySpec::UrlEncoded {
                fields: vec![
                    kvd("username", "demo", "登录账号"),
                    kvd("password", "xiaozou123", "登录密码"),
                ],
            },
            ..RequestSpec::default()
        },
    );
    let me = endpoint(
        &users,
        Some(&auth_folder),
        "当前用户",
        HttpMethod::GET,
        "/auth/me",
        "Bearer {{token}} 认证示例（token 来自环境变量）",
        EndpointStatus::Developing,
        6,
        RequestSpec {
            auth: miyo_core::model::AuthSpec::Bearer {
                token: "{{token}}".to_string(),
            },
            ..RequestSpec::default()
        },
    );
    for e in [
        &list_users,
        &get_user,
        &create_user,
        &update_user,
        &delete_user,
        &login,
        &me,
    ] {
        repo::save_endpoint(db, e).await?;
    }

    // 响应示例（文档页展示 + Mock 快速填充的素材）
    for example in [
        response_example_docs(
            &list_users,
            "200 成功",
            200,
            r#"[{"id": 1, "name": "奏小雪", "email": "xuexue@xiaozou.tech", "dept": "研发部"}, {"id": 2, "name": "奏小风", "email": "xiaofeng@xiaozou.tech", "dept": "产品部"}]"#,
            &[
                ("/0/id", "用户 ID"),
                ("/0/name", "姓名"),
                ("/0/email", "企业邮箱"),
                ("/0/dept", "所属部门"),
            ],
        ),
        response_example(
            &get_user,
            "200 成功",
            200,
            r#"{"id": 1, "name": "奏小雪", "email": "xuexue@xiaozou.tech", "dept": "研发部", "company": "小奏技术"}"#,
        ),
        response_example(
            &create_user,
            "201 已创建",
            201,
            r#"{"id": 1001, "name": "奏小新", "email": "xinxin@xiaozou.tech", "dept": "设计部", "company": "小奏技术"}"#,
        ),
        response_example(&delete_user, "204 无内容", 204, ""),
        response_example(
            &login,
            "200 成功",
            200,
            r#"{"token": "xz-mock-token-666", "expires_in": 3600, "company": "小奏技术"}"#,
        ),
    ] {
        repo::save_response_example(db, &example).await?;
    }

    // Mock 规则（启动 Mock 服务后，http://127.0.0.1:4010 直接命中）
    for rule in [
        mock_rule(
            &users,
            Some(&list_users),
            "用户列表",
            HttpMethod::GET,
            "/users",
            200,
            r#"[{"id": 1, "name": "奏小雪", "email": "xuexue@xiaozou.tech", "dept": "研发部"}, {"id": 2, "name": "奏小风", "email": "xiaofeng@xiaozou.tech", "dept": "产品部"}]"#,
            100,
        ),
        mock_rule(
            &users,
            Some(&get_user),
            "用户详情",
            HttpMethod::GET,
            "/users/1",
            200,
            r#"{"id": 1, "name": "奏小雪", "email": "xuexue@xiaozou.tech", "dept": "研发部", "company": "小奏技术"}"#,
            0,
        ),
        mock_rule(
            &users,
            Some(&create_user),
            "创建用户",
            HttpMethod::POST,
            "/users",
            201,
            r#"{"id": 1001, "name": "奏小新", "email": "xinxin@xiaozou.tech", "dept": "设计部", "company": "小奏技术"}"#,
            0,
        ),
        mock_rule(
            &users,
            Some(&login),
            "登录",
            HttpMethod::POST,
            "/auth/login",
            200,
            r#"{"token": "xz-mock-token-666", "expires_in": 3600, "company": "小奏技术"}"#,
            200,
        ),
        mock_rule(
            &users,
            Some(&delete_user),
            "删除用户",
            HttpMethod::DELETE,
            "/users/1",
            204,
            "",
            0,
        ),
    ] {
        repo::save_mock_rule(db, &rule).await?;
    }

    // 高级 Mock 规则：query 精确匹配 / 故障注入 / 当前用户（演示匹配优先级与故障注入）
    for rule in [
        mock_rule_ex(
            &users,
            Some(&list_users),
            "小奏技术·page=7 精确匹配",
            HttpMethod::GET,
            "/users",
            200,
            r#"{"page": 7, "list": [{"id": 7, "name": "奏小柒", "email": "qiqi@xiaozou.tech", "dept": "测试部", "company": "小奏技术"}]}"#,
            0,
            vec![MockMatchItem {
                key: "page".into(),
                value: "7".into(),
            }],
            20,
            0,
        ),
        mock_rule_ex(
            &users,
            Some(&create_user),
            "小奏技术·创建用户故障注入（30% 5xx）",
            HttpMethod::POST,
            "/users",
            201,
            r#"{"id": 1001, "name": "奏小新", "email": "xinxin@xiaozou.tech", "dept": "设计部", "company": "小奏技术"}"#,
            0,
            vec![],
            20,
            30,
        ),
        mock_rule_ex(
            &users,
            Some(&me),
            "小奏技术·当前用户",
            HttpMethod::GET,
            "/auth/me",
            200,
            r#"{"id": 1, "name": "奏小雪", "company": "小奏技术", "role": "admin", "issuedBy": "Miyo Mock"}"#,
            0,
            vec![],
            0,
            0,
        ),
    ] {
        repo::save_mock_rule(db, &rule).await?;
    }

    // ---- 开放演示：真实公网接口（JSONPlaceholder） ----
    // 按业务域分文件夹（编辑器树按文件夹分组、可折叠）：文章 / 评论 / 用户 /
    // 相册 / 杂项。JSONPlaceholder 的 6 类资源全覆盖（posts / comments /
    // users / albums / photos / todos），既能演示纯查询，也能演示 POST/PUT/PATCH/DELETE、
    // 嵌套子资源（/posts/1/comments）、路径变量（/users/{id}）、分页与过滤。
    let post_folder = folder(&open_demo, None, "文章");
    let comment_folder = folder(&open_demo, None, "评论");
    let user_folder = folder(&open_demo, None, "用户");
    let album_folder = folder(&open_demo, None, "相册");
    let misc_folder = folder(&open_demo, None, "杂项");
    for f in [
        &post_folder,
        &comment_folder,
        &user_folder,
        &album_folder,
        &misc_folder,
    ] {
        repo::save_folder(db, f).await?;
    }

    // 收集已保存端点（名称 → 端点），供测试用例 / 请求用例 / 场景挂载引用
    let mut open_endpoints: HashMap<String, Endpoint> = HashMap::new();
    for (i, (folder, name, method, path, desc, status, request)) in [
        // ---- 文章：CRUD + 嵌套子资源 ----
        (
            Some(&post_folder),
            "文章列表",
            HttpMethod::GET,
            "/posts",
            "Query 参数 _limit / _page 分页，userId 按作者过滤",
            EndpointStatus::Released,
            RequestSpec {
                params: vec![
                    kvd("_limit", "5", "返回条数"),
                    kvd("_page", "1", "页码，从 1 开始"),
                    kvd("userId", "1", "只返回该作者的文章（可留空）"),
                ],
                ..RequestSpec::default()
            },
        ),
        (
            Some(&post_folder),
            "文章详情",
            HttpMethod::GET,
            "/posts/1",
            "按 id 查询",
            EndpointStatus::Released,
            RequestSpec::default(),
        ),
        (
            Some(&post_folder),
            "文章评论",
            HttpMethod::GET,
            "/posts/1/comments",
            "嵌套子资源：文章下的全部评论",
            EndpointStatus::Released,
            RequestSpec::default(),
        ),
        (
            Some(&post_folder),
            "发布文章",
            HttpMethod::POST,
            "/posts",
            "JSON Body（服务端返回 201 模拟创建）",
            EndpointStatus::Testing,
            json_body(
                r#"{"title": "小奏技术", "body": "由小奏技术调试工具 Miyo 发送", "userId": 1}"#,
            ),
        ),
        (
            Some(&post_folder),
            "局部更新文章",
            HttpMethod::PATCH,
            "/posts/1",
            "PATCH JSON Body：只提交要改的字段（对比 PUT 整体更新）",
            EndpointStatus::Developing,
            json_body(r#"{"title": "小奏技术周报 #43（标题已修订）"}"#),
        ),
        (
            Some(&post_folder),
            "更新文章",
            HttpMethod::PUT,
            "/posts/1",
            "JSON Body 整体更新",
            EndpointStatus::Developing,
            json_body(
                r#"{"id": 1, "title": "小奏技术周报", "body": "本周小奏技术动态", "userId": 1}"#,
            ),
        ),
        (
            Some(&post_folder),
            "删除文章",
            HttpMethod::DELETE,
            "/posts/1",
            "服务端返回 200 + 空对象（模拟删除）",
            EndpointStatus::Developing,
            RequestSpec::default(),
        ),
        // ---- 评论：列表 / 详情 / 按文章过滤 / 提交 ----
        (
            Some(&comment_folder),
            "评论列表",
            HttpMethod::GET,
            "/comments",
            "Query 参数 postId 按文章过滤，_limit 限制条数",
            EndpointStatus::Released,
            RequestSpec {
                params: vec![
                    kvd("postId", "1", "只返回该文章的评论"),
                    kvd("_limit", "10", "返回条数"),
                ],
                ..RequestSpec::default()
            },
        ),
        (
            Some(&comment_folder),
            "评论详情",
            HttpMethod::GET,
            "/comments/1",
            "按 id 查询单条评论",
            EndpointStatus::Released,
            RequestSpec::default(),
        ),
        (
            Some(&comment_folder),
            "发表评论",
            HttpMethod::POST,
            "/comments",
            "JSON Body：postId / name / email / body",
            EndpointStatus::Testing,
            json_body(
                r#"{"postId": 1, "name": "奏小雪", "email": "xiaoxue@xiaozou.tech", "body": "文档写得很清楚，已联调通过。"}"#,
            ),
        ),
        // ---- 用户：列表 / 详情 / 嵌套资源 / 改资料 ----
        (
            Some(&user_folder),
            "用户列表",
            HttpMethod::GET,
            "/users",
            "全部用户（含 company / address 嵌套对象）",
            EndpointStatus::Released,
            RequestSpec::default(),
        ),
        (
            Some(&user_folder),
            "用户详情",
            HttpMethod::GET,
            "/users/1",
            "嵌套资源示例",
            EndpointStatus::Released,
            RequestSpec::default(),
        ),
        (
            Some(&user_folder),
            "用户文章",
            HttpMethod::GET,
            "/users/1/posts",
            "用户下的文章列表；改路径变量或 ctx 切换目标用户",
            EndpointStatus::Released,
            RequestSpec::default(),
        ),
        (
            Some(&user_folder),
            "用户待办",
            HttpMethod::GET,
            "/users/1/todos",
            "Query 参数 completed 过滤已完成 / 未完成",
            EndpointStatus::Released,
            RequestSpec {
                params: vec![kvd("completed", "false", "true / false，留空返回全部")],
                ..RequestSpec::default()
            },
        ),
        (
            Some(&user_folder),
            "修改用户资料",
            HttpMethod::PATCH,
            "/users/1",
            "PATCH JSON Body：只改联系方式",
            EndpointStatus::Developing,
            json_body(r#"{"email": "xiaoxue@xiaozou.tech", "phone": "138-0000-0000"}"#),
        ),
        // ---- 相册：相册与照片（大量数据，演示分页） ----
        (
            Some(&album_folder),
            "相册列表",
            HttpMethod::GET,
            "/albums",
            "Query 参数 _limit / userId 过滤",
            EndpointStatus::Released,
            RequestSpec {
                params: vec![
                    kvd("_limit", "10", "返回条数"),
                    kvd("userId", "1", "只返回该用户的相册"),
                ],
                ..RequestSpec::default()
            },
        ),
        (
            Some(&album_folder),
            "相册照片",
            HttpMethod::GET,
            "/albums/1/photos",
            "嵌套子资源：相册下的照片（5000 条数据，适合试分页）",
            EndpointStatus::Released,
            RequestSpec {
                params: vec![kvd("_limit", "20", "返回条数")],
                ..RequestSpec::default()
            },
        ),
        (
            Some(&album_folder),
            "照片详情",
            HttpMethod::GET,
            "/photos/1",
            "按 id 查询单张照片（含缩略图 / 原图地址）",
            EndpointStatus::Released,
            RequestSpec::default(),
        ),
        // ---- 杂项：待办 + 远端资源（跨域演示） ----
        (
            Some(&misc_folder),
            "待办列表",
            HttpMethod::GET,
            "/todos",
            "Query 参数 completed 过滤",
            EndpointStatus::Released,
            RequestSpec {
                params: vec![kvd("completed", "true", "true / false，留空返回全部")],
                ..RequestSpec::default()
            },
        ),
        (
            Some(&misc_folder),
            "新建待办",
            HttpMethod::POST,
            "/todos",
            "JSON Body：title / completed / userId",
            EndpointStatus::Testing,
            json_body(r#"{"title": "补充开放演示接口数据", "completed": false, "userId": 1}"#),
        ),
        (
            Some(&misc_folder),
            "远端 IP 回显",
            HttpMethod::GET,
            "https://httpbin.org/ip",
            "跨域演示：完整 URL 路径（不走环境前缀），返回请求方公网 IP",
            EndpointStatus::Released,
            RequestSpec::default(),
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let ep = endpoint(
            &open_demo, folder, name, method, path, desc, status, i as i64, request,
        );
        repo::save_endpoint(db, &ep).await?;
        open_endpoints.insert(name.to_string(), ep);
    }

    // ---- 集成场景：链路演示（真实公网 API，无需启动 Mock） ----
    // 专供场景演示的链路端点：路径直接引用 {{userId}}（由前一步提取；
    // 无对应路径变量行，applyPathVars 不改写，执行期由 runtime 渲染）。
    let chain_ep = endpoint(
        &open_demo,
        None,
        "用户文章",
        HttpMethod::GET,
        "/users/{{userId}}/posts",
        "链路演示：{{userId}} 由场景前一步从「文章详情」响应中提取",
        EndpointStatus::Released,
        open_endpoints.len() as i64,
        RequestSpec::default(),
    );
    repo::save_endpoint(db, &chain_ep).await?;
    let now = Utc::now();
    let scenario = Scenario {
        id: Uuid::new_v4(),
        project_id: open_demo.id,
        name: "文章链路：详情 → 该作者的文章 → 作者详情".into(),
        description:
            "第一步提取 userId，后续步骤的 URL 与断言期望用 {{userId}} 引用——链路变量传递演示。"
                .into(),
        category: "链路演示".into(),
        sort_order: 0,
        steps: vec![
            ScenarioStep {
                endpoint_id: Some(open_endpoints["文章详情"].id),
                enabled: true,
                extract: serde_json::json!([
                    { "name": "userId", "from": "body", "path": "$.userId" }
                ]),
                assertions: serde_json::json!([
                    { "type": "status", "op": "eq", "expected": 200 }
                ]),
                ..Default::default()
            },
            ScenarioStep {
                endpoint_id: Some(chain_ep.id),
                enabled: true,
                extract: serde_json::json!([]),
                assertions: serde_json::json!([
                    { "type": "status", "op": "eq", "expected": 200 },
                    { "type": "jsonpath", "path": "$[0].userId", "op": "eq", "expected": "{{userId}}" }
                ]),
                ..Default::default()
            },
            ScenarioStep {
                endpoint_id: Some(open_endpoints["用户详情"].id),
                enabled: true,
                extract: serde_json::json!([]),
                assertions: serde_json::json!([
                    { "type": "jsonpath", "path": "$.id", "op": "eq", "expected": "{{userId}}" }
                ]),
                ..Default::default()
            },
        ],
        script: None,
        created_at: now,
        updated_at: now,
    };
    repo::save_scenario(db, &scenario).await?;

    // ---- 电商下单：本地 Mock 通用业务链路演示（下单 → 预览 → 支付）----
    //
    // 与上一条公网链路互补：JSONPlaceholder 不收 POST、无查询类接口，演示不出
    // 「POST 请求 + 用上一步的请求/响应值拼下一步参数」。这里全部走本地 Mock，
    // 启动 Mock 服务即可离线跑通，且每一步都带真实的请求参数（query / 路径变量 /
    // JSON Body），可在场景结果行内展开「请求参数」核对实际发出的内容。
    //
    // 链路本身取最常见的电商主流程：POST 下单拿 orderNo / payToken → GET 预览
    // 支付页（query 带 orderNo、路径变量带 payToken）→ POST 发起支付（Body 带
    // 预览返回的 payableAmount）。订单号由 Mock 的 {{mock.uuid}} 生成，每跑一次
    // 都不一样，能直观看到「值确实是上一步传下来的」。
    let now = Utc::now();
    let shop_project = Project {
        id: Uuid::new_v4(),
        name: "小奏技术 · 电商下单（Mock 链路）".into(),
        description:
            "电商主流程演示：全部走本地 Mock（启动 Mock 服务后 127.0.0.1:4010 直接命中）。\
                      第 1 步 POST 下单拿 orderNo / payToken，第 2 步 GET 预览支付页，\
                      第 3 步 POST 发起支付——支付金额与订单号都来自前一步的响应。"
                .into(),
        variables: HashMap::new(),
        team_id: Some(team_platform.id),
        created_at: now,
        updated_at: now,
    };
    repo::save_project(db, &shop_project).await?;
    let shop_folder = folder(&shop_project, None, "下单支付");
    repo::save_folder(db, &shop_folder).await?;

    let create_order = endpoint(
        &shop_project,
        Some(&shop_folder),
        "创建订单",
        HttpMethod::POST,
        "/shop/orders",
        "JSON Body：商品 SKU / 数量 / 收货人；Mock 返回新订单号与支付令牌",
        EndpointStatus::Released,
        0,
        RequestSpec {
            headers: vec![kvd("Content-Type", "application/json", "请求类型")],
            body: BodySpec::Json {
                raw: r#"{"sku": "MIYO-PRO-1Y", "quantity": 2, "customer": "奏小雪", "couponCode": "NEW2026"}"#
                    .into(),
            },
            ..RequestSpec::default()
        },
    );
    repo::save_endpoint(db, &create_order).await?;
    let preview_pay = endpoint(
        &shop_project,
        Some(&shop_folder),
        "预览支付",
        HttpMethod::GET,
        "/shop/orders/{orderNo}/pay-preview",
        "路径变量 {orderNo}（第 1 步提取）+ Query payToken（第 1 步提取）",
        EndpointStatus::Released,
        1,
        RequestSpec {
            params: vec![
                kvd("payToken", "{{payToken}}", "支付令牌，来自第 1 步"),
                kvd("channel", "alipay", "支付渠道"),
            ],
            path_variables: vec![kvd("orderNo", "{{orderNo}}", "订单号，来自第 1 步")],
            ..RequestSpec::default()
        },
    );
    repo::save_endpoint(db, &preview_pay).await?;
    let pay_order = endpoint(
        &shop_project,
        Some(&shop_folder),
        "支付订单",
        HttpMethod::POST,
        "/shop/orders/{orderNo}/pay",
        "路径变量 {orderNo} + JSON Body：金额取第 2 步预览结果，渠道沿用 query",
        EndpointStatus::Released,
        2,
        RequestSpec {
            headers: vec![kvd("Content-Type", "application/json", "请求类型")],
            params: vec![kvd("channel", "alipay", "支付渠道")],
            path_variables: vec![kvd("orderNo", "{{orderNo}}", "订单号，来自第 1 步")],
            body: BodySpec::Json {
                raw: r#"{"amount": "{{payableAmount}}", "currency": "CNY", "payToken": "{{payToken}}"}"#
                    .into(),
            },
            ..RequestSpec::default()
        },
    );
    repo::save_endpoint(db, &pay_order).await?;

    // Mock 规则：响应模板用 {{params.x}} / {{query.x}} / {{mock.uuid}} 回显入参——
    // 跑完一步就能在响应里看到「实际发出去的参数」，不必去 Mock 日志里翻。
    for rule in [
        mock_rule(
            &shop_project,
            Some(&create_order),
            "电商·下单成功",
            HttpMethod::POST,
            "/shop/orders",
            201,
            r#"{"orderNo": "SO{{mock.uuid}}", "status": "CREATED", "payToken": "pt-{{mock.uuid}}", "amount": "398.00", "currency": "CNY"}"#,
            0,
        ),
        mock_rule(
            &shop_project,
            Some(&preview_pay),
            "电商·支付预览",
            HttpMethod::GET,
            "/shop/orders/{orderNo}/pay-preview",
            200,
            r#"{"orderNo": "{{params.orderNo}}", "amount": "398.00", "discount": "9.80", "payableAmount": "388.20", "channel": "{{query.channel}}", "payTokenValid": true}"#,
            0,
        ),
        mock_rule(
            &shop_project,
            Some(&pay_order),
            "电商·支付成功",
            HttpMethod::POST,
            "/shop/orders/{orderNo}/pay",
            200,
            r#"{"orderNo": "{{params.orderNo}}", "status": "PAID", "paidAt": "2026-11-04T10:30:00Z", "tradeNo": "ALI{{mock.int}}", "channel": "{{query.channel}}"}"#,
            0,
        ),
    ] {
        repo::save_mock_rule(db, &rule).await?;
    }

    let shop_scenario = Scenario {
        id: Uuid::new_v4(),
        project_id: shop_project.id,
        name: "电商主流程：下单 → 支付预览 → 支付".into(),
        description:
            "第 1 步 POST 下单，提取 orderNo / payToken；第 2 步 GET 用它们拼路径变量与 Query \
             预览支付页，提取 payableAmount；第 3 步 POST 支付，Body 带上预览算出的实付金额。"
                .into(),
        category: "电商".into(),
        sort_order: 0,
        steps: vec![
            ScenarioStep {
                endpoint_id: Some(create_order.id),
                enabled: true,
                extract: serde_json::json!([
                    { "name": "orderNo", "from": "body", "path": "$.orderNo" },
                    { "name": "payToken", "from": "body", "path": "$.payToken" }
                ]),
                assertions: serde_json::json!([
                    { "type": "status", "op": "eq", "expected": 201 },
                    { "type": "jsonpath", "path": "$.orderNo", "op": "exists", "expected": null }
                ]),
                ..Default::default()
            },
            ScenarioStep {
                endpoint_id: Some(preview_pay.id),
                enabled: true,
                extract: serde_json::json!([
                    { "name": "payableAmount", "from": "body", "path": "$.payableAmount" }
                ]),
                assertions: serde_json::json!([
                    { "type": "status", "op": "eq", "expected": 200 },
                    { "type": "jsonpath", "path": "$.payableAmount", "op": "exists", "expected": null }
                ]),
                ..Default::default()
            },
            ScenarioStep {
                endpoint_id: Some(pay_order.id),
                enabled: true,
                extract: serde_json::json!([
                    { "name": "tradeNo", "from": "body", "path": "$.tradeNo" }
                ]),
                assertions: serde_json::json!([
                    { "type": "status", "op": "eq", "expected": 200 },
                    { "type": "jsonpath", "path": "$.status", "op": "eq", "expected": "PAID" }
                ]),
                ..Default::default()
            },
        ],
        script: None,
        created_at: now,
        updated_at: now,
    };
    repo::save_scenario(db, &shop_scenario).await?;

    // ---- 集成测试补充场景（2026-11）：覆盖鉴权传递 / 条件分支 / while 轮询 / 内联请求 ----

    // ---- 用户服务（Mock 驱动，需启动 Mock 服务后跑 127.0.0.1:4010）----
    // 场景 A：鉴权链路 —— 登录提取 token，下一步经 Bearer {{token}} 认证。
    let now = Utc::now();
    let auth_scenario = Scenario {
        id: Uuid::new_v4(),
        project_id: users.id,
        name: "用户鉴权链路：登录 → 携带 Token 查当前用户".into(),
        description: "第 1 步登录提取 token；第 2 步的接口认证即 Bearer {{token}}——\
                      演示认证凭据在链路中的自动传递。启动 Mock 服务后可直接运行。"
            .into(),
        category: "鉴权".into(),
        sort_order: 0,
        steps: vec![
            ScenarioStep {
                endpoint_id: Some(login.id),
                extract: serde_json::json!([
                    { "name": "token", "from": "body", "path": "$.token" }
                ]),
                assertions: serde_json::json!([
                    { "type": "status", "op": "eq", "expected": 200 },
                    { "type": "jsonpath", "path": "$.expires_in", "op": "exists" }
                ]),
                ..Default::default()
            },
            ScenarioStep {
                endpoint_id: Some(me.id),
                assertions: serde_json::json!([
                    { "type": "status", "op": "eq", "expected": 200 },
                    { "type": "jsonpath", "path": "$.role", "op": "eq", "expected": "admin" }
                ]),
                ..Default::default()
            },
        ],
        script: None,
        created_at: now,
        updated_at: now,
    };
    repo::save_scenario(db, &auth_scenario).await?;

    // 场景 B：条件分支演示 —— 上一步提取的 role 决定删除步骤是否执行。
    let now = Utc::now();
    let condition_scenario = Scenario {
        id: Uuid::new_v4(),
        project_id: users.id,
        name: "管理员操作链路：角色校验 → 删除用户（条件执行）".into(),
        description: "第 2 步提取 role；第 3 步配置执行条件 role eq admin——\
                      满足才执行删除（不满足标记「已跳过」）。演示按响应内容做分支控制。"
            .into(),
        category: "条件演示".into(),
        sort_order: 1,
        steps: vec![
            ScenarioStep {
                endpoint_id: Some(login.id),
                extract: serde_json::json!([
                    { "name": "token", "from": "body", "path": "$.token" }
                ]),
                assertions: serde_json::json!([
                    { "type": "status", "op": "eq", "expected": 200 }
                ]),
                ..Default::default()
            },
            ScenarioStep {
                endpoint_id: Some(me.id),
                extract: serde_json::json!([
                    { "name": "role", "from": "body", "path": "$.role" }
                ]),
                assertions: serde_json::json!([
                    { "type": "jsonpath", "path": "$.role", "op": "exists" }
                ]),
                ..Default::default()
            },
            ScenarioStep {
                endpoint_id: Some(delete_user.id),
                condition: Some(serde_json::json!({
                    "source": "var", "name": "role", "op": "eq", "value": "admin"
                })),
                assertions: serde_json::json!([
                    { "type": "status", "op": "eq", "expected": 204 }
                ]),
                ..Default::default()
            },
        ],
        script: None,
        created_at: now,
        updated_at: now,
    };
    repo::save_scenario(db, &condition_scenario).await?;

    // ---- 开放演示（默认激活项目；公网 JSONPlaceholder，开箱即跑）----
    // 场景 C：内联请求演示 —— 步骤自持请求定义，不依赖接口集合。
    let now = Utc::now();
    let inline_scenario = Scenario {
        id: Uuid::new_v4(),
        project_id: open_demo.id,
        name: "内联请求链路：查询文章 → 查作者（免接口集合）".into(),
        description: "两步均为内联请求（请求定义存在场景里，不引用接口）：\
                      第 1 步查文章提取 userId；第 2 步用 {{userId}} 查作者并校验。\
                      演示「临时接口也能直接编排成链路」。"
            .into(),
        category: "内联演示".into(),
        sort_order: 1,
        steps: vec![
            ScenarioStep {
                name: Some("查询文章".into()),
                request: Some(miyo_core::model::ScenarioInlineRequest {
                    method: "GET".into(),
                    url: "https://jsonplaceholder.typicode.com/posts/1".into(),
                    spec: RequestSpec::default(),
                }),
                extract: serde_json::json!([
                    { "name": "userId", "from": "body", "path": "$.userId" },
                    { "name": "postTitle", "from": "body", "path": "$.title" }
                ]),
                assertions: serde_json::json!([
                    { "type": "status", "op": "eq", "expected": 200 },
                    { "type": "jsonpath", "path": "$.id", "op": "eq", "expected": 1 }
                ]),
                ..Default::default()
            },
            ScenarioStep {
                name: Some("查询作者".into()),
                request: Some(miyo_core::model::ScenarioInlineRequest {
                    method: "GET".into(),
                    url: "https://jsonplaceholder.typicode.com/users/{{userId}}".into(),
                    spec: RequestSpec::default(),
                }),
                assertions: serde_json::json!([
                    { "type": "status", "op": "eq", "expected": 200 },
                    { "type": "jsonpath", "path": "$.id", "op": "eq", "expected": "{{userId}}" }
                ]),
                ..Default::default()
            },
        ],
        script: None,
        created_at: now,
        updated_at: now,
    };
    repo::save_scenario(db, &inline_scenario).await?;

    // ---- 电商项目：while 轮询 + 条件查询（需 2 条新 Mock 规则支撑）----
    let order_status_rule = mock_rule(
        &shop_project,
        None,
        "电商·订单状态查询",
        HttpMethod::GET,
        "/shop/orders/{orderNo}",
        200,
        r#"{"orderNo": "{{params.orderNo}}", "status": "PAID", "payableAmount": "388.20", "currency": "CNY"}"#,
        0,
    );
    repo::save_mock_rule(db, &order_status_rule).await?;
    let invoice_rule = mock_rule(
        &shop_project,
        None,
        "电商·发票查询",
        HttpMethod::GET,
        "/shop/invoices/{orderNo}",
        200,
        r#"{"orderNo": "{{params.orderNo}}", "invoiceNo": "INV-2026-001", "amount": "388.20", "title": "小奏技术有限公司"}"#,
        0,
    );
    repo::save_mock_rule(db, &invoice_rule).await?;

    // 场景 D：while 轮询演示 —— 支付后轮询订单状态直到 PAID。
    let now = Utc::now();
    let poll_scenario = Scenario {
        id: Uuid::new_v4(),
        project_id: shop_project.id,
        name: "订单状态轮询：下单 → 支付 → 轮询直到 PAID（while 演示）".into(),
        description: "第 3 步配置 while 循环：每轮后检查 payStatus neq PAID，\
                      成立则继续轮询（间隔 300ms，最多 5 轮）。演示轮询等待类业务。"
            .into(),
        category: "循环演示".into(),
        sort_order: 1,
        steps: vec![
            ScenarioStep {
                endpoint_id: Some(create_order.id),
                extract: serde_json::json!([
                    { "name": "orderNo", "from": "body", "path": "$.orderNo" },
                    { "name": "payToken", "from": "body", "path": "$.payToken" }
                ]),
                assertions: serde_json::json!([
                    { "type": "status", "op": "eq", "expected": 201 }
                ]),
                ..Default::default()
            },
            ScenarioStep {
                endpoint_id: Some(pay_order.id),
                extract: serde_json::json!([
                    { "name": "tradeNo", "from": "body", "path": "$.tradeNo" }
                ]),
                assertions: serde_json::json!([
                    { "type": "jsonpath", "path": "$.status", "op": "eq", "expected": "PAID" }
                ]),
                ..Default::default()
            },
            ScenarioStep {
                name: Some("轮询订单状态".into()),
                request: Some(miyo_core::model::ScenarioInlineRequest {
                    method: "GET".into(),
                    url: "{{base_url}}/shop/orders/{{orderNo}}".into(),
                    spec: RequestSpec::default(),
                }),
                r#loop: Some(serde_json::json!({
                    "mode": "while",
                    "while_condition": {
                        "source": "var", "name": "payStatus", "op": "neq", "value": "PAID"
                    },
                    "max_loops": 5,
                    "interval_ms": 300
                })),
                extract: serde_json::json!([
                    { "name": "payStatus", "from": "body", "path": "$.status" }
                ]),
                assertions: serde_json::json!([
                    { "type": "status", "op": "eq", "expected": 200 },
                    { "type": "jsonpath", "path": "$.status", "op": "eq", "expected": "PAID" }
                ]),
                ..Default::default()
            },
        ],
        script: None,
        created_at: now,
        updated_at: now,
    };
    repo::save_scenario(db, &poll_scenario).await?;

    // 场景 E：条件查询演示 —— 预览金额超过阈值才查发票。
    let now = Utc::now();
    let invoice_scenario = Scenario {
        id: Uuid::new_v4(),
        project_id: shop_project.id,
        name: "发票按需查询：预览金额 > 100 才查发票（条件演示）".into(),
        description: "第 2 步提取实付金额 payableAmount；第 3 步条件 payableAmount gt 100——\
                      演示用上一步数值做业务分支（金额是数字，条件按数值比较）。"
            .into(),
        category: "条件演示".into(),
        sort_order: 2,
        steps: vec![
            ScenarioStep {
                endpoint_id: Some(create_order.id),
                extract: serde_json::json!([
                    { "name": "orderNo", "from": "body", "path": "$.orderNo" }
                ]),
                assertions: serde_json::json!([
                    { "type": "status", "op": "eq", "expected": 201 }
                ]),
                ..Default::default()
            },
            ScenarioStep {
                endpoint_id: Some(preview_pay.id),
                extract: serde_json::json!([
                    { "name": "payableAmount", "from": "body", "path": "$.payableAmount" }
                ]),
                assertions: serde_json::json!([
                    { "type": "jsonpath", "path": "$.payableAmount", "op": "exists" }
                ]),
                ..Default::default()
            },
            ScenarioStep {
                name: Some("查询发票".into()),
                request: Some(miyo_core::model::ScenarioInlineRequest {
                    method: "GET".into(),
                    url: "{{base_url}}/shop/invoices/{{orderNo}}".into(),
                    spec: RequestSpec::default(),
                }),
                condition: Some(serde_json::json!({
                    "source": "var", "name": "payableAmount", "op": "gt", "value": 100
                })),
                assertions: serde_json::json!([
                    { "type": "status", "op": "eq", "expected": 200 },
                    { "type": "jsonpath", "path": "$.invoiceNo", "op": "exists" }
                ]),
                ..Default::default()
            },
        ],
        script: None,
        created_at: now,
        updated_at: now,
    };
    repo::save_scenario(db, &invoice_scenario).await?;

    // 响应示例：文档页素材（与 Mock 规则同形状）
    for example in [
        response_example(
            &create_order,
            "201 已创建",
            201,
            r#"{"orderNo": "SO7f3c1a2b", "status": "CREATED", "payToken": "pt-9d21e0", "amount": "398.00", "currency": "CNY"}"#,
        ),
        response_example(
            &preview_pay,
            "200 成功",
            200,
            r#"{"orderNo": "SO7f3c1a2b", "amount": "398.00", "discount": "9.80", "payableAmount": "388.20", "channel": "alipay", "payTokenValid": true}"#,
        ),
        response_example(
            &pay_order,
            "200 成功",
            200,
            r#"{"orderNo": "SO7f3c1a2b", "status": "PAID", "paidAt": "2026-11-04T10:30:00Z", "tradeNo": "ALI1024", "channel": "alipay"}"#,
        ),
    ] {
        repo::save_response_example(db, &example).await?;
    }

    // ---- GraphQL：公共接口 ----
    let mut graphql_endpoints: HashMap<String, Endpoint> = HashMap::new();
    for (i, (name, desc, query, variables)) in [
        (
            "国家列表",
            "POST { countries { code name emoji capital } }",
            "query { countries { code name emoji capital } }",
            "",
        ),
        (
            "国家详情（带变量）",
            "带 $code 变量的查询，variables 里改 code 试试",
            "query Country($code: ID!) { country(code: $code) { code name emoji capital currency phone } }",
            r#"{"code": "US"}"#,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let ep = endpoint(
            &graphql,
            None,
            name,
            HttpMethod::POST,
            "/graphql",
            desc,
            EndpointStatus::Released,
            i as i64,
            RequestSpec {
                body: BodySpec::GraphQL {
                    spec: GraphQLSpec {
                        query: query.to_string(),
                        variables: variables.to_string(),
                        operation_name: String::new(),
                    },
                },
                ..RequestSpec::default()
            },
        );
        repo::save_endpoint(db, &ep).await?;
        graphql_endpoints.insert(name.to_string(), ep);
    }

    // ---- gRPC：公共反射演示服务（grpcb.in：免 proto 文件，走服务端反射） ----
    for (i, (name, desc, address, spec)) in [
        (
            "gRPC 回显调用（unary）",
            "grpcbin.GRPCBin/DummyUnary：服务端反射免 proto 文件，回显请求消息；元数据（请求头）会被服务端记录",
            "grpcb.in:9000",
            GrpcSpec {
                service: "grpcbin.GRPCBin".into(),
                method: "DummyUnary".into(),
                message: r#"{"f_string": "小奏技术"}"#.into(),
                use_tls: false,
                proto_ids: vec![],
            },
        ),
        (
            "gRPC 服务端流",
            "hello.HelloService/LotsOfReplies：一次请求，服务端流回十条问候消息",
            "grpcb.in:9000",
            GrpcSpec {
                service: "hello.HelloService".into(),
                method: "LotsOfReplies".into(),
                message: r#"{"greeting": "小奏技术"}"#.into(),
                use_tls: false,
                proto_ids: vec![],
            },
        ),
    ]
    .into_iter()
    .enumerate()
    {
        repo::save_endpoint(
            db,
            &endpoint(
                &open_demo,
                None,
                name,
                HttpMethod::Grpc,
                address,
                desc,
                EndpointStatus::Released,
                200 + i as i64,
                RequestSpec {
                    headers: vec![kv("x-demo", "miyo")],
                    body: BodySpec::Grpc { spec },
                    ..RequestSpec::default()
                },
            ),
        )
        .await?;
    }

    // ---- 开放演示 / GraphQL：响应示例（文档页素材；含字段注释演示） ----
    for example in [
        response_example_docs(
            open_endpoints.get("文章列表").expect("seed 文章列表端点"),
            "200 成功",
            200,
            r#"[{"userId": 1, "id": 1, "title": "小奏技术周报 #42", "body": "本周动态：gRPC 调试上线"}, {"userId": 2, "id": 2, "title": "小奏技术月报", "body": "本月动态：浅色主题优化"}]"#,
            &[
                ("/0/userId", "作者用户 ID"),
                ("/0/id", "文章 ID"),
                ("/0/title", "文章标题"),
                ("/0/body", "文章正文"),
            ],
        ),
        response_example(
            open_endpoints.get("发布文章").expect("seed 发布文章端点"),
            "201 已创建",
            201,
            r#"{"userId": 1, "id": 101, "title": "小奏技术周报 #42", "body": "本周动态：gRPC 调试上线"}"#,
        ),
        response_example(
            open_endpoints.get("文章评论").expect("seed 文章评论端点"),
            "200 成功",
            200,
            r#"[{"postId": 1, "id": 1, "name": "id labore ex et quam laborum", "email": "Eliseo@gardner.biz", "body": "laudantium enim quasi est quidem magnam voluptate ipsam eos"}, {"postId": 1, "id": 2, "name": "quo vero reiciendis velit similique earum", "email": "Jayne_Kuhic@sydney.com", "body": "est natus enim nihil est dolore omnis voluptatem numquam"}]"#,
        ),
        response_example_docs(
            open_endpoints.get("评论列表").expect("seed 评论列表端点"),
            "200 成功",
            200,
            r#"[{"postId": 1, "id": 1, "name": "小奏技术 · 首条评论", "email": "xiaoxue@xiaozou.tech", "body": "文档写得很清楚，已联调通过。"}]"#,
            &[
                ("/0/postId", "所属文章 ID"),
                ("/0/email", "评论者邮箱"),
                ("/0/body", "评论内容"),
            ],
        ),
        response_example_docs(
            open_endpoints.get("用户详情").expect("seed 用户详情端点"),
            "200 成功",
            200,
            r#"{"id": 1, "name": "Leanne Graham", "username": "Bret", "email": "Sincere@april.biz", "address": {"street": "Kulas Light", "suite": "Apt. 556", "city": "Gwenborough", "zipcode": "92998-3874"}, "phone": "1-770-736-8031 x56442", "website": "hildegard.org", "company": {"name": "Romaguera-Crona", "catchPhrase": "Multi-layered client-server neural-net", "bs": "harness real-time e-markets"}}"#,
            &[
                ("/username", "登录名"),
                ("/address/city", "所在城市"),
                ("/company/name", "公司名称"),
            ],
        ),
        response_example(
            open_endpoints.get("相册照片").expect("seed 相册照片端点"),
            "200 成功",
            200,
            r#"[{"albumId": 1, "id": 1, "title": "accusamus beatae ad facilis cum similique qui sunt", "url": "https://via.placeholder.com/600/92c952", "thumbnailUrl": "https://via.placeholder.com/150/92c952"}]"#,
        ),
        response_example(
            open_endpoints.get("待办列表").expect("seed 待办列表端点"),
            "200 成功",
            200,
            r#"[{"userId": 1, "id": 1, "title": "delectus aut autem", "completed": false}, {"userId": 1, "id": 2, "title": "quis ut nam facilis et officia qui", "completed": false}]"#,
        ),
        response_example(
            graphql_endpoints
                .get("国家列表")
                .expect("seed 国家列表端点"),
            "200 成功",
            200,
            r#"{"data": {"countries": [{"code": "CN", "name": "China", "emoji": "🇨🇳", "capital": "Beijing"}, {"code": "US", "name": "United States", "emoji": "🇺🇸", "capital": "Washington"}]}}"#,
        ),
        response_example_docs(
            graphql_endpoints
                .get("国家详情（带变量）")
                .expect("seed 国家详情端点"),
            "200 成功",
            200,
            r#"{"data": {"country": {"code": "US", "name": "United States", "emoji": "🇺🇸", "capital": "Washington", "currency": "USD", "phone": "1"}}}"#,
            &[
                ("/data/country/code", "ISO 3166-1 国家代码"),
                ("/data/country/currency", "货币代码"),
                ("/data/country/phone", "国际电话区号"),
            ],
        ),
    ] {
        repo::save_response_example(db, &example).await?;
    }

    // ---- 测试用例（Apifox 风格用例管理；分类为存库数据值，展示经 caseCategoryLabel 翻译） ----
    for case in [
        // 用户服务（启动 Mock 后可整组运行）
        test_case(
            &list_users,
            "正向-小奏技术默认分页",
            "正向",
            "分页",
            HttpMethod::GET,
            "/users",
            vec![kv("page", "1"), kv("limit", "10")],
            vec![],
            "none",
            "",
        ),
        test_case(
            &list_users,
            "边界值-limit 上限探测",
            "边界值",
            "分页",
            HttpMethod::GET,
            "/users",
            vec![kv("page", "1"), kv("limit", "1000")],
            vec![],
            "none",
            "",
        ),
        test_case(
            &get_user,
            "正向-查询奏小雪",
            "正向",
            "查询",
            HttpMethod::GET,
            "/users/1",
            vec![],
            vec![],
            "none",
            "",
        ),
        test_case(
            &get_user,
            "负向-不存在的用户",
            "负向",
            "查询",
            HttpMethod::GET,
            "/users/99999",
            vec![],
            vec![],
            "none",
            "",
        ),
        test_case(
            &login,
            "正向-小奏管理员登录",
            "正向",
            "鉴权",
            HttpMethod::POST,
            "/auth/login",
            vec![],
            vec![],
            "urlencoded",
            r#"{"username": "xiaozou_admin", "password": "xz@2024"}"#,
        ),
        test_case(
            &login,
            "负向-空密码登录",
            "负向",
            "鉴权",
            HttpMethod::POST,
            "/auth/login",
            vec![],
            vec![],
            "urlencoded",
            r#"{"username": "xiaozou_admin", "password": ""}"#,
        ),
        test_case(
            &create_user,
            "正向-新增小奏技术成员",
            "正向",
            "",
            HttpMethod::POST,
            "/users",
            vec![],
            vec![],
            "json",
            r#"{"name": "奏小新", "email": "xinxin@xiaozou.tech", "dept": "研发部"}"#,
        ),
        test_case(
            &me,
            "安全性-伪造 Token 访问",
            "安全性",
            "鉴权",
            HttpMethod::GET,
            "/auth/me",
            vec![],
            vec![kv("Authorization", "Bearer xz-forged-token")],
            "none",
            "",
        ),
        // 开放演示（公网 JSONPlaceholder，直接可跑）
        test_case(
            open_endpoints.get("文章列表").expect("seed 文章列表端点"),
            "正向-小奏技术文章分页",
            "正向",
            "分页",
            HttpMethod::GET,
            "/posts",
            vec![kv("_limit", "5"), kv("_page", "2")],
            vec![],
            "none",
            "",
        ),
        test_case(
            open_endpoints.get("发布文章").expect("seed 发布文章端点"),
            "正向-发布小奏技术周报",
            "正向",
            "发布流程",
            HttpMethod::POST,
            "/posts",
            vec![],
            vec![],
            "json",
            r#"{"title": "小奏技术周报 #42", "body": "本周小奏技术动态：gRPC 调试上线", "userId": 1}"#,
        ),
        // GraphQL 网关（body_type=graphql，body_content 为查询文本）
        test_case(
            graphql_endpoints
                .get("国家详情（带变量）")
                .expect("seed 国家详情端点"),
            "正向-查询小奏技术所在国家",
            "正向",
            "GraphQL",
            HttpMethod::POST,
            "/graphql",
            vec![],
            vec![],
            "graphql",
            r#"query { country(code: "CN") { name emoji capital } }"#,
        ),
        // 开放演示负向（JSONPlaceholder 对不存在资源返回 404）
        test_case(
            open_endpoints.get("文章详情").expect("seed 文章详情端点"),
            "负向-查询不存在的文章",
            "负向",
            "",
            HttpMethod::GET,
            "/posts/999999",
            vec![],
            vec![],
            "none",
            "",
        ),
        // 开放演示扩展：评论 / 用户 / 相册 / 待办（覆盖新增文件夹的分类用例）
        test_case(
            open_endpoints.get("文章评论").expect("seed 文章评论端点"),
            "正向-文章下的评论",
            "正向",
            "评论",
            HttpMethod::GET,
            "/posts/1/comments",
            vec![],
            vec![],
            "none",
            "",
        ),
        test_case(
            open_endpoints.get("评论列表").expect("seed 评论列表端点"),
            "边界值-空文章无评论",
            "边界值",
            "评论",
            HttpMethod::GET,
            "/comments",
            vec![kv("postId", "999999")],
            vec![],
            "none",
            "",
        ),
        test_case(
            open_endpoints.get("评论详情").expect("seed 评论详情端点"),
            "负向-不存在的评论",
            "负向",
            "评论",
            HttpMethod::GET,
            "/comments/999999",
            vec![],
            vec![],
            "none",
            "",
        ),
        test_case(
            open_endpoints.get("发表评论").expect("seed 发表评论端点"),
            "正向-发表小奏技术评论",
            "正向",
            "评论",
            HttpMethod::POST,
            "/comments",
            vec![],
            vec![],
            "json",
            r#"{"postId": 1, "name": "奏小雪", "email": "xiaoxue@xiaozou.tech", "body": "文档写得很清楚，已联调通过。"}"#,
        ),
        test_case(
            open_endpoints.get("用户列表").expect("seed 用户列表端点"),
            "正向-全部用户",
            "正向",
            "用户",
            HttpMethod::GET,
            "/users",
            vec![],
            vec![],
            "none",
            "",
        ),
        test_case(
            open_endpoints.get("用户文章").expect("seed 用户文章端点"),
            "正向-作者的十篇文章",
            "正向",
            "用户",
            HttpMethod::GET,
            "/users/1/posts",
            vec![],
            vec![],
            "none",
            "",
        ),
        test_case(
            open_endpoints.get("用户待办").expect("seed 用户待办端点"),
            "正向-未完成待办",
            "正向",
            "用户",
            HttpMethod::GET,
            "/users/1/todos",
            vec![kv("completed", "false")],
            vec![],
            "none",
            "",
        ),
        test_case(
            open_endpoints
                .get("修改用户资料")
                .expect("seed 修改用户资料端点"),
            "正向-修改联系方式",
            "正向",
            "用户",
            HttpMethod::PATCH,
            "/users/1",
            vec![],
            vec![],
            "json",
            r#"{"email": "xiaoxue@xiaozou.tech"}"#,
        ),
        test_case(
            open_endpoints.get("相册列表").expect("seed 相册列表端点"),
            "边界值-单页上限 100",
            "边界值",
            "相册",
            HttpMethod::GET,
            "/albums",
            vec![kv("_limit", "100")],
            vec![],
            "none",
            "",
        ),
        test_case(
            open_endpoints.get("相册照片").expect("seed 相册照片端点"),
            "正向-相册前 20 张照片",
            "正向",
            "相册",
            HttpMethod::GET,
            "/albums/1/photos",
            vec![kv("_limit", "20")],
            vec![],
            "none",
            "",
        ),
        test_case(
            open_endpoints.get("待办列表").expect("seed 待办列表端点"),
            "正向-已完成待办",
            "正向",
            "杂项",
            HttpMethod::GET,
            "/todos",
            vec![kv("completed", "true")],
            vec![],
            "none",
            "",
        ),
        test_case(
            open_endpoints.get("新建待办").expect("seed 新建待办端点"),
            "负向-缺少标题",
            "负向",
            "杂项",
            HttpMethod::POST,
            "/todos",
            vec![],
            vec![],
            "json",
            r#"{"completed": false, "userId": 1}"#,
        ),
        // GraphQL 负向（ZZ 非有效国家代码，country 返回 null）
        test_case(
            graphql_endpoints
                .get("国家详情（带变量）")
                .expect("seed 国家详情端点"),
            "负向-无效国家代码",
            "负向",
            "GraphQL",
            HttpMethod::POST,
            "/graphql",
            vec![],
            vec![],
            "graphql",
            r#"query { country(code: "ZZ") { name } }"#,
        ),
    ] {
        repo::create_test_case(db, &case).await?;
    }

    // ---- 请求用例（请求快照，一键回填调试页） ----
    for example in [
        request_example_snap(&list_users, "小奏技术·第一页", RequestSpec {
            params: vec![kv("page", "1"), kv("limit", "10")],
            ..RequestSpec::default()
        }),
        request_example_snap(&list_users, "小奏技术·每页 50 条", RequestSpec {
            params: vec![kv("page", "1"), kv("limit", "50")],
            ..RequestSpec::default()
        }),
        request_example_snap(&login, "小奏管理员账号", RequestSpec {
            body: BodySpec::UrlEncoded {
                fields: vec![kv("username", "xiaozou_admin"), kv("password", "xz@2024")],
            },
            ..RequestSpec::default()
        }),
        request_example_snap(&create_user, "小奏技术新成员（研发部）", RequestSpec {
            body: BodySpec::Json {
                raw: r#"{"name": "奏小新", "email": "xinxin@xiaozou.tech", "dept": "研发部"}"#.into(),
            },
            ..RequestSpec::default()
        }),
        request_example_snap(
            open_endpoints.get("更新文章").expect("seed 更新文章端点"),
            "小奏技术周报样例",
            RequestSpec {
                body: BodySpec::Json {
                    raw: r#"{"id": 1, "title": "小奏技术周报", "body": "本周小奏技术动态", "userId": 1}"#.into(),
                },
                ..RequestSpec::default()
            },
        ),
        request_example_snap(
            open_endpoints.get("文章列表").expect("seed 文章列表端点"),
            "小奏技术·只取作者 1 的前 3 条",
            RequestSpec {
                params: vec![kv("userId", "1"), kv("_limit", "3")],
                ..RequestSpec::default()
            },
        ),
        request_example_snap(
            open_endpoints.get("文章评论").expect("seed 文章评论端点"),
            "小奏技术·文章 1 的全部评论",
            RequestSpec {
                params: vec![kv("_limit", "50")],
                ..RequestSpec::default()
            },
        ),
        request_example_snap(
            open_endpoints.get("发表评论").expect("seed 发表评论端点"),
            "小奏技术·评论样例",
            RequestSpec {
                body: BodySpec::Json {
                    raw: r#"{"postId": 1, "name": "奏小新", "email": "xinxin@xiaozou.tech", "body": "这条评论由 Miyo 请求用例回填。"}"#.into(),
                },
                ..RequestSpec::default()
            },
        ),
        request_example_snap(
            open_endpoints.get("用户待办").expect("seed 用户待办端点"),
            "小奏技术·未完成待办",
            RequestSpec {
                params: vec![kv("completed", "false")],
                ..RequestSpec::default()
            },
        ),
        request_example_snap(
            open_endpoints.get("相册照片").expect("seed 相册照片端点"),
            "小奏技术·相册前 10 张",
            RequestSpec {
                params: vec![kv("_limit", "10")],
                ..RequestSpec::default()
            },
        ),
        request_example_snap(
            graphql_endpoints
                .get("国家详情（带变量）")
                .expect("seed 国家详情端点"),
            "小奏技术·查中国（带变量）",
            RequestSpec {
                body: BodySpec::GraphQL {
                    spec: GraphQLSpec {
                        query: "query Country($code: ID!) { country(code: $code) { code name emoji capital currency phone } }".into(),
                        variables: r#"{"code": "CN"}"#.into(),
                        operation_name: String::new(),
                    },
                },
                ..RequestSpec::default()
            },
        ),
    ] {
        repo::create_request_example(db, &example).await?;
    }

    // ---- 轻量项目测试用例（每项目 2 条：正向 + 负向 / 边界值） ----
    for (proj, ep_name, case_name, category, case_path, params, headers, body_type, body_content) in [
        (
            "小奏技术 · 网关服务",
            "路由列表",
            "正向-默认分页拉取路由",
            "正向",
            "/routes",
            vec![kv("page", "1"), kv("size", "20")],
            vec![],
            "none",
            "",
        ),
        (
            "小奏技术 · 网关服务",
            "新增路由",
            "负向-缺少 path 字段",
            "负向",
            "/routes",
            vec![],
            vec![],
            "json",
            r#"{"upstream": "notify-service"}"#,
        ),
        (
            "小奏技术 · 监控告警",
            "告警规则列表",
            "正向-只看 firing 告警",
            "正向",
            "/alerts",
            vec![kv("status", "firing"), kv("level", "critical")],
            vec![],
            "none",
            "",
        ),
        (
            "小奏技术 · 监控告警",
            "更新告警",
            "负向-更新不存在的告警",
            "负向",
            "/alerts/999999",
            vec![],
            vec![],
            "json",
            r#"{"name": "幽灵告警", "threshold": 100}"#,
        ),
        (
            "小奏技术 · 支付中心",
            "创建支付订单",
            "正向-创建 199 元订单",
            "正向",
            "/orders",
            vec![],
            vec![],
            "json",
            r#"{"order_no": "XZ20261014001", "amount": 199.0, "currency": "CNY", "item": "Miyo 专业版"}"#,
        ),
        (
            "小奏技术 · 支付中心",
            "创建支付订单",
            "负向-金额为负数",
            "负向",
            "/orders",
            vec![],
            vec![],
            "json",
            r#"{"order_no": "XZ20261014002", "amount": -1.0, "currency": "CNY", "item": "Miyo 专业版"}"#,
        ),
        (
            "小奏技术 · 营销活动",
            "活动列表",
            "正向-微信渠道投放中活动",
            "正向",
            "/campaigns",
            vec![kv("status", "active"), kv("channel", "wechat")],
            vec![],
            "none",
            "",
        ),
        (
            "小奏技术 · 营销活动",
            "创建活动",
            "边界值-预算为 0 的草稿",
            "边界值",
            "/campaigns",
            vec![],
            vec![],
            "json",
            r#"{"name": "零预算预热", "budget": 0, "start_at": "2026-11-01T00:00:00Z", "channel": "weibo"}"#,
        ),
        (
            "小奏技术 · 数据大屏",
            "大屏指标聚合",
            "正向-近 24 小时指标",
            "正向",
            "/metrics",
            vec![kv("range", "24h"), kv("unit", "percent")],
            vec![],
            "none",
            "",
        ),
        (
            "小奏技术 · 数据大屏",
            "趋势数据",
            "边界值-回看 30 天上限",
            "边界值",
            "/metrics/trend",
            vec![kv("days", "30")],
            vec![],
            "none",
            "",
        ),
        (
            "小奏技术 · 文档中心",
            "文档列表",
            "正向-检索已发布文档",
            "正向",
            "/docs",
            vec![kv("keyword", "Miyo"), kv("status", "published")],
            vec![],
            "none",
            "",
        ),
        (
            "小奏技术 · 文档中心",
            "发布文档",
            "负向-标题为空",
            "负向",
            "/docs",
            vec![],
            vec![],
            "json",
            r#"{"title": "", "content": "正文", "status": "draft"}"#,
        ),
    ] {
        let by_name = light_endpoints
            .get(proj)
            .unwrap_or_else(|| panic!("seed 轻量项目 {proj}"));
        let ep = by_name
            .get(ep_name)
            .unwrap_or_else(|| panic!("seed 轻量端点 {proj} / {ep_name}"));
        repo::create_test_case(
            db,
            &test_case(
                ep,
                case_name,
                category,
                "",
                ep.method,
                case_path,
                params,
                headers,
                body_type,
                body_content,
            ),
        )
        .await?;
    }

    // ---- 轻量项目请求用例（每项目 1 条，一键回填调试页） ----
    for (proj, ep_name, ex_name, request) in [
        (
            "小奏技术 · 网关服务",
            "路由列表",
            "第一页 10 条",
            RequestSpec {
                params: vec![kv("page", "1"), kv("size", "10")],
                ..RequestSpec::default()
            },
        ),
        (
            "小奏技术 · 监控告警",
            "告警规则列表",
            "只看 critical 告警",
            RequestSpec {
                params: vec![kv("status", "firing"), kv("level", "critical")],
                ..RequestSpec::default()
            },
        ),
        (
            "小奏技术 · 支付中心",
            "创建支付订单",
            "99 元基础版订单",
            json_body(
                r#"{"order_no": "XZ20261014009", "amount": 99.0, "currency": "CNY", "item": "Miyo 基础版"}"#,
            ),
        ),
        (
            "小奏技术 · 营销活动",
            "创建活动",
            "微博渠道双十二活动",
            json_body(
                r#"{"name": "双十二返场", "budget": 50000, "start_at": "2026-12-12T00:00:00Z", "channel": "weibo"}"#,
            ),
        ),
        (
            "小奏技术 · 数据大屏",
            "趋势数据",
            "近 7 天趋势",
            RequestSpec {
                params: vec![kv("days", "7")],
                ..RequestSpec::default()
            },
        ),
        (
            "小奏技术 · 文档中心",
            "文档列表",
            "搜索 Miyo 草稿",
            RequestSpec {
                params: vec![kv("keyword", "Miyo"), kv("status", "draft")],
                ..RequestSpec::default()
            },
        ),
    ] {
        let by_name = light_endpoints
            .get(proj)
            .unwrap_or_else(|| panic!("seed 轻量项目 {proj}"));
        let ep = by_name
            .get(ep_name)
            .unwrap_or_else(|| panic!("seed 轻量端点 {proj} / {ep_name}"));
        repo::create_request_example(db, &request_example_snap(ep, ex_name, request)).await?;
    }

    // ---- 环境（项目维度：每个项目各自的开发 / 测试环境 + 单一 Base URL） ----
    let users_dev = environment(
        &users,
        "开发环境",
        "http://127.0.0.1:4010",
        vec![
            env_var(
                "token",
                "xz-dev-token-123",
                "小奏技术登录接口返回的 Bearer Token 示例",
            ),
            env_var("env_name", "development", "当前环境标识"),
            env_var("trace_id", "xz-trace-dev-001", "链路追踪 ID 示例"),
        ],
    );
    let users_test = environment(
        &users,
        "测试环境",
        "http://127.0.0.1:4010",
        vec![
            env_var(
                "token",
                "xz-test-token-456",
                "小奏技术登录接口返回的 Bearer Token 示例",
            ),
            env_var("env_name", "staging", "当前环境标识"),
            env_var("trace_id", "xz-trace-stg-001", "链路追踪 ID 示例"),
        ],
    );
    let open_dev = environment(
        &open_demo,
        "开发环境",
        "https://jsonplaceholder.typicode.com",
        vec![
            env_var(
                "token",
                "xz-dev-token-123",
                "小奏技术登录接口返回的 Bearer Token 示例",
            ),
            env_var("env_name", "development", "当前环境标识"),
            env_var("trace_id", "xz-trace-dev-001", "链路追踪 ID 示例"),
        ],
    );
    let open_test = environment(
        &open_demo,
        "测试环境",
        "https://jsonplaceholder.typicode.com",
        vec![
            env_var(
                "token",
                "xz-test-token-456",
                "小奏技术登录接口返回的 Bearer Token 示例",
            ),
            env_var("env_name", "staging", "当前环境标识"),
            env_var("trace_id", "xz-trace-stg-001", "链路追踪 ID 示例"),
        ],
    );
    let gql_dev = environment(
        &graphql,
        "开发环境",
        "https://countries.trevorblades.com",
        vec![
            env_var(
                "token",
                "xz-dev-token-123",
                "小奏技术登录接口返回的 Bearer Token 示例",
            ),
            env_var("env_name", "development", "当前环境标识"),
            env_var("trace_id", "xz-trace-dev-001", "链路追踪 ID 示例"),
        ],
    );
    let gql_test = environment(
        &graphql,
        "测试环境",
        "https://countries.trevorblades.com",
        vec![
            env_var(
                "token",
                "xz-test-token-456",
                "小奏技术登录接口返回的 Bearer Token 示例",
            ),
            env_var("env_name", "staging", "当前环境标识"),
            env_var("trace_id", "xz-trace-stg-001", "链路追踪 ID 示例"),
        ],
    );
    // 电商下单（链路 Mock 演示）：基址指向本地 Mock 服务，启动 Mock 后整条
    // 链路（下单 → 预览 → 支付，含两次 POST）离线可跑。
    let shop_env = environment(
        &shop_project,
        "开发环境",
        "http://127.0.0.1:4010",
        vec![
            env_var("token", "xz-dev-token-123", "Mock 演示用 Bearer Token"),
            env_var(
                "channel",
                "alipay",
                "默认支付渠道（也可直接在接口 Query 里改）",
            ),
        ],
    );
    for env in [
        &users_dev,
        &users_test,
        &open_dev,
        &open_test,
        &gql_dev,
        &gql_test,
        &shop_env,
    ] {
        repo::save_environment(db, env).await?;
    }

    // ---- 全局参数（注入制演示） ----
    repo::save_global_params(
        db,
        &[
            GlobalParam {
                key: "X-Client".to_string(),
                value: "XiaoZouTech-Miyo".to_string(),
                enabled: true,
                location: GlobalParamLocation::Header,
            },
            GlobalParam {
                key: "trace_id".to_string(),
                value: "xz-seed-trace-001".to_string(),
                enabled: false,
                location: GlobalParamLocation::Query,
            },
        ],
    )
    .await?;

    // ---- 激活项：启动即落在「小奏技术 · 开放演示」（jsonplaceholder 公网接口）+ 其开发环境 ----
    // 默认激活公网项目而非本地 Mock（127.0.0.1:4010），保证 dev 开箱即可直接发送请求；
    // 需要本地 Mock 演示时切到「小奏技术 · 用户服务」并启动 Mock 服务即可。
    repo::set_setting(db, KEY_ACTIVE_PROJECT, &setting_id(&open_demo.id)).await?;
    repo::set_setting(
        db,
        &format!("{KEY_ACTIVE_ENVIRONMENT_PREFIX}{}", open_demo.id),
        &setting_id(&open_dev.id),
    )
    .await?;

    Ok(())
}

/* ---------- 边界场景：大数据量 Body / 响应生成 ---------- */

/// 大 JSON Body（约 200 行 / 15 个嵌套对象）：
/// Body 编辑器滚动、行号对齐、拖拽缩放手柄的压测素材。
fn big_batch_users_body() -> String {
    const DEPTS: [&str; 5] = ["研发部", "设计部", "市场部", "销售部", "运维部"];
    let rows: Vec<String> = (1..=15)
        .map(|i| {
            format!(
                "    {{\n      \"username\": \"user{i:03}\",\n      \"name\": \"演示用户 {i:02}\",\n      \"email\": \"user{i:03}@xiaozou.tech\",\n      \"dept\": \"{}\",\n      \"roles\": [\"admin\", \"editor\"],\n      \"profile\": {{\n        \"city\": \"上海\",\n        \"age\": {},\n        \"active\": true\n      }},\n      \"remark\": \"边界场景演示数据第 {i} 条，用于验证编辑器滚动与行号对齐\"\n    }}",
                DEPTS[i % DEPTS.len()],
                22 + (i % 30),
            )
        })
        .collect();
    format!(
        "{{\n  \"batch_no\": \"XZ-BATCH-20261015\",\n  \"operator\": \"奏小雪\",\n  \"users\": [\n{}\n  ]\n}}",
        rows.join(",\n")
    )
}

/// 超大 JSON Body（约 2000 行 / >200KB）：
/// 超过前端 JsonEditor 的 LARGE_DOC_CHARS（200_000）阈值，触发大文档降级提示。
fn heavy_report_body() -> String {
    let rows: Vec<String> = (1..=2000)
        .map(|i| {
            format!(
                "    {{ \"metric\": \"cpu.usage.node{i:04}\", \"value\": {}.{:02}, \"unit\": \"percent\", \"host\": \"node-{i:03}\", \"region\": \"cn-east-1\" }}",
                i % 100,
                i % 60,
            )
        })
        .collect();
    format!(
        "{{\n  \"title\": \"全量指标快照（超大 Body 演示）\",\n  \"generated_at\": \"2026-10-15T08:00:00Z\",\n  \"metrics\": [\n{}\n  ]\n}}",
        rows.join(",\n")
    )
}

/// 大响应示例（约 1800 行 / ~130KB）：响应区滚动 / 树视图展开的压测素材。
fn heavy_users_export_body() -> String {
    let rows: Vec<String> = (1..=300)
        .map(|i| {
            format!(
                "  {{\n    \"id\": {i},\n    \"name\": \"导出用户 {i:03}\",\n    \"email\": \"export{i:03}@xiaozou.tech\",\n    \"status\": \"{}\",\n    \"dept\": \"导出演示部\",\n    \"labels\": [\"export\", \"bulk\"]\n  }}",
                if i % 7 == 0 { "disabled" } else { "active" },
            )
        })
        .collect();
    format!("[\n{}\n]", rows.join(",\n"))
}

/* ---------- 构造辅助 ---------- */

fn project(name: &str, description: &str, team_id: Option<Uuid>) -> Project {
    let now = Utc::now();
    Project {
        id: Uuid::new_v4(),
        name: name.to_string(),
        description: description.to_string(),
        variables: HashMap::new(),
        team_id: team_id.or(Some(miyo_core::model::DEFAULT_TEAM_ID)),
        created_at: now,
        updated_at: now,
    }
}

fn folder(project: &Project, parent: Option<&Folder>, name: &str) -> Folder {
    let now = Utc::now();
    Folder {
        id: Uuid::new_v4(),
        project_id: project.id,
        parent_id: parent.map(|f| f.id),
        name: name.to_string(),
        sort_order: 0,
        created_at: now,
        updated_at: now,
    }
}

#[allow(clippy::too_many_arguments)]
fn endpoint(
    project: &Project,
    folder: Option<&Folder>,
    name: &str,
    method: HttpMethod,
    path: &str,
    description: &str,
    status: EndpointStatus,
    sort_order: i64,
    request: RequestSpec,
) -> Endpoint {
    let now = Utc::now();
    Endpoint {
        id: Uuid::new_v4(),
        project_id: project.id,
        folder_id: folder.map(|f| f.id),
        name: name.to_string(),
        method,
        path: path.to_string(),
        description: description.to_string(),
        status,
        sort_order,
        request,
        created_at: now,
        updated_at: now,
    }
}

fn response_example(
    endpoint: &Endpoint,
    name: &str,
    status: u16,
    body: &str,
) -> miyo_core::model::ResponseExample {
    let now = Utc::now();
    miyo_core::model::ResponseExample {
        id: Uuid::new_v4(),
        endpoint_id: endpoint.id,
        name: name.to_string(),
        status,
        headers: HashMap::new(),
        body: body.to_string(),
        content_type: "application/json".to_string(),
        docs: HashMap::new(),
        created_at: now,
        updated_at: now,
    }
}

/// 响应示例 + 字段注释（JSON Pointer → FieldDoc，设计页结构化编辑器 / 文档素材）。
fn response_example_docs(
    endpoint: &Endpoint,
    name: &str,
    status: u16,
    body: &str,
    docs: &[(&str, &str)],
) -> miyo_core::model::ResponseExample {
    let mut example = response_example(endpoint, name, status, body);
    example.docs = docs
        .iter()
        .map(|(pointer, desc)| {
            (
                (*pointer).to_string(),
                miyo_core::model::FieldDoc {
                    description: (*desc).to_string(),
                    required: true,
                },
            )
        })
        .collect();
    example
}

#[allow(clippy::too_many_arguments)]
fn mock_rule(
    project: &Project,
    endpoint: Option<&Endpoint>,
    name: &str,
    method: HttpMethod,
    path: &str,
    status: u16,
    body: &str,
    delay_ms: u64,
) -> MockRule {
    let now = Utc::now();
    MockRule {
        id: Uuid::new_v4(),
        project_id: project.id,
        endpoint_id: endpoint.map(|e| e.id),
        name: name.to_string(),
        method,
        path: path.to_string(),
        match_query: Vec::<MockMatchItem>::new(),
        match_headers: Vec::<MockMatchItem>::new(),
        response_status: status,
        response_headers: HashMap::new(),
        response_body_template: body.to_string(),
        delay_ms,
        fault_rate_pct: 0,
        fault_status: 500,
        enabled: true,
        priority: 0,
        created_at: now,
        updated_at: now,
    }
}

fn environment(
    project: &Project,
    name: &str,
    base_url: &str,
    variables: Vec<EnvironmentVariable>,
) -> Environment {
    let now = Utc::now();
    Environment {
        id: Uuid::new_v4(),
        project_id: project.id,
        name: name.to_string(),
        base_url: base_url.to_string(),
        variables,
        created_at: now,
        updated_at: now,
    }
}

fn env_var(key: &str, value: &str, description: &str) -> EnvironmentVariable {
    EnvironmentVariable {
        key: key.to_string(),
        remote_value: value.to_string(),
        local_value: String::new(),
        enabled: true,
        local_override: None,
        sensitive: None,
        description: Some(description.to_string()),
    }
}

fn kv(key: &str, value: &str) -> KeyValue {
    KeyValue::new(key, value)
}

/// 带说明的键值：文档页参数 / 头部 / 表单字段表展示 description，种子请求参数一律带说明。
fn kvd(key: &str, value: &str, description: &str) -> KeyValue {
    KeyValue {
        description: description.to_string(),
        ..KeyValue::new(key, value)
    }
}

fn json_body(raw: &str) -> RequestSpec {
    RequestSpec {
        body: BodySpec::Json {
            raw: raw.to_string(),
        },
        ..RequestSpec::default()
    }
}

/// 与 miyo-tauri state.rs `setting_value` 一致：JSON 字符串 `"uuid"`。
fn setting_id(id: &Uuid) -> String {
    serde_json::to_string(&id.to_string()).unwrap_or_else(|_| "null".into())
}

/// 测试用例（挂到主接口；分类为存库数据值：正向/负向/边界值/安全性）。
#[allow(clippy::too_many_arguments)]
fn test_case(
    endpoint: &Endpoint,
    name: &str,
    category: &str,
    folder: &str,
    method: HttpMethod,
    url_path: &str,
    params: Vec<KeyValue>,
    headers: Vec<KeyValue>,
    body_type: &str,
    body_content: &str,
) -> TestCase {
    TestCase {
        id: Uuid::new_v4(),
        request_id: endpoint.id,
        name: name.to_string(),
        category: category.to_string(),
        folder: folder.to_string(),
        method,
        url_path: url_path.to_string(),
        // 种子用例跟随主接口的路径变量，保证 `/alerts/{id}` 单跑时能替换。
        path_variables: endpoint.request.path_variables.clone(),
        params,
        headers,
        body_type: body_type.to_string(),
        body_content: body_content.to_string(),
        assertions: serde_json::Value::Array(Vec::new()),
        extractors: serde_json::Value::Array(Vec::new()),
        auth: None,
        timeout_ms: None,
        disable_cookies: false,
        sort_order: 0,
        last_run_status: TestCaseStatus::Untested,
        last_run_at: None,
        last_run_ok: None,
        last_run_fail: None,
        created_at: Utc::now(),
    }
}

/// 请求用例（当前请求快照，可一键回填调试页）。
fn request_example_snap(endpoint: &Endpoint, name: &str, request: RequestSpec) -> RequestExample {
    let now = Utc::now();
    RequestExample {
        id: Uuid::new_v4(),
        endpoint_id: endpoint.id,
        name: name.to_string(),
        request,
        created_at: now,
        updated_at: now,
    }
}

/// 高级 Mock 规则：支持 query 精确匹配 / 优先级 / 故障注入（基础版 mock_rule 的超集）。
#[allow(clippy::too_many_arguments)]
fn mock_rule_ex(
    project: &Project,
    endpoint: Option<&Endpoint>,
    name: &str,
    method: HttpMethod,
    path: &str,
    status: u16,
    body: &str,
    delay_ms: u64,
    match_query: Vec<MockMatchItem>,
    priority: i64,
    fault_rate_pct: u8,
) -> MockRule {
    MockRule {
        match_query,
        priority,
        fault_rate_pct,
        ..mock_rule(
            project, endpoint, name, method, path, status, body, delay_ms,
        )
    }
}
