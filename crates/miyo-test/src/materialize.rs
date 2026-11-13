//! 步骤物化单源：tauri IPC 与 agent 控制面共用的「步骤 → ScenarioStepInput」编排。
//!
//! 历史包袱：同一物化链（apply_global_params → TestSpec 解析 → condition /
//! loop 解析 → wait_ms 收钳 → ScenarioStepInput 构造）曾在 tauri
//! `commands/scenarios.rs` 里存在 3 份闭包、agent `server.rs` 里 2 份实现，
//! 行为口径靠人工同步。本模块把它收敛为一个纯函数：调用方把各自的步骤模型
//! / DB 解析为 [`StepMaterialInput`]（纯数据），物化细节在此单源。
//!
//! 刻意留在调用方的差异项：步骤来源解析（agent 的 DB 查询 / 内联合成
//! Endpoint 与 `url_template_for`）、变量来源（UI 态 vs 控制面显式传参）、
//! 脱敏、事件推送 / 进度表、落库——这些是两端的真实边界，不是重复。

use miyo_core::model::{Endpoint, GlobalParam, StepRetry};
use miyo_core::VariableMap;

use crate::config::{StepCondition, StepLoop, TestSpec};
use crate::runner::ScenarioStepInput;

/// 步骤物化输入（纯数据；步骤来源解析由调用方完成）。
#[derive(Debug, Clone)]
pub struct StepMaterialInput {
    /// 步骤归属接口（引用式步骤为 DB 当前配置；内联步骤为合成端点）。
    pub endpoint: Endpoint,
    /// URL 模板（保留 `{{var}}`，执行期由引擎渲染）。
    pub url_template: String,
    /// 提取规则 JSON（miyo-test `ExtractSpec` 数组形状透传）。
    pub extract_json: serde_json::Value,
    /// 断言规则 JSON（miyo-test `AssertionSpec` 数组形状透传）。
    pub assertions_json: serde_json::Value,
    /// 执行条件 JSON（`StepCondition` 形状；None = 无条件）。
    pub condition_json: Option<serde_json::Value>,
    /// 循环配置 JSON（`StepLoop` 形状；None = 不循环）。
    pub loop_json: Option<serde_json::Value>,
    /// 步骤重试策略（显式配置才启用）。
    pub retry: Option<StepRetry>,
    /// 是否启用（禁用步骤照传，runner 落 skipped 行）。
    pub enabled: bool,
    /// 执行前等待毫秒（自动收钳到 `StepLoop::MAX_INTERVAL_MS`）。
    pub wait_ms: Option<u64>,
    /// 与相邻 parallel 步骤成批并发。
    pub parallel: bool,
    /// 步骤参数覆盖（JSON；`url` 键已由调用方先行替换 url_template）。
    pub overrides_json: Option<serde_json::Value>,
    /// 写方法自动幂等键（Some(键) 时：方法为 POST/PUT/PATCH/DELETE 且未
    /// 显式配置 `Idempotency-Key` 头则注入；None = 不注入）。
    pub auto_idempotency_key: Option<String>,
    /// 错误信息里的步骤标签（如 `第 3 步` / `setup/teardown 第 1 步`）。
    pub step_label: String,
}

/// 单步物化：全局参数补缺 → 参数覆盖 → 自动幂等键 → 提取/断言解析 →
/// 执行控制解析 → 引擎输入。失败返回带步骤标签的校验错误描述。
pub fn materialize_step(
    input: StepMaterialInput,
    global_params: &[GlobalParam],
    vars: &VariableMap,
) -> Result<ScenarioStepInput, String> {
    let label = &input.step_label;
    let mut spec = input.endpoint.request.clone();
    // 顺序对齐 agent 历史实现：参数覆盖 → 自动幂等键 → 全局参数补缺
    //（全局参数若配置了 Idempotency-Key 键，注入顺序影响 has_key 判定结果）。
    if let Some(ov) = &input.overrides_json {
        miyo_core::apply_step_overrides(&mut spec, ov)
            .map_err(|e| format!("{label}的参数覆盖无效：{e}"))?;
    }
    if let Some(key) = &input.auto_idempotency_key {
        // 写操作自动幂等：POST/PUT/PATCH/DELETE 且未显式配置 Idempotency-Key
        // 时补键（键 = runId-位次；循环轮次共用——同一逻辑操作重试不重复执行）。
        let is_write = ["POST", "PUT", "PATCH", "DELETE"]
            .contains(&input.endpoint.method.as_str().to_uppercase().as_str());
        let has_key = spec
            .headers
            .iter()
            .any(|h| h.enabled && h.key.eq_ignore_ascii_case("Idempotency-Key"));
        if is_write && !has_key {
            spec.headers.push(miyo_core::model::KeyValue::new(
                "Idempotency-Key",
                key.clone(),
            ));
        }
    }
    // 全局参数补缺（key 大小写不敏感、不覆盖步骤显式值）。
    miyo_core::apply_global_params(&mut spec, global_params, vars);
    let config = TestSpec::from_request_value(Some(&serde_json::json!({
        "extract": input.extract_json,
        "assertions": input.assertions_json,
    })))
    .map_err(|e| format!("{label}的提取/断言配置无效：{e}"))?;
    let condition = input
        .condition_json
        .as_ref()
        .map(|v| serde_json::from_value::<StepCondition>(v.clone()))
        .transpose()
        .map_err(|e| format!("{label}的执行条件无效：{e}"))?;
    let r#loop = input
        .loop_json
        .as_ref()
        .map(|v| serde_json::from_value::<StepLoop>(v.clone()))
        .transpose()
        .map_err(|e| format!("{label}的循环配置无效：{e}"))?;
    Ok(ScenarioStepInput {
        endpoint: input.endpoint,
        url_template: input.url_template,
        spec_template: spec,
        retry: input.retry,
        extract: config.extract,
        assertions: config.assertions,
        enabled: input.enabled,
        condition,
        r#loop,
        wait_ms: input.wait_ms.map(|w| w.min(StepLoop::MAX_INTERVAL_MS)),
        parallel: input.parallel,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use miyo_core::model::{EndpointStatus, HttpMethod, KeyValue};

    fn ep(method: HttpMethod) -> Endpoint {
        Endpoint {
            id: uuid::Uuid::new_v4(),
            project_id: uuid::Uuid::new_v4(),
            folder_id: None,
            name: "e".into(),
            method,
            path: "/x".into(),
            description: String::new(),
            status: EndpointStatus::Developing,
            sort_order: 0,
            request: miyo_core::model::RequestSpec::default(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    fn input(method: HttpMethod) -> StepMaterialInput {
        StepMaterialInput {
            endpoint: ep(method),
            url_template: "{{base_url}}/x".into(),
            extract_json: serde_json::json!([]),
            assertions_json: serde_json::json!([]),
            condition_json: None,
            loop_json: None,
            retry: None,
            enabled: true,
            wait_ms: None,
            parallel: false,
            overrides_json: None,
            auto_idempotency_key: None,
            step_label: "第 1 步".into(),
        }
    }

    /// 全局参数补缺：步骤未配置的同名键补上，显式配置的不覆盖。
    #[test]
    fn global_params_fill_missing_not_override() {
        let mut spec = input(HttpMethod::GET).endpoint.request.clone();
        spec.headers.push(KeyValue::new("X-Step", "step"));
        let mut inp = input(HttpMethod::GET);
        inp.endpoint.request = spec;
        let params = vec![
            GlobalParam {
                key: "X-Step".into(),
                value: "global".into(),
                enabled: true,
                location: miyo_core::model::GlobalParamLocation::Header,
            },
            GlobalParam {
                key: "X-Only".into(),
                value: "g".into(),
                enabled: true,
                location: miyo_core::model::GlobalParamLocation::Header,
            },
        ];
        let out = materialize_step(inp, &params, &Default::default()).unwrap();
        let step_header = out
            .spec_template
            .headers
            .iter()
            .find(|h| h.key == "X-Step")
            .unwrap();
        assert_eq!(step_header.value, "step", "步骤显式值不被全局参数覆盖");
        assert!(out.spec_template.headers.iter().any(|h| h.key == "X-Only"));
    }

    /// 自动幂等：写方法且无显式键 → 注入；读方法 / 已有键 → 不动。
    #[test]
    fn auto_idempotency_only_for_write_without_explicit_key() {
        let mut inp = input(HttpMethod::POST);
        inp.auto_idempotency_key = Some("run-1-0".into());
        let out = materialize_step(inp, &[], &Default::default()).unwrap();
        assert!(out
            .spec_template
            .headers
            .iter()
            .any(|h| h.key == "Idempotency-Key" && h.value == "run-1-0"));

        let inp = input(HttpMethod::GET).clone();
        let out = materialize_step(inp, &[], &Default::default()).unwrap();
        assert!(!out
            .spec_template
            .headers
            .iter()
            .any(|h| h.key == "Idempotency-Key"));

        let mut inp = input(HttpMethod::POST);
        inp.auto_idempotency_key = Some("run-1-0".into());
        inp.endpoint
            .request
            .headers
            .push(KeyValue::new("idempotency-key", "mine"));
        let out = materialize_step(inp, &[], &Default::default()).unwrap();
        let keys: Vec<_> = out
            .spec_template
            .headers
            .iter()
            .filter(|h| h.key.eq_ignore_ascii_case("Idempotency-Key"))
            .collect();
        assert_eq!(keys.len(), 1, "已有显式键不重复注入");
        assert_eq!(keys[0].value, "mine");
    }

    /// wait_ms 收钳到 StepLoop::MAX_INTERVAL_MS；步骤标签进入错误信息。
    #[test]
    fn wait_ms_clamped_and_label_in_errors() {
        let mut inp = input(HttpMethod::GET);
        inp.wait_ms = Some(u64::MAX);
        let out = materialize_step(inp, &[], &Default::default()).unwrap();
        assert_eq!(out.wait_ms, Some(StepLoop::MAX_INTERVAL_MS));

        let mut inp = input(HttpMethod::GET);
        inp.assertions_json = serde_json::json!([{"bad": 1}]);
        let err = materialize_step(inp, &[], &Default::default()).unwrap_err();
        assert!(err.contains("第 1 步"), "错误信息含步骤标签：{err}");
    }

    /// 参数覆盖：headers 合并（显式行替换 / 新键追加）。
    #[test]
    fn overrides_merge_headers() {
        let mut inp = input(HttpMethod::GET);
        inp.endpoint
            .request
            .headers
            .push(KeyValue::new("X-A", "old"));
        inp.overrides_json = Some(serde_json::json!({
            "headers": [{"key": "X-A", "value": "new", "enabled": true}, {"key": "X-B", "value": "b", "enabled": true}]
        }));
        let out = materialize_step(inp, &[], &Default::default()).unwrap();
        let a = out
            .spec_template
            .headers
            .iter()
            .find(|h| h.key == "X-A")
            .unwrap();
        assert_eq!(a.value, "new");
        assert!(out.spec_template.headers.iter().any(|h| h.key == "X-B"));
    }
}
