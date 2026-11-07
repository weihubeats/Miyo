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
}

/// 步骤循环：同一请求按条件重复发送（轮询 / 重试放行场景）。
///
/// - `mode = "count"`：固定执行 `count` 轮；
/// - `mode = "while"`：每轮结束后对 `while_condition` 求值，true 继续，
///   最多 `max_loops` 轮（达上限仍 true 判失败——轮询超时语义）；
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
            "while" => self.while_condition.as_ref().is_some_and(|c| c.is_valid()),
            _ => false,
        }
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
    }
}
