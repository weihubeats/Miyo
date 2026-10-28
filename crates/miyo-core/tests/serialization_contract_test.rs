//! 序列化契约测试：IPC 响应模型的 JSON 键必须是 snake_case。
//!
//! 背景：list_project_stats 曾因结构体加 `#[serde(rename_all = "camelCase")]`
//! 导致前端按蛇形读取全部得到 undefined，仪表板统计显示为 0。本测试对
//! miyo-core 全部对外模型做键名扫描，防止同类命名断裂再次发生：
//! 任何人给模型加 camelCase 重命名都会在此处红灯。

use serde::Serialize;
use std::collections::HashMap;
use uuid::Uuid;

use miyo_core::model::{
    ApiKeyLocation, AuthSpec, BodySpec, Endpoint, EndpointStatus, Environment, EnvironmentVariable,
    GlobalParam, GlobalParamLocation, GraphQLSpec, GrpcSpec, HttpMethod, KeyValue, MockMatchItem,
    MockRule, MultipartField, OAuth2Token, Project, ProtoFile, RequestExample, RequestHistory,
    RequestSpec, ResponseExample, TestCase, TestCaseStatus, TestRun,
};

/// 合法键：snake_case（小写字母/数字/下划线，且不含大写字母）。
fn assert_snake_case_keys(value: &serde_json::Value, path: &str) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, v) in map {
                assert!(
                    !key.chars().any(char::is_uppercase),
                    "JSON 键必须为 snake_case，发现 {path}.{key}"
                );
                assert_snake_case_keys(v, &format!("{path}.{key}"));
            }
        }
        serde_json::Value::Array(items) => {
            for (i, v) in items.iter().enumerate() {
                assert_snake_case_keys(v, &format!("{path}[{i}]"));
            }
        }
        _ => {}
    }
}

fn check<T: Serialize>(label: &str, value: &T) {
    let json = serde_json::to_value(value).expect(label);
    assert_snake_case_keys(&json, label);
}

fn kv(key: &str, value: &str) -> KeyValue {
    KeyValue::new(key, value)
}

fn request_spec() -> RequestSpec {
    RequestSpec {
        params: vec![kv("page", "1")],
        headers: vec![kv("X-Client", "Miyo")],
        path_variables: vec![kv("id", "1")],
        auth: AuthSpec::ApiKey {
            key: "X-Key".into(),
            value: "secret".into(),
            location: ApiKeyLocation::Header,
        },
        body: BodySpec::Multipart {
            fields: vec![MultipartField {
                key: "file".into(),
                value_type: miyo_core::model::MultipartValueType::FilePath,
                value: "/tmp/a.png".into(),
                enabled: true,
            }],
        },
        active_tab: Some("body".into()),
        timeout_ms: Some(30_000),
        follow_redirects: true,
        tests: None,
        disable_cookies: false,
        body_docs: std::collections::HashMap::new(),
        scripts: None,
    }
}

fn endpoint() -> Endpoint {
    Endpoint {
        id: Uuid::new_v4(),
        project_id: Uuid::new_v4(),
        folder_id: Some(Uuid::new_v4()),
        name: "宠物列表".into(),
        method: HttpMethod::GET,
        path: "/pets".into(),
        description: String::new(),
        status: EndpointStatus::Released,
        sort_order: 0,
        request: request_spec(),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    }
}

#[test]
fn ipc_models_serialize_snake_case_keys() {
    let now = chrono::Utc::now();

    check(
        "Project",
        &Project {
            id: Uuid::new_v4(),
            name: "P".into(),
            description: String::new(),
            variables: HashMap::new(),
            team_id: None,
            created_at: now,
            updated_at: now,
        },
    );

    check("Endpoint", &endpoint());
    check("RequestSpec", &request_spec());

    check(
        "Environment",
        &Environment {
            id: Uuid::new_v4(),
            project_id: Uuid::new_v4(),
            name: "开发环境".into(),
            base_url: "http://127.0.0.1:4010".into(),
            variables: vec![EnvironmentVariable {
                key: "token".into(),
                remote_value: "abc".into(),
                local_value: String::new(),
                enabled: true,
                description: Some("示例".into()),
            }],
            created_at: now,
            updated_at: now,
        },
    );

    check(
        "ResponseExample",
        &ResponseExample {
            id: Uuid::new_v4(),
            endpoint_id: Uuid::new_v4(),
            name: "200".into(),
            status: 200,
            headers: HashMap::new(),
            body: "{}".into(),
            content_type: "application/json".into(),
            docs: HashMap::new(),
            created_at: now,
            updated_at: now,
        },
    );

    check(
        "RequestExample",
        &RequestExample {
            id: Uuid::new_v4(),
            endpoint_id: Uuid::new_v4(),
            name: "示例".into(),
            request: request_spec(),
            created_at: now,
            updated_at: now,
        },
    );

    check(
        "TestCase",
        &TestCase {
            id: Uuid::new_v4(),
            request_id: Uuid::new_v4(),
            name: "正向用例".into(),
            category: "正向".into(),
            method: HttpMethod::POST,
            url_path: "/pets".into(),
            path_variables: vec![kv("id", "1")],
            params: vec![kv("page", "1")],
            headers: vec![],
            body_type: "json".into(),
            body_content: "{}".into(),
            last_run_status: TestCaseStatus::Untested,
            created_at: now,
        },
    );

    check(
        "MockRule",
        &MockRule {
            id: Uuid::new_v4(),
            project_id: Uuid::new_v4(),
            endpoint_id: None,
            name: "规则".into(),
            method: HttpMethod::GET,
            path: "/pets".into(),
            match_query: vec![MockMatchItem {
                key: "k".into(),
                value: "v".into(),
            }],
            match_headers: vec![],
            response_status: 200,
            response_headers: HashMap::new(),
            response_body_template: "{}".into(),
            delay_ms: 0,
            fault_rate_pct: 0,
            fault_status: 500,
            enabled: true,
            priority: 0,
            created_at: now,
            updated_at: now,
        },
    );

    check(
        "TestRun",
        &TestRun {
            id: Uuid::new_v4(),
            project_id: Uuid::new_v4(),
            environment_id: None,
            name: "运行".into(),
            result_json: "{}".into(),
            started_at: now,
            finished_at: None,
        },
    );

    check(
        "RequestHistory",
        &RequestHistory {
            id: Uuid::new_v4(),
            project_id: Uuid::new_v4(),
            endpoint_id: None,
            method: "GET".into(),
            url: "http://127.0.0.1/pets".into(),
            status: Some(200),
            duration_ms: Some(12),
            request_summary_json: "{}".into(),
            response_summary_json: "{}".into(),
            created_at: now,
        },
    );

    check(
        "GlobalParam",
        &GlobalParam {
            key: "X-Client".into(),
            value: "Miyo".into(),
            enabled: true,
            location: GlobalParamLocation::Header,
        },
    );

    check(
        "OAuth2Token",
        &OAuth2Token {
            access_token: "at".into(),
            token_type: "Bearer".into(),
            refresh_token: None,
            expires_at: now,
        },
    );

    check("GraphQLSpec", &GraphQLSpec::default());
}

/// 动态签名枚举的 IPC 值契约：前端按固定字符串传参，误用 `rename_all`
/// 会把 `MD5` 序列化成 `m_d5`、`SHA256`→`s_h_a256`，导致
/// `unknown variant`（见 miyo-core 历史 bug）。此处锁定 wire 格式。
#[test]
fn signature_enums_ipc_values() {
    use miyo_core::model::{SignatureAlgorithm as Algo, SignatureEncoding as Enc};

    assert_eq!(serde_json::to_value(Algo::MD5).unwrap(), "md5");
    assert_eq!(serde_json::to_value(Algo::SHA256).unwrap(), "sha256");
    assert_eq!(
        serde_json::to_value(Algo::HmacSHA256).unwrap(),
        "hmac_sha256"
    );

    assert_eq!(serde_json::to_value(Enc::HexLower).unwrap(), "hex_lower");
    assert_eq!(serde_json::to_value(Enc::HexUpper).unwrap(), "hex_upper");
    assert_eq!(serde_json::to_value(Enc::Base64).unwrap(), "base64");

    // 反序列化对称（前端传来的值能还原）。
    for algo in [Algo::MD5, Algo::SHA256, Algo::HmacSHA256] {
        let json = serde_json::to_value(algo).unwrap();
        assert_eq!(serde_json::from_value::<Algo>(json).unwrap(), algo);
    }
}

/// gRPC 契约：wire 值锁定 + BodySpec::Grpc 往返 + 旧数据兼容（`#[serde(default)]`）。
#[test]
fn grpc_ipc_values_and_roundtrip() {
    // HttpMethod::Grpc 的 wire 值（UPPERCASE rename；与历史落库数据兼容）
    assert_eq!(serde_json::to_value(HttpMethod::Grpc).unwrap(), "GRPC");
    assert_eq!(
        serde_json::from_str::<HttpMethod>("\"GRPC\"").unwrap(),
        HttpMethod::Grpc
    );
    assert_eq!(HttpMethod::Grpc.as_str(), "GRPC");
    // 方法下拉仍为纯 HTTP（gRPC 是独立协议维度）
    assert!(!HttpMethod::all().contains(&HttpMethod::Grpc));
    assert!(HttpMethod::all_with_grpc().contains(&HttpMethod::Grpc));

    // BodySpec::Grpc wire 标签与字段往返
    let spec = GrpcSpec {
        service: "grpcbin.GRPCBin".into(),
        method: "DummyUnary".into(),
        message: "{}".into(),
        use_tls: false,
        proto_ids: vec![],
    };
    let body = BodySpec::Grpc { spec };
    let v = serde_json::to_value(&body).unwrap();
    assert_eq!(v["mode"], "grpc");
    assert_eq!(v["spec"]["service"], "grpcbin.GRPCBin");
    let back: BodySpec = serde_json::from_value(v).unwrap();
    assert!(matches!(back, BodySpec::Grpc { .. }));

    // 旧版本请求规格（无 grpc 字段）反序列化兼容：default 填空 GrpcSpec
    let legacy: RequestSpec = serde_json::from_str("{}").unwrap();
    assert!(matches!(legacy.body, BodySpec::None));

    // ProtoFile 键名契约（snake_case 由上方统一扫描兜底，此处锁 roundtrip）
    let now = chrono::Utc::now();
    let file = ProtoFile {
        id: Uuid::new_v4(),
        project_id: Uuid::new_v4(),
        name: "user/v1/user.proto".into(),
        content: "syntax = \"proto3\";".into(),
        created_at: now,
        updated_at: now,
    };
    check("ProtoFile", &file);
    check("GrpcSpec", &GrpcSpec::default());
    let json = serde_json::to_value(&file).unwrap();
    let back: ProtoFile = serde_json::from_value(json).unwrap();
    assert_eq!(back, file);
}
