//! Pre-request 脚本生命周期的端到端回归（issue #193）。
//!
//! 覆盖 execute_request 里「先跑 Pre-request 脚本、再用最终变量表渲染」这条链路：
//! 脚本里 `pm.environment.set` / `pm.variables.set` 造出的值必须能替换本次请求
//! URL / 参数 / 请求头 / 请求体中的 `{{占位}}`（旧实现先渲染后跑脚本，永远取旧值）。
//!
//! 只做纯模型层 + 沙箱组合验证，不需要起 HTTP 服务或数据库。

use std::collections::HashMap;

use miyo_core::model::{BodySpec, HttpMethod, KeyValue, RequestSpec, ScriptsSpec};
use miyo_core::VariableMap;
use miyo_http::script_sandbox::{ScriptInput, ScriptRequestData, ScriptSandbox};

/// 复刻 execute_request 的脚本 + 渲染两段（不含 I/O），便于断言渲染结果。
async fn run_lifecycle(
    url: &str,
    mut spec: RequestSpec,
    vars: VariableMap,
) -> (RequestSpec, String, VariableMap) {
    let sandbox = ScriptSandbox::new().unwrap();
    let (pre_code, _post) = match &spec.scripts {
        Some(s) => (s.pre_request.clone(), s.post_response.clone()),
        None => (String::new(), String::new()),
    };
    let mut runtime_vars = vars;

    let snapshot = ScriptRequestData::from_spec(HttpMethod::GET, url, &spec);
    let r = sandbox
        .run(ScriptInput {
            code: pre_code,
            environment: std::sync::Arc::new(std::sync::Mutex::new(runtime_vars.clone())),
            request: snapshot,
            response: None,
            locals: HashMap::new(),
        })
        .await
        .unwrap();
    // 头：沿用测试里的简化口径（直接采用脚本回传列表的启用项）。
    let headers: Vec<KeyValue> = r
        .headers
        .iter()
        .filter(|(k, _)| !k.trim().is_empty())
        .map(|(k, v)| KeyValue::new(k.clone(), v.clone()))
        .collect();
    // 与 execute_request 一致：头列表来自脚本快照，但脚本没改头时仍应走变量渲染。
    let headers_untouched =
        r.headers == ScriptRequestData::from_spec(HttpMethod::GET, url, &spec).headers;
    if !headers_untouched {
        spec.headers = headers;
    }
    for (k, v) in r.environment {
        runtime_vars.insert(k, v);
    }
    let script_locals = r.locals;
    let script_request = Some(r.request);

    let render_url = match script_request.as_ref() {
        Some(req) => req.apply_to_spec(&mut spec, url),
        None => url.to_string(),
    };
    let mut render_vars = runtime_vars.clone();
    for (k, v) in &script_locals {
        render_vars.insert(k.clone(), v.clone());
    }
    let (spec, url) = miyo_core::prepare_rendered(&spec, &render_url, &render_vars);
    (spec, url, script_locals)
}

fn base_vars() -> VariableMap {
    let mut vars = VariableMap::new();
    vars.insert("base_url".into(), "http://localhost:8080".into());
    vars
}

/// issue #193 原始场景：URL 里 `time_stamp={{timestamp}}`，脚本里 set timestamp。
#[tokio::test]
async fn pre_request_environment_set_reaches_url_params_headers_and_body() {
    let spec = RequestSpec {
        params: vec![KeyValue::new("stamp", "{{timestamp}}")],
        headers: vec![KeyValue::new("X-Stamp", "{{timestamp}}")],
        body: BodySpec::Json {
            raw: r#"{"ts":"{{timestamp}}"}"#.into(),
        },
        scripts: Some(ScriptsSpec {
            pre_request: "pm.environment.set('timestamp', '1731052800000');".into(),
            post_response: String::new(),
        }),
        ..Default::default()
    };
    // URL 模板里带占位符（与前端拼好的完整地址同形态）。
    let url = "{{base_url}}/api/data?time_stamp={{timestamp}}";

    let (rendered, url, locals) = run_lifecycle(url, spec.clone(), base_vars()).await;

    assert_eq!(
        url,
        "http://localhost:8080/api/data?time_stamp=1731052800000"
    );
    assert_eq!(
        rendered
            .params
            .iter()
            .find(|kv| kv.key == "stamp")
            .map(|kv| kv.value.as_str()),
        Some("1731052800000"),
        "参数里的占位符应使用脚本写入的值"
    );
    let expected_header = KeyValue::new("X-Stamp", "1731052800000");
    assert_eq!(
        rendered.headers,
        vec![expected_header],
        "请求头里的占位符应使用脚本写入的值"
    );
    assert_eq!(
        rendered.body,
        BodySpec::Json {
            raw: r#"{"ts":"1731052800000"}"#.into()
        },
        "请求体里的占位符应使用脚本写入的值"
    );
    assert!(
        locals.is_empty(),
        "pm.environment 写的是环境表，不该进局部表"
    );

    // 对照：旧实现（先渲染再跑脚本）拿到的就是未解析原文。
    let stale = miyo_core::prepare_rendered(&spec, &url, &base_vars());
    assert!(
        stale.0.body == spec.body,
        "对照项：先渲染后跑脚本时占位符不会解析"
    );
}

/// `pm.variables.set` 只在本轮生效，且参与本次渲染。
#[tokio::test]
async fn pre_request_pm_variables_set_reaches_url() {
    let mut spec = RequestSpec {
        scripts: Some(ScriptsSpec {
            pre_request: "pm.variables.set('sig', 'abc');".into(),
            post_response: String::new(),
        }),
        ..Default::default()
    };
    spec.params = vec![KeyValue::new("sig", "{{sig}}")];
    let url = "{{base_url}}/api?sign={{sig}}";

    let (rendered, url, locals) = run_lifecycle(url, spec, base_vars()).await;

    assert_eq!(url, "http://localhost:8080/api?sign=abc");
    assert_eq!(
        rendered
            .params
            .iter()
            .find(|kv| kv.key == "sig")
            .map(|kv| kv.value.as_str()),
        Some("abc")
    );
    assert_eq!(locals.get("sig").map(String::as_str), Some("abc"));
}

/// 脚本直接改写 `pm.request.url`（Postman 习惯）也能生效，且结果不再二次渲染。
#[tokio::test]
async fn pre_request_can_rewrite_url_directly() {
    let spec = RequestSpec {
        scripts: Some(ScriptsSpec {
            pre_request: "pm.request.url = pm.request.url + '&injected=1';".into(),
            post_response: String::new(),
        }),
        ..Default::default()
    };
    let url = "{{base_url}}/api?x=1";

    let (_rendered, url, _locals) = run_lifecycle(url, spec, base_vars()).await;
    assert_eq!(url, "http://localhost:8080/api?x=1&injected=1");
}

/// 不写脚本时行为完全不变（回归保护）。
#[tokio::test]
async fn without_scripts_rendering_is_unchanged() {
    let spec = RequestSpec {
        params: vec![KeyValue::new("q", "{{q}}")],
        ..Default::default()
    };
    let mut vars = base_vars();
    vars.insert("q".into(), "hello".into());
    let (rendered, url, locals) = run_lifecycle("{{base_url}}/api?t={{q}}", spec, vars).await;
    assert_eq!(url, "http://localhost:8080/api?t=hello");
    assert_eq!(
        rendered
            .params
            .iter()
            .find(|kv| kv.key == "q")
            .map(|kv| kv.value.as_str()),
        Some("hello")
    );
    assert!(locals.is_empty());
}
