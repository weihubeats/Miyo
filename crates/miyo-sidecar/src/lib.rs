//! Miyo Sidecar Runtime Supervisor。
//!
//! 面向**常驻型**插件 sidecar 进程的生命周期管理（短命令型 sidecar——如 Java
//! attach launcher——按一次性进程设计，不经过本模块）。
//!
//! 职责（对应插件系统 2.0 设计）：
//! - 启动 / 停止 / 重启（崩溃恢复 + 防启动循环）；
//! - 握手鉴权：一次性会话 token 经 **stdin 握手帧**下发（不经 argv，`ps`
//!   不可见），hello_ack 必须回显正确 token；
//! - 心跳：10s ping / 30s 无 pong 判死 → 按重启策略处理；
//! - RPC：请求-响应（id 关联）+ 事件推送 + 日志采集（stderr 与 log 帧统一
//!   经回调上送 plugin_logs）；
//! - 资源限制：内存 rlimit / Windows Job Object；进程数上限（总 8 / 单插件 1）；
//! - 退出清理：unix 进程组 kill、Windows Job Object KILL_ON_JOB_CLOSE、
//!   kill_on_drop 兜底——防僵尸、防 Miyo 退出残留。
//!
//! 隔离等级（诚实边界，详见 docs/plugin/SIDECAR.md）：
//! 平台沙箱能力不同——本模块提供的是**进程级隔离与资源限额**，不是安全沙箱；
//! 不可信代码仍应依赖 OS 级机制（Landlock 仅 Linux 且 best-effort）。

pub mod limits;
pub mod protocol;
pub mod supervisor;

pub use protocol::{SidecarFrame, SIDECAR_PROTOCOL_VERSION};
pub use supervisor::{
    NoopSink, RestartPolicy, SidecarSession, SidecarSink, SidecarSpec, SidecarStatus, Supervisor,
    SupervisorTiming,
};
