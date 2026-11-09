//! 测试配置解析（SPEC §17.3）。

use serde::{Deserialize, Serialize};

/// 变量设置：pre_request。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetVariable {
    pub name: String,
    pub value: String,
}

/// 变量提取：extract。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtractSpec {
    pub name: String,
    /// "body" 或 "header"。
    #[serde(default = "default_extract_from")]
    pub from: String,
    /// body 时是 JSONPath（如 $.id），header 时是头名。
    pub path: String,
}

fn default_extract_from() -> String {
    "body".to_string()
}

/// 步骤执行条件（不满足 → 整步 skipped，不发送请求）。
///
/// - `source = "var"`：对运行时变量求值（`name` 为变量名，每轮循环重取最新值）；
/// - `source = "prevStatus"`：对最近一个已执行步骤的 HTTP 状态码求值；
/// - `op` 与断言算子同集（eq/neq/contains/…/exists/empty），
///   `value` 为期望值（数字/布尔/字符串按断言同款宽松比较）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StepCondition {
    /// "var"（缺省） | "prevStatus"。
    #[serde(default = "default_condition_source")]
    pub source: String,
    /// source = var 时的变量名。
    #[serde(default)]
    pub name: String,
    /// 响应源取值路径：`response.body` 时为 JSONPath（如 `$.code`），
    /// `response.header` 时为头名。`var` / `prevStatus` / `response.status`
    /// 忽略此字段。AI 方言与 extractors / assertions 的 `expression` 同名。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expression: Option<String>,
    /// 比较算子（同 AssertionSpec.op）。
    pub op: String,
    #[serde(default)]
    pub value: Option<serde_json::Value>,
}

fn default_condition_source() -> String {
    "var".to_string()
}

impl StepCondition {
    pub fn is_valid(&self) -> bool {
        match self.source.as_str() {
            "var" => !self.name.trim().is_empty() && !self.op.trim().is_empty(),
            "prevStatus" => !self.op.trim().is_empty(),
            _ => false,
        }
    }

    /// while 循环继续条件的合法性（比执行条件宽松）：每轮结束后对本轮
    /// 响应求值，因此响应源也可用，语义与断言 / extractors 的 source 对齐。
    pub fn is_valid_while(&self) -> bool {
        match self.source.as_str() {
            "var" => !self.name.trim().is_empty() && !self.op.trim().is_empty(),
            "response.status" | "prevStatus" => !self.op.trim().is_empty(),
            "response.body" | "response.header" => {
                !self.op.trim().is_empty()
                    && self
                        .expression
                        .as_deref()
                        .is_some_and(|e| !e.trim().is_empty())
            }
            _ => false,
        }
    }
}

/// 步骤循环：同一请求按条件重复发送（轮询 / 重试放行场景）。
///
/// - `mode = "count"`：固定执行 `count` 轮；
/// - `mode = "while"`：每轮结束后对 `while_condition` 求值，true 继续，
///   最多 `max_loops` 轮（达上限仍 true 判失败——轮询超时语义）；
///   `while_condition.source` 支持 `var` / `prevStatus` / `response.status` /
///   `response.body` / `response.header`——响应源直接对本轮响应求值
///   （异步落账轮询「本轮响应码 ≠ X 则重试」无需先 extract 再引用，issue #156）；
/// - 任一轮断言失败 / 请求失败即终止整步（failed）；取消即时中止；
/// - `interval_ms` 为轮间隔（轮询节流）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StepLoop {
    /// "count" | "while"。
    pub mode: String,
    /// count 模式的轮数（1..=1000）。
    #[serde(default)]
    pub count: Option<u32>,
    /// while 模式的继续条件（每轮结束后求值）。
    #[serde(default)]
    pub while_condition: Option<StepCondition>,
    /// while 模式轮数上限（1..=100，缺省 10）。
    #[serde(default)]
    pub max_loops: Option<u32>,
    /// 轮间隔毫秒（0..=60_000，缺省 0）。
    #[serde(default)]
    pub interval_ms: Option<u64>,
}

impl StepLoop {
    pub const MAX_COUNT: u32 = 1000;
    pub const MAX_LOOPS: u32 = 100;
    pub const MAX_INTERVAL_MS: u64 = 60_000;

    pub fn is_valid(&self) -> bool {
        match self.mode.as_str() {
            "count" => self.count.is_some(),
            "while" => self
                .while_condition
                .as_ref()
                .is_some_and(|c| c.is_valid_while()),
            _ => false,
        }
    }

    /// while 配置的具体问题（校验 / 运行错误信息用；None = 合法或非 while）。
    ///
    /// 错误话术直接指向真实原因（source / expression / 缺条件），不再笼统
    /// 报「mode/count/while_condition 组合不合法」——那会把 source 用法错误
    /// 误导向 mode 组合，排查成本极高（issue #156）。
    pub fn while_problem(&self) -> Option<String> {
        if self.mode != "while" {
            return None;
        }
        let c = self.while_condition.as_ref()?;
        if c.is_valid_while() {
            return None;
        }
        Some(match c.source.as_str() {
            "var" => "while_condition.source=var 需要提供 name（变量名）与 op".into(),
            "response.body" | "response.header" => format!(
                "while_condition.source={} 需要提供 expression（{}）与 op",
                c.source,
                if c.source == "response.body" { "JSONPath" } else { "头名" }
            ),
            "status" | "header" | "body" => format!(
                "while_condition.source 不支持简写「{}」——请用 response.{}",
                c.source, c.source
            ),
            other => format!(
                "while_condition.source 仅支持 var / prevStatus / response.status / response.body / response.header，收到 {other}"
            ),
        })
    }

    /// 有效轮数上限（收钳到安全范围）。
    pub fn rounds_cap(&self) -> u32 {
        match self.mode.as_str() {
            "count" => self.count.unwrap_or(1).clamp(1, Self::MAX_COUNT),
            _ => self.max_loops.unwrap_or(10).clamp(1, Self::MAX_LOOPS),
        }
    }

    pub fn interval(&self) -> u64 {
        self.interval_ms.unwrap_or(0).min(Self::MAX_INTERVAL_MS)
    }
}

/// 断言项。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssertionSpec {
    #[serde(default)]
    pub name: Option<String>,
    /// status | header | body | jsonpath | response_time_ms
    pub r#type: String,
    /// header 时是头名；jsonpath 时是路径；body 时忽略。
    #[serde(default)]
    pub path: Option<String>,
    /// eq | neq | contains | not_contains | gt | gte | lt | lte | exists | not_exists
    /// | matches | not_matches（正则） | empty | not_empty。
    /// type 扩展：graphql_errors（body.errors 数组）与 length（path 处值的长度，走数字比较）。
    #[serde(default)]
    pub op: Option<String>,
    #[serde(default)]
    pub expected: Option<serde_json::Value>,
}

/// 完整测试规格（对应 request_json 里的 "tests"）。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TestSpec {
    #[serde(default)]
    pub pre_request: Vec<SetVariable>,
    #[serde(default)]
    pub extract: Vec<ExtractSpec>,
    #[serde(default)]
    pub assertions: Vec<AssertionSpec>,
}

impl TestSpec {
    /// 从 request_json.tests 解析；缺失或非法返回 None（非法时收集原因）。
    pub fn from_request_value(value: Option<&serde_json::Value>) -> Result<TestSpec, String> {
        let Some(value) = value else {
            return Ok(TestSpec::default());
        };
        if value.is_null() {
            return Ok(TestSpec::default());
        }
        serde_json::from_value(value.clone()).map_err(|e| format!("测试配置解析失败：{e}"))
    }

    pub fn to_value(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    pub fn is_empty(&self) -> bool {
        self.pre_request.is_empty() && self.extract.is_empty() && self.assertions.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_absent_and_null() {
        assert!(TestSpec::from_request_value(None).unwrap().is_empty());
        assert!(TestSpec::from_request_value(Some(&serde_json::Value::Null))
            .unwrap()
            .is_empty());
    }

    #[test]
    fn parse_spec_example() {
        let v = json!({
            "pre_request": [
                {"type": "set_variable", "name": "timestamp", "value": "{{$timestamp}}"}
            ],
            "extract": [
                {"name": "userId", "from": "body", "path": "$.id"}
            ],
            "assertions": [
                {"type": "status", "op": "eq", "expected": 200},
                {"type": "jsonpath", "path": "$.name", "op": "contains", "expected": "test"},
                {"type": "response_time_ms", "op": "lt", "expected": 2000}
            ]
        });
        let spec = TestSpec::from_request_value(Some(&v)).unwrap();
        assert_eq!(spec.pre_request.len(), 1);
        assert_eq!(spec.pre_request[0].name, "timestamp");
        assert_eq!(spec.extract.len(), 1);
        assert_eq!(spec.extract[0].path, "$.id");
        assert_eq!(spec.assertions.len(), 3);
        assert_eq!(spec.assertions[2].r#type, "response_time_ms");
        assert_eq!(spec.assertions[2].op.as_deref(), Some("lt"));
        // roundtrip
        let back = TestSpec::from_request_value(Some(&spec.to_value())).unwrap();
        assert_eq!(back, spec);
    }

    #[test]
    fn invalid_json_fails() {
        assert!(TestSpec::from_request_value(Some(&json!({"assertions": [{"type": 1}]}))).is_err());
    }

    #[test]
    fn parse_step_condition_and_loop() {
        let c: StepCondition = serde_json::from_value(json!({
            "source": "var", "name": "status", "op": "eq", "value": "PAID"
        }))
        .unwrap();
        assert!(c.is_valid());
        let prev: StepCondition =
            serde_json::from_value(json!({"source": "prevStatus", "op": "eq", "value": 200}))
                .unwrap();
        assert!(prev.is_valid());
        assert!(!StepCondition {
            source: "var".into(),
            name: "  ".into(),
            expression: None,
            op: "eq".into(),
            value: None
        }
        .is_valid());

        let l: StepLoop = serde_json::from_value(json!({
            "mode": "while", "max_loops": 5, "interval_ms": 500,
            "while_condition": {"source": "var", "name": "paid", "op": "neq", "value": "true"}
        }))
        .unwrap();
        assert!(l.is_valid());
        assert_eq!(l.rounds_cap(), 5);
        assert_eq!(l.interval(), 500);
        let count_loop: StepLoop =
            serde_json::from_value(json!({"mode": "count", "count": 3})).unwrap();
        assert_eq!(count_loop.rounds_cap(), 3);
        assert!(!StepLoop {
            mode: "while".into(),
            count: None,
            while_condition: None,
            max_loops: None,
            interval_ms: None
        }
        .is_valid());

        // ---- while_condition 响应源（issue #156）：合法性与错误话术 ----
        let wc: StepCondition = serde_json::from_value(json!({
            "source": "response.body", "expression": "$.code", "op": "neq", "value": 200
        }))
        .unwrap();
        assert!(wc.is_valid_while());
        assert!(
            !wc.is_valid(),
            "响应源仍不得用于步骤执行条件（无响应可求值）"
        );
        // expression 缺失 / 空白 → 无效
        let no_expr: StepCondition =
            serde_json::from_value(json!({"source": "response.body", "op": "neq"})).unwrap();
        assert!(!no_expr.is_valid_while());
        let blank: StepCondition = serde_json::from_value(json!(
            {"source": "response.header", "expression": "  ", "op": "eq"}
        ))
        .unwrap();
        assert!(!blank.is_valid_while());
        // 简写 source 与未知 source → 无效，且错误话术指向真实原因
        let shorthand: StepCondition =
            serde_json::from_value(json!({"source": "status", "op": "neq"})).unwrap();
        assert!(!shorthand.is_valid_while());
        let bad_loop: StepLoop = serde_json::from_value(json!({
            "mode": "while", "max_loops": 5,
            "while_condition": {"source": "status", "op": "neq"}
        }))
        .unwrap();
        assert!(!bad_loop.is_valid());
        let problem = bad_loop.while_problem().unwrap();
        assert!(
            problem.contains("response.status"),
            "话术应指向真实原因：{problem}"
        );
        assert!(
            !problem.contains("mode/count"),
            "不应再误导为 mode 组合问题：{problem}"
        );
        let ok_loop: StepLoop = serde_json::from_value(json!({
            "mode": "while", "max_loops": 10, "interval_ms": 1000,
            "while_condition": {"source": "response.body", "expression": "$.code", "op": "neq", "value": 200}
        }))
        .unwrap();
        assert!(ok_loop.is_valid());
        assert!(ok_loop.while_problem().is_none());
    }
}
