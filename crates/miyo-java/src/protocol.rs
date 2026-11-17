//! 调试协议 v1 的类型定义与稳定方法标识（method_id）。
//!
//! 类型两端共享：Rust 侧序列化 / SDK（Java）侧逐字段对齐
//!（`java/miyo-java-runtime/src/main/java/io/miyo/debug/json/ProtocolJson.java`）。
//! 字段命名 snake_case；改动必须同时升级 `PROTOCOL_VERSION` 并同步 SDK。

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// 当前调试协议版本。不兼容变更（字段删除 / 语义变化）必须 +1，
/// 客户端在 `info` 握手时校验，不匹配即拒绝连接。
pub const PROTOCOL_VERSION: u32 = 1;

/// 请求头：协议版本。
pub const HEADER_PROTOCOL: &str = "X-Miyo-Protocol-Version";
/// 请求头：Bearer Token（值前缀 `"Bearer "`）。
pub const HEADER_AUTHORIZATION: &str = "Authorization";

/// Runtime 自述信息（`GET /miyo/debug/info`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RuntimeInfo {
    /// Runtime 支持的协议版本（与 [`PROTOCOL_VERSION`] 不等即握手失败）。
    pub protocol_version: u32,
    /// SDK 版本（如 `0.1.0`）。
    pub runtime_version: String,
    /// 目标应用名（SDK 配置注册，缺省取主类名）。
    pub app_name: String,
    /// `java.version`（如 `21.0.6`）。
    pub java_version: String,
    /// JVM 名 + 版本（如 `OpenJDK 64-Bit Server VM 21.0.6`）。
    pub jvm_version: String,
    /// Runtime 已启动时长（毫秒）。
    pub uptime_ms: u64,
    /// 已注册（白名单内）类数量。
    pub class_count: usize,
    /// 已注册（白名单内）方法数量。
    pub method_count: usize,
    /// 实例提供方式：`static`（静态/新建）或 `spring`（Bean 容器解析）。
    pub instance_provider: String,
}

impl Default for RuntimeInfo {
    fn default() -> Self {
        RuntimeInfo {
            protocol_version: PROTOCOL_VERSION,
            runtime_version: String::new(),
            app_name: String::new(),
            java_version: String::new(),
            jvm_version: String::new(),
            uptime_ms: 0,
            class_count: 0,
            method_count: 0,
            instance_provider: "static".into(),
        }
    }
}

/// 白名单类信息（`GET /miyo/debug/classes`）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct JavaClassInfo {
    /// 类全限定名（如 `com.example.order.service.OrderService`）。
    pub class_name: String,
    /// 包名（无包类为空串）。
    pub package_name: String,
    /// 简名（`OrderService`）。
    pub simple_name: String,
    /// 白名单内方法数量。
    pub method_count: usize,
    /// Spring Bean 名或注册名（无则 None）。
    pub bean_name: Option<String>,
}

/// 参数描述（类型为擦除后名称，泛型经 `generic_type` 补充展示）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct JavaParamInfo {
    /// 参数名（编译期未带 `-parameters` 时退化为 `arg0..n`）。
    pub name: String,
    /// 擦除后类型（`long` / `List` / `CreateOrderRequest`）。
    pub type_name: String,
    /// 泛型展示（如 `List<Long>`；无泛型为空串）。
    pub generic_type: String,
    /// 基本类型必填；引用类型可传 null。
    pub required: bool,
}

/// 白名单方法信息（`GET /miyo/debug/methods?class={FQN}`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct JavaMethodInfo {
    /// 稳定方法标识（见模块文档；调用时唯一依据）。
    pub method_id: String,
    /// 所属类 FQN。
    pub class_name: String,
    /// 方法名。
    pub method_name: String,
    /// 人类可读完整签名（`public OrderResult createOrder(CreateOrderRequest, String)`）。
    pub signature: String,
    /// 返回类型擦除名（`void` 原样）。
    pub return_type: String,
    /// 返回类型泛型展示（空串 = 无）。
    pub return_generic: String,
    pub params: Vec<JavaParamInfo>,
    /// 静态方法（无需实例）。
    pub static_method: bool,
    /// 可见性：public / protected / package / private。
    pub visibility: String,
    /// 同名不同参的重载是否存在于白名单内。
    pub overloaded: bool,
    /// 声明类（方法可能声明于父类）。
    pub declaring_class: String,
}

impl Default for JavaMethodInfo {
    fn default() -> Self {
        JavaMethodInfo {
            method_id: String::new(),
            class_name: String::new(),
            method_name: String::new(),
            signature: String::new(),
            return_type: String::new(),
            return_generic: String::new(),
            params: Vec::new(),
            static_method: false,
            visibility: "public".into(),
            overloaded: false,
            declaring_class: String::new(),
        }
    }
}

impl JavaMethodInfo {
    /// 按 Miyo 展示约定生成签名（`name(param: Type, param: Type) -> ReturnType`）。
    pub fn display_signature(&self) -> String {
        let params = self
            .params
            .iter()
            .map(|p| {
                let t = if p.generic_type.is_empty() {
                    p.type_name.clone()
                } else {
                    p.generic_type.clone()
                };
                format!("{}: {}", p.name, t)
            })
            .collect::<Vec<_>>()
            .join(", ");
        let ret = if self.return_generic.is_empty() {
            self.return_type.clone()
        } else {
            self.return_generic.clone()
        };
        format!("{}({}) -> {}", self.method_name, params, ret)
    }
}

/// 调用请求（`POST /miyo/debug/invoke`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InvokeRequest {
    /// 客户端生成的追踪 id（取消与审计的关联键）。
    pub invocation_id: Uuid,
    /// 稳定方法标识。
    pub method_id: String,
    /// 位置参数（JSON 值数组，SDK 按形参类型转换）。
    pub arguments: Vec<serde_json::Value>,
    /// 本次调用超时（毫秒）；None 用 Runtime 默认。
    pub timeout_ms: Option<u64>,
}

/// 调用结果状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvocationStatus {
    /// 方法正常返回（含返回 null / void）。
    Success,
    /// 方法抛出异常（不吞异常：类型 + 消息 + 堆栈全量回传）。
    Failed,
    /// 超过超时上限（协作式中断已发出；线程可能仍在运行）。
    Timeout,
    /// 收到取消请求并触发中断（不保证方法已停止）。
    Cancelled,
    /// Runtime 连接中断前未取得结果。
    RuntimeDisconnected,
}

/// 异常详情（堆栈按 Runtime 权限配置输出）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExceptionInfo {
    pub exception_type: String,
    pub message: Option<String>,
    /// 堆栈帧（`com.x.Y.method(Source.java:42)`）。
    pub stack_trace: Vec<String>,
    /// cause 链（`Caused by: ...` 汇总行）。
    pub cause_chain: Vec<String>,
}

/// 调用响应。`status` 覆盖成功 / 失败 / 超时 / 取消 / 断连五态；
/// `result` 与 `exception` 互斥（Failed/Timeout 时也可能已有部分结果为 None）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct InvokeResponse {
    pub invocation_id: Uuid,
    pub method_id: String,
    pub status: InvocationStatus,
    /// 返回值 JSON（void / null → Null）。
    pub result: Option<serde_json::Value>,
    /// 返回值超限被截断（原始结果不完整，前端必须提示）。
    pub result_truncated: bool,
    pub exception: Option<ExceptionInfo>,
    /// 方法执行耗时（毫秒，不含网络传输）。
    pub duration_ms: Option<f64>,
    /// 执行开始时刻（RFC3339，Runtime 时钟）。
    pub started_at: Option<String>,
    /// 执行结束时刻。
    pub finished_at: Option<String>,
}

impl Default for InvokeResponse {
    fn default() -> Self {
        InvokeResponse {
            invocation_id: Uuid::nil(),
            method_id: String::new(),
            status: InvocationStatus::Success,
            result: None,
            result_truncated: false,
            exception: None,
            duration_ms: None,
            started_at: None,
            finished_at: None,
        }
    }
}

/// 取消响应（`POST /miyo/debug/invocations/{id}/cancel`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CancelResponse {
    /// 在途调用已收到中断请求；方法不响应中断时仍会跑完（协作式语义）。
    pub delivered: bool,
}

/// Runtime 错误码（错误响应体 `{"error":{"code":..,"message":..}}`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeErrorKind {
    /// token 缺失 / 不匹配。
    NotAuthorized,
    /// 类或方法不在白名单。
    NotRegistered,
    /// 白名单内但被安全策略拦截（黑名单 / 生产禁用等）。
    NotAllowed,
    /// 参数个数 / 类型转换失败。
    BadArguments,
    /// 请求体非法。
    BadRequest,
    /// 触发频率 / 并发限制。
    RateLimited,
    /// 调用 id 不存在（取消时）。
    UnknownInvocation,
    /// Runtime 内部错误。
    Internal,
    /// 其他未知码。
    Other,
}

impl RuntimeErrorKind {
    /// 从错误响应体解析（未知码归 [`RuntimeErrorKind::Other`]）。
    pub fn parse(body: &str) -> Self {
        let parsed: Result<serde_json::Value, _> = serde_json::from_str(body);
        let code = parsed
            .ok()
            .and_then(|v| v.get("error")?.get("code")?.as_str().map(str::to_owned));
        match code.as_deref() {
            Some("not_authorized") => RuntimeErrorKind::NotAuthorized,
            Some("not_registered") => RuntimeErrorKind::NotRegistered,
            Some("not_allowed") => RuntimeErrorKind::NotAllowed,
            Some("bad_arguments") => RuntimeErrorKind::BadArguments,
            Some("bad_request") => RuntimeErrorKind::BadRequest,
            Some("rate_limited") => RuntimeErrorKind::RateLimited,
            Some("unknown_invocation") => RuntimeErrorKind::UnknownInvocation,
            Some("internal") => RuntimeErrorKind::Internal,
            _ => RuntimeErrorKind::Other,
        }
    }
}

/// 稳定方法标识构造：`com.x.OrderService#createOrder(CreateOrderRequest,long)`。
///
/// 参数类型用擦除名（`java.util.List` → `List`，`java.lang.Long` → `Long`，
/// 基本类型 `long` 原样）。两端 SDK 侧同规则生成，客户端只做透传与校验。
pub fn build_method_id(class_name: &str, method_name: &str, param_types: &[String]) -> String {
    format!("{}#{}({})", class_name, method_name, param_types.join(","))
}

/// 校验 method_id 形状（`Class#method(...)`），非法直接拒绝调用。
pub fn validate_method_id(method_id: &str) -> Result<(), miyo_core::AppError> {
    let (class, rest) = method_id.split_once('#').ok_or_else(|| {
        miyo_core::AppError::Validation(format!("method_id 缺少 '#'：{method_id}"))
    })?;
    if class.is_empty() || !class.contains('.') {
        return Err(miyo_core::AppError::Validation(format!(
            "method_id 类名必须是全限定名：{method_id}"
        )));
    }
    let (name, params) = rest.split_once('(').ok_or_else(|| {
        miyo_core::AppError::Validation(format!("method_id 缺少参数表：{method_id}"))
    })?;
    if name.is_empty() {
        return Err(miyo_core::AppError::Validation(format!(
            "method_id 方法名缺失：{method_id}"
        )));
    }
    let params = params.strip_suffix(')').ok_or_else(|| {
        miyo_core::AppError::Validation(format!("method_id 参数表未闭合：{method_id}"))
    })?;
    // 参数表形状检查：悬挂逗号 / 空参段（`() `已在 split 前排除）。
    if params.starts_with(',') || params.ends_with(',') || params.contains(",,") {
        return Err(miyo_core::AppError::Validation(format!(
            "method_id 参数表非法：{method_id}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn method_id_roundtrip() {
        let id = build_method_id(
            "com.example.OrderService",
            "createOrder",
            &["CreateOrderRequest".into(), "long".into()],
        );
        assert_eq!(
            id,
            "com.example.OrderService#createOrder(CreateOrderRequest,long)"
        );
        validate_method_id(&id).unwrap();
    }

    #[test]
    fn method_id_no_args() {
        let id = build_method_id("com.example.Health", "ping", &[]);
        assert_eq!(id, "com.example.Health#ping()");
        validate_method_id(&id).unwrap();
    }

    #[test]
    fn method_id_rejects_malformed() {
        for bad in [
            "createOrder(CreateOrderRequest)",
            "com.example.OrderService-createOrder()",
            "#createOrder()",
            "com.example.OrderService#()",
            "com.example.OrderService#createOrder(",
            "com.example.OrderService#createOrder(long,)",
        ] {
            assert!(validate_method_id(bad).is_err(), "应拒绝：{bad}");
        }
    }

    #[test]
    fn error_kind_parse() {
        let body = r#"{"error":{"code":"not_authorized","message":"token 不匹配"}}"#;
        assert_eq!(
            RuntimeErrorKind::parse(body),
            RuntimeErrorKind::NotAuthorized
        );
        assert_eq!(RuntimeErrorKind::parse("oops"), RuntimeErrorKind::Other);
    }

    #[test]
    fn runtime_info_defaults_match_protocol() {
        let info = RuntimeInfo::default();
        assert_eq!(info.protocol_version, PROTOCOL_VERSION);
        assert_eq!(info.instance_provider, "static");
    }

    #[test]
    fn display_signature_with_generics() {
        let m = JavaMethodInfo {
            method_name: "createOrder".into(),
            return_type: "OrderResult".into(),
            params: vec![
                JavaParamInfo {
                    name: "userId".into(),
                    type_name: "String".into(),
                    generic_type: String::new(),
                    required: true,
                },
                JavaParamInfo {
                    name: "ids".into(),
                    type_name: "List".into(),
                    generic_type: "List<Long>".into(),
                    required: true,
                },
            ],
            ..Default::default()
        };
        assert_eq!(
            m.display_signature(),
            "createOrder(userId: String, ids: List<Long>) -> OrderResult"
        );
    }
}
