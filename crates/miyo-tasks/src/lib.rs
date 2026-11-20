//! 统一 TaskManager：长时任务的注册 / 状态机 / 取消 / 进度 / 历史与退出清理。
//!
//! 覆盖（迁移映射见 docs/arch/TASKMANAGER.md）：场景/矩阵/压测/集合运行、
//! HTTP 与 gRPC 请求取消、WS/SSE/gRPC 长连接会话、AI 聊天循环、单实例服务
//! （Mock / Agent）、Java Attach 心跳等。短生命周期的普通 HTTP 请求**不改造**
//! 为复杂任务，仅登记取消令牌（保持既有轻量语义）。
//!
//! 设计要点：
//! - **状态机**：Pending → Running → Cancelling → Cancelled / Completed / Failed；
//!   终态不可再迁移（Cancelled 幂等）；
//! - **RAII**：[`TaskHandle`] Drop 即从注册表摘除并置 Cancelled——消灭
//!   「命令 future 被丢弃 → 注册表条目泄漏」整类问题（原 run_cancels /
//!   request_cancels 的系统性缺口）；
//! - **取消幂等**：[`Supervisor::cancel`] 恒返回 [`CancelOutcome`]，重复取消
//!   返回 AlreadyFinished 而非报错；
//! - **资源键并发守卫**：同 resource_key（如 scenario_id）运行中拒绝重入
//!   （迁移 scenario_running，并修复「无 run_id 不守卫」漏洞）；
//! - **统一取消入口**：5 个取消命令收敛为 1 个（IPC 命令名保留为薄转发）。

pub mod registry;

pub use registry::{
    CancelOutcome, TaskHandle, TaskKind, TaskRecord, TaskState, TaskSupervisor, HISTORY_CAP,
};
