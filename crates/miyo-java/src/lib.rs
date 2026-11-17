//! # miyo-java — Java 方法实时调试（Runtime 通信层）
//!
//! Miyo 与目标 Java 应用之间的「调试协议 + HTTP 客户端 + 安全策略」三件套。
//!
//! ## 架构定位
//!
//! ```text
//! ┌──────────────────────────┐   HTTP + JSON 调试协议 v1   ┌────────────────────────────┐
//! │ Miyo（桌面端 / Agent）     │ ──────────────────────────> │ Java Debug Runtime（SDK）   │
//! │ miyo-java::JavaRuntimeClient│  Bearer Token 鉴权        │ 嵌入目标 JVM，显式注册白名单   │
//! └──────────────────────────┘                              └────────────────────────────┘
//! ```
//!
//! Runtime 采用「独立 Java Debug SDK」方案（`java/miyo-java-runtime/`）：
//! 用户在目标项目中显式注册允许调试的类（白名单），SDK 在本进程内起一个
//! 仅监听配置地址的 HTTP 端点。Miyo 不做 JVM Attach、不在主进程执行任何
//! 不可信 Java 代码；反射调用全部发生在目标应用自身进程与安全边界内。
//!
//! ## 协议 v1 端点
//!
//! | 方法   | 路径                                        | 用途                     |
//! |------|-------------------------------------------|------------------------|
//! | GET  | `/miyo/debug/info`                          | 运行时信息（版本/环境/JVM）  |
//! | GET  | `/miyo/debug/classes`                       | 已注册白名单类清单          |
//! | GET  | `/miyo/debug/methods?class={FQN}`           | 某类的可调试方法清单        |
//! | POST | `/miyo/debug/invoke`                        | 执行方法（同步返回结果）     |
//! | POST | `/miyo/debug/invocations/{id}/cancel`       | 请求取消（协作式中断）      |
//!
//! 鉴权：`Authorization: Bearer {token}`（两端必配，token 明文不出 Miyo 进程）。
//! 版本协商：请求带 `X-Miyo-Protocol-Version`，Runtime 返回的 `info.protocol_version`
//! 不匹配时客户端拒绝工作（协议演进护栏）。
//!
//! ## 稳定方法标识（method_id）
//!
//! 重载方法只凭名称无法定位，统一用
//! `com.example.OrderService#createOrder(CreateOrderRequest,long)`
//! （类 FQN + 方法名 + 擦除参数类型表）。两端共享解析/构造逻辑见 [`protocol`]。
//!
//! ## 安全边界（与 SDK 端双保险）
//!
//! - 显式白名单：只有用户在目标项目里注册过的类/方法可调用；
//! - 危险类黑名单两端各查一遍（[`policy::BLACKLISTED_CLASSES`]）；
//! - 参数 / 返回值体积、超时上限、并发与频率限制由 [`policy::SecurityPolicy`] 收口；
//! - Miyo 侧 token 只存数据库加密列，不进日志。
//!
//! 超时语义：线程中断是**协作式**的——业务代码不响应中断时方法仍会跑完，
//! 客户端按 `timeout` 状态返回但 Runtime 侧任务继续（SDK 文档明确该语义；
//! 强制终止需进程级隔离，属后续阶段能力）。

pub mod client;
pub mod discover;
pub mod policy;
pub mod protocol;

pub use client::JavaRuntimeClient;
pub use discover::{discover_local, JavaDiscoveredRuntime};
pub use policy::{SecurityPolicy, BLACKLISTED_CLASSES};
pub use protocol::{
    ExceptionInfo, InvocationStatus, InvokeRequest, InvokeResponse, JavaClassInfo, JavaMethodInfo,
    JavaParamInfo, RuntimeInfo, PROTOCOL_VERSION,
};
