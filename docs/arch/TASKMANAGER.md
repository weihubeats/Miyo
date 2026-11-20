# 统一 TaskManager 设计（miyo-tasks）

> 实现：crates/miyo-tasks/src/registry.rs（单文件，无子模块——接口面窄）。
> 状态机测试：crates/miyo-tasks/tests/state_machine.rs（9 用例）。

## 状态机（六态）

```
Pending → Running → Cancelling → Cancelled
                  → Completed
                  → Failed
```

终态不可逆。`cancel()` 幂等：首次 Cancelled，重复 AlreadyFinished，未注册 NotFound。

## API 面

| 方法 | 签名 | 说明 |
|---|---|---|
| start | (&Arc<Self>, id, kind, resource_key?) → Result<Arc<TaskHandle>> | 注册+Running；同 id 替换旧任务（旧令牌触发）；资源键守卫 |
| cancel | (&self, id) → CancelOutcome | 幂等取消：令牌触发 + 自定义钩子 + 终态化进历史 |
| status | (&self, id) → Option<TaskRecord> | 单任务快照 |
| running | (&self) → Vec<TaskRecord> | 全部运行中快照 |
| history | (&self, kind) → Vec<TaskRecord> | 每 kind 历史环形（100 条） |
| shutdown_all | (&self) | 全量取消（退出清理统一入口） |
| set_on_cancel | (&self, id, Box<dyn FnOnce()>) | 自定义清理钩子（WS abort / gRPC abort） |
| finish | (&self, id, state, error) | 终态化（自然完成/失败/取消路径共用） |

## TaskHandle（RAII）

- Drop：非终态 → Cancelled 终态化（消灭「future 被 drop → 注册表泄漏」）；
- `mark_completed()` / `mark_failed(err)`：显式终态（Cell 标记防 Drop 重复）；
- `cancellation_token()`：本任务令牌快照（duplicate-id 替换后仍指正确令牌）。

## 资源键并发守卫

`resource_key`（如 `scenario:{id}`）：同键运行中重入返回 Validation 错误。
修复原 scenario_running 两处缺口：无 run_id 不守卫 + 键被覆盖。

## 迁移映射（已完成）

| 原注册表 | TaskKind | 迁移后命令 |
|---|---|---|
| run_cancels（压测/集合/单跑） | LoadTest / TestCollection / TestEndpoint | cancel_run 薄转发 |
| scenario_running（单飞守卫） | ScenarioRun / ScenarioMatrix | 资源键守卫 |
| request_cancels（HTTP/gRPC unary） | HttpRequest / GrpcUnary | cancel_request 薄转发 |
| grpc_streams（服务端流） | GrpcStream | grpc_stream_close 薄转发 |
| ai_chats.cancels | AiChat | ai_chat_cancel |
| （新增覆盖） | WsSession / SseSession | ws_disconnect / sse_disconnect |
| （新增覆盖） | MockService / AgentService | 退出清理 |

## AppState 字段变化

删除：request_cancels / run_cancels / scenario_running / grpc_streams / sse（5 字段）
新增：tasks（1 字段）——净减 4 字段；上帝对象瘦身的 TaskManager 部分完成。

## 退出清理覆盖

原：agent / java_attach / sidecar（3/11）
新：+ WS / SSE / gRPC 流 / AI 循环 / 压测 / 集合 / 场景 / 矩阵 / Mock / Agent（tasks.shutdown_all 一并取消全部在途任务令牌 + 自定义钩子）

## 已知边界

- WS 客户端（WsSession.client）仍在 state.ws——ws_send 数据面需要 client 克隆；
  纯生命周期归 tasks。合并需 TaskManager 支持任意 payload 存储（路线图）；
- Mock / Agent 服务的 start/stop 仍为直接管理（未注册进 tasks）——它们是
  固定单实例服务而非多实例任务，Running 可见性由各自 status 命令提供；
- 进度事件名保持各域现有（miyo:load-progress / scenario-progress / matrix-progress /
  ai-chat / ws-event / sse-event / grpc-event）——统一 task-progress 通道为路线图。
