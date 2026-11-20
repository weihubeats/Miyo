# 修改清单（架构重构全量）

> 提交序列按 Phase 排列；每项含受影响文件与测试。兼容性详见 [COMPATIBILITY.md](COMPATIBILITY.md)。

## Phase 0 — 审计

| 文件 | 变更 |
|---|---|
| docs/arch/AUDIT.md | 新建：三审计综合（依赖/状态/命令层/错误/网络/质量/DB） |
| docs/arch/ARCHITECTURE.md | 新建：目标分层图 + 迁移阻力表 |

## Phase 1 — 低风险质量修复包（审计 Top-10）

| 文件 | 变更 | 对应审计项 |
|---|---|---|
| crates/miyo-storage/src/repository/ai_sessions.rs | +append_ai_messages_bulk（单事务批量）+list_ai_messages_after（seq 分页） | AI 消息 N+1 |
| crates/miyo-tauri/src/commands/ai.rs | 循环改批量落库 + let _ =→warn | AI 吞错 |
| crates/miyo-tauri/src/commands/plugins.rs | market_downloads 锁 poisoning ×2 修复 | 锁审计 |
| crates/miyo-tauri/src/commands/agent.rs | ensure_started 锁外准备 + CAS | 锁跨 await |
| crates/miyo-java-attach/src/manager.rs | discover/probe/attach launcher/OS 扫描移出锁 | 锁跨 await |
| crates/miyo-java-attach/src/jdk.rs | ToolchainRegistry +Clone（锁外克隆模式需要） | 同上 |
| crates/miyo-storage/src/repository/test_runs.rs | +滚动清理 50/项目 | 5 表无清理 |
| crates/miyo-storage/src/repository/scenario_drafts.rs | +滚动清理 10/场景 | 5 表无清理 |
| crates/miyo-storage/src/repository/plugins.rs | +plugin_permission_audit 滚动 500 | 5 表无清理 |
| crates/miyo-tauri/src/commands/scenarios.rs | export_scenario_report N+1 → IN 批量；delete_scenario 两步删包事务 | N+1/事务 |
| crates/miyo-tauri/src/commands/sync.rs | sync 存在性检查 N+1 → list_project_ids 集合 | N+1 |
| crates/miyo-storage/src/repository/projects.rs | +list_project_ids | N+1 |
| crates/miyo-http/src/script_sandbox.rs | ScriptInput.environment → Arc<Mutex<HashMap>>（可写回） | 大 clone |
| crates/miyo-tauri/src/commands/request.rs | 调用点适配 Arc<Mutex> | 大 clone |
| crates/miyo-test/src/assert.rs | 同上 | 大 clone |
| crates/miyo-tauri/src/commands/backup.rs | BEGIN IMMEDIATE 注释修正 | 注释漂移 |
| 6 个 Cargo.toml | 冗余 thiserror 删除 | 依赖清理 |
| 3 个 Cargo.toml | agent/ai/mcp version.workspace 继承 | 版本漂移 |

## Phase 2 — 统一错误体系

| 文件 | 变更 |
|---|---|
| crates/miyo-core/src/error.rs | Database/Http 载荷改 String；from_reqwest/classify/source_chain 迁出 |
| crates/miyo-core/Cargo.toml | sqlx/reqwest 依赖删除 |
| crates/miyo-http/src/network.rs | 新建：from_reqwest/classify/source_chain（自 core 迁入） |
| crates/miyo-storage/src/db.rs | +db_err（sqlx→AppError 唯一入口） |
| crates/miyo-storage/src/repository/*.rs | 176 处裸 ? → map_err(db_err)（cargo JSON 诊断精确定位机械转换） |
| crates/miyo-tauri/src/commands/{backup,sync}.rs | map_err(AppError::Database) → map_err(miyo_storage::db_err) |
| crates/miyo-ai/src/error.rs | +From<AiError> for AppError |
| crates/miyo-ai/Cargo.toml | +miyo-core（分层 Infrastructure→Domain） |
| crates/miyo-java-attach/src/client.rs | +From<AgentClientError> for AppError |
| crates/miyo-java/Cargo.toml + src/client.rs | +miyo-http 依赖；classify 调用迁移 |
| crates/miyo-oauth/src/client.rs | +From<OAuth2Error> for AppError（孤儿规则合规——本 crate 实现） |
| crates/miyo-tauri/src/commands/oauth.rs | 旁路改经 AppError 中转 |

## Phase 3 — 统一 TaskManager

| 文件 | 变更 |
|---|---|
| crates/miyo-tasks/** | 新建 crate：registry.rs（状态机/cancel/守卫/历史）+ tests/state_machine.rs（9 用例） |
| crates/miyo-tauri/src/state.rs | +tasks 字段；-run_cancels/-scenario_running/-request_cancels/-grpc_streams/-sse（5 字段删除） |
| crates/miyo-tauri/src/commands/load_test.rs | test_endpoint/load_test/test_collection 注册改 tasks.start；cancel_run 薄转发 |
| crates/miyo-tauri/src/commands/scenarios.rs | run_scenario/matrix 守卫改资源键；cancel_scenario_run 薄转发 |
| crates/miyo-tauri/src/commands/request.rs | execute_request 注册改 tasks.start；cancel_request 薄转发 |
| crates/miyo-tauri/src/commands/grpc.rs | unary/stream 注册改 tasks.start；grpc_stream_close 薄转发；终态改 tasks.finish |
| crates/miyo-tauri/src/commands/ws.rs | ws/sse 生命周期改 tasks.start + on_cancel 钩子 |
| crates/miyo-tauri/src/commands/ai.rs | ai_chat_send 注册改 tasks.start；cancel/delete 改 tasks.cancel + approvals 前缀回收 |
| frontend/src-tauri/src/main.rs | 退出清理追加 tasks.shutdown_all（全覆盖） |
| crates/miyo-tauri/src/lib.rs | java heartbeat 句柄注册进 tasks |
| crates/miyo-tauri/Cargo.toml | +miyo-tasks |

## Phase 5 — 巨型文件

| 文件 | 变更 |
|---|---|
| crates/miyo-mcp/src/tools.rs | 46 处序列化 expect → unwrap_or(Value::Null)（防 panic 击穿 stdio） |

## Phase 6 — DB 治理

| 文件 | 变更 |
|---|---|
| crates/miyo-storage/migrations/0034_db_indexes.sql | 新增：ai_messages 复合索引 + test_runs 复合索引 |

## Phase 7 — 文档

| 文件 | 变更 |
|---|---|
| docs/arch/TASKMANAGER.md | 新建：TaskManager 设计文档 |
| docs/arch/ERRORS.md | 新建：错误体系对照 + 本轮收敛清单 |
| docs/arch/COMPATIBILITY.md | 新建：兼容性报告（IPC/错误码/DB/前端/Rust API/已知限制） |
| docs/arch/CHANGE-LOG.md | 本文件 |
