//! 安全策略：体积 / 超时 / 并发上限与危险类黑名单（Miyo 侧预检，SDK 侧同规则终检）。
//!
//! Miyo 侧预检的意义：在请求发出前拦截明显违规的调用（超限参数、超长超时、
//! 黑名单目标），把错误留在本地；SDK 侧仍然是最终裁决方——两道闸缺一不可。

use miyo_core::AppError;

/// 默认禁止注册 / 调用的类（前缀匹配）。SDK 侧注册与调用时同样检查。
///
/// 原则：凡是「拿到即等于任意代码执行 / 进程逃逸 / JVM 自改」的能力一律
/// 拒绝。显式注册模型下用户不会主动注册这些类，兜底防误配与防协议滥用。
pub const BLACKLISTED_CLASSES: &[&str] = &[
    "java.lang.Runtime",
    "java.lang.ProcessBuilder",
    "java.lang.ProcessHandle",
    "java.lang.System",
    "java.lang.ClassLoader",
    "java.lang.SecurityManager",
    "java.lang.Thread",
    "java.lang.ThreadGroup",
    "java.lang.reflect.",
    "java.lang.invoke.",
    "javax.script.",
    "jdk.nashorn.",
    "jdk.dynalink.",
    "sun.misc.Unsafe",
    "java.lang.Module",
    "java.lang.ModuleLayer",
    "java.lang.instrument.",
    "java.io.File",
    "java.io.RandomAccessFile",
    "java.nio.file.Files",
    "java.nio.file.Paths",
    "java.net.Socket",
    "java.net.ServerSocket",
    "javax.net.ssl.SSLSocketFactory",
];

/// 调用安全策略（每个 Java 项目一份，随项目配置落库）。
#[derive(Debug, Clone, PartialEq)]
pub struct SecurityPolicy {
    /// 单次调用参数序列化后最大字节数。
    pub max_args_bytes: usize,
    /// 返回值最大字节数（超出截断并标记 `result_truncated`）。
    pub max_result_bytes: usize,
    /// 单次调用超时上限（毫秒）；超出即拒绝。
    pub max_timeout_ms: u64,
    /// 单方法最大参数个数。
    pub max_params: usize,
    /// 对同一 Runtime 的最大在途调用数（SDK 侧同样限流，双保险）。
    pub max_concurrent_invocations: u32,
}

impl Default for SecurityPolicy {
    fn default() -> Self {
        SecurityPolicy {
            max_args_bytes: 1024 * 1024,   // 1 MiB
            max_result_bytes: 1024 * 1024, // 1 MiB
            max_timeout_ms: 300_000,       // 与全局请求超时上限对齐
            max_params: 32,
            max_concurrent_invocations: 4,
        }
    }
}

impl SecurityPolicy {
    /// 参数预检：个数上限 + 序列化体积上限。
    pub fn validate_arguments(&self, args: &[serde_json::Value]) -> Result<usize, AppError> {
        if args.len() > self.max_params {
            return Err(AppError::Validation(format!(
                "参数个数 {} 超过上限 {}",
                args.len(),
                self.max_params
            )));
        }
        let bytes = serde_json::to_vec(args)
            .map_err(|e| AppError::Validation(format!("参数无法序列化为 JSON：{e}")))?;
        if bytes.len() > self.max_args_bytes {
            return Err(AppError::Validation(format!(
                "参数体积 {} 字节超过上限 {} 字节",
                bytes.len(),
                self.max_args_bytes
            )));
        }
        Ok(bytes.len())
    }

    /// 超时收敛：None → 默认（取上限的一半，偏保守）；超出上限直接拒绝。
    pub fn resolve_timeout_ms(&self, requested: Option<u64>) -> Result<u64, AppError> {
        let requested = requested.unwrap_or(self.max_timeout_ms / 2);
        if requested == 0 {
            return Err(AppError::Validation("超时时间必须大于 0".into()));
        }
        if requested > self.max_timeout_ms {
            return Err(AppError::Validation(format!(
                "超时 {}ms 超过上限 {}ms（可在 Java 项目设置中调整）",
                requested, self.max_timeout_ms
            )));
        }
        Ok(requested)
    }

    /// 黑名单预检：method_id 的类名命中前缀即拒绝。
    pub fn check_method_allowed(&self, method_id: &str) -> Result<(), AppError> {
        let class = method_id
            .split_once('#')
            .map(|(c, _)| c)
            .ok_or_else(|| AppError::Validation(format!("method_id 非法：{method_id}")))?;
        if blacklisted(class) {
            return Err(AppError::Forbidden(format!(
                "{class} 属于受限类，禁止通过调试协议调用"
            )));
        }
        Ok(())
    }

    /// 返回值截断（保留合法 UTF-8 前缀，标记截断）。
    pub fn truncate_result(&self, value: serde_json::Value) -> (serde_json::Value, bool) {
        let Ok(raw) = serde_json::to_string(&value) else {
            return (serde_json::Value::Null, false);
        };
        if raw.len() <= self.max_result_bytes {
            return (value, false);
        }
        // 体积超限：丢原文，仅保留结构占位（避免半截 JSON 让前端解析失败）。
        (
            serde_json::json!({
                "__truncated": true,
                "original_bytes": raw.len(),
                "max_bytes": self.max_result_bytes,
            }),
            true,
        )
    }
}

/// 黑名单前缀匹配（大小写敏感——Java 类名本就大小写敏感）。
pub fn blacklisted(class_name: &str) -> bool {
    BLACKLISTED_CLASSES
        .iter()
        .any(|p| class_name.starts_with(p))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn args_size_limit() {
        let policy = SecurityPolicy {
            max_args_bytes: 64,
            ..Default::default()
        };
        assert!(policy.validate_arguments(&[json!({"k": "v"})]).is_ok());
        let big = [json!({"k": "x".repeat(100)})];
        assert!(policy.validate_arguments(&big).is_err());
    }

    #[test]
    fn args_count_limit() {
        let policy = SecurityPolicy::default();
        let args: Vec<serde_json::Value> = (0..policy.max_params).map(|i| json!(i)).collect();
        assert!(policy.validate_arguments(&args).is_ok());
        assert!(policy.validate_arguments(&args).is_ok());
        let mut over = args;
        over.push(json!(1));
        assert!(policy.validate_arguments(&over).is_err());
    }

    #[test]
    fn timeout_clamp() {
        let policy = SecurityPolicy::default();
        assert_eq!(policy.resolve_timeout_ms(None).unwrap(), 150_000);
        assert_eq!(policy.resolve_timeout_ms(Some(5_000)).unwrap(), 5_000);
        assert!(policy.resolve_timeout_ms(Some(301_000)).is_err());
        assert!(policy.resolve_timeout_ms(Some(0)).is_err());
    }

    #[test]
    fn blacklist_matches_prefixes() {
        for cls in [
            "java.lang.Runtime",
            "java.lang.ProcessBuilder",
            "java.lang.reflect.Method",
            "java.lang.invoke.MethodHandles",
            "sun.misc.Unsafe",
            "javax.script.ScriptEngineManager",
        ] {
            assert!(blacklisted(cls), "{cls} 应被拦截");
        }
        for cls in [
            "com.example.OrderService",
            "java.lang.String",
            "java.lang.StringBuilder",
        ] {
            assert!(!blacklisted(cls), "{cls} 不应被拦截");
        }
    }

    #[test]
    fn method_check_rejects_blacklisted() {
        let policy = SecurityPolicy::default();
        assert!(policy
            .check_method_allowed("java.lang.Runtime#exec(String)")
            .is_err());
        assert!(policy
            .check_method_allowed("com.example.OrderService#createOrder(CreateOrderRequest)")
            .is_ok());
    }

    #[test]
    fn result_truncation() {
        let policy = SecurityPolicy {
            max_result_bytes: 32,
            ..Default::default()
        };
        let (small, truncated) = policy.truncate_result(json!({"a": 1}));
        assert!(!truncated);
        assert_eq!(small, json!({"a": 1}));
        let (big, truncated) = policy.truncate_result(json!({"a": "x".repeat(100)}));
        assert!(truncated);
        assert_eq!(big["__truncated"], json!(true));
        assert!(big["original_bytes"].as_u64().unwrap() > 32);
    }
}
