# Miyo Rust 后端架构审计（2026-11-20）

> 三份并行代码级审计的综合（全部结论有 file:line 证据）。本文件是重构的起点事实基线；
> 目标架构见 [ARCHITECTURE.md](ARCHITECTURE.md)，逐项修复见 CHANGE-LOG.md。

## 一、依赖图与分层

依赖图**无环**（DAG，全部单向指向 miyo-core），但存在分层违规边：

| 违规 | 证据 | 影响 |
|---|---|---|
| **miyo-core（领域层）依赖 sqlx + reqwest** | core/Cargo.toml:22-23（`AppError::Database(#[from] sqlx::Error)` error.rs:7、`Http(#[from] reqwest::Error)` :19） | 22 个下游 crate 全部传递编译 sqlx/reqwest（纯函数的 miyo-sign / miyo-codegen 等） |
| **miyo-storage → miyo-plugin 反向依赖** | storage/Cargo.toml:6（plugins 五张表需 PluginManifest 解析） | 持久层依赖插件系统 |
| **miyo-http → miyo-storage** | http/Cargo.toml:8（读全局参数/OAuth token） | 应用层与持久层强耦合 |
| **oauth 循环自证** | oauth/client.rs:42-45 注释自认「miyo-http 依赖本 crate，为避环自建等价单例」 | 一份重复的 HTTP client（见三） |

tauri 依赖合规：仅 miyo-tauri 与 frontend/src-tauri 依赖 tauri，无领域 crate 违规。

版本漂移点：miyo-agent/ai/mcp 用本地 version（未继承 workspace）；miyo-tauri 与 frontend/src-tauri 独立 workspace 全套手写 sqlx/reqwest/ts-rs 版本。

## 二、AppState：上帝对象

**19 字段、8 个互不相关域、5 种锁风格**（state.rs:75-123）：

| 域 | 字段 |
|---|---|
| 配置 | db、active、global_settings_cache、settings_gen |
| 请求级取消 | request_cancels |
| 长任务取消 | run_cancels、scenario_running |
| 实时连接 | ws、sse、grpc_streams |
| 服务进程 | mock、agent、sidecar、java_attach |
| AI | ai_chats |
| 响应缓存 | response_bodies |
| 插件 | plugin_sessions、market_downloads、script_sandbox |

域之间零交叉引用；锁风格混用（tokio RwLock / std Mutex / std RwLock / AtomicU64 / OnceLock），「持有期间不 await」的约束只存在于注释。

## 三、长任务/取消体系：11 类各自为政

| 类别 | ID 来源 | 状态机 | 历史 | 进度事件 | 取消语义 | 退出清理 |
|---|---|---|---|---|---|---|
| request_cancels | 前端 uuid | 无 | 落库 | 无 | remove 型（二次 false） | 尾部 remove（无 Drop guard） |
| run_cancels | 前端 uuid | 无 | 场景/用例落库 | 4 种事件 | remove 型 | 尾部 remove |
| scenario_running | scenario_id | 键存在=运行中 | 有 | scenario/matrix | remove 型 | 尾部 remove |
| ws | 前端/后端 uuid | 事件态 | 无 | miyo:ws-event | remove | abort+stop |
| sse | 前端/后端 uuid | 事件态 | 无 | miyo:sse-event | remove | abort+自摘除 |
| grpc_streams | **后端 uuid** | 事件态 | unary 落库 | miyo:grpc-event | remove | close+终态自摘除 |
| ai_chats | session_id | LoopStop（终态） | 消息落库 | miyo:ai-chat | get+clone 型 | **approvals 泄漏** |
| mock / agent | 固定单实例 | 错误码/bool | 内存 100 条 / 无 | agent 有 | 幂等 stop | agent 有 / **mock 无** |
| **sidecar（基准）** | plugin_id | SidecarStatus + RestartPolicy | plugin_logs | Sink 抽象 | start 幂等/stop 报错 | 进程组 kill+guard+心跳 abort |
| java_attach | pid (i64) | Snapshot | java_invocations | 无（轮询） | 协作式转发 | 心跳+清扫+退出清理 |

**系统性缺口**：无 Drop guard（future 被 drop → 注册表条目泄漏到重启）；退出清理只覆盖 agent/java_attach/sidecar 三类；取消命令三份逐字重复（load_test.rs:209-238 与 scenarios.rs:1373-1381）；`miyo:test-progress` 事件前端零监听（死通道）；AI approvals 挂起条目无界累积；同 session 重发覆盖旧令牌（旧循环不可取消）；java heartbeat JoinHandle 丢弃。

**基准实现**：miyo-sidecar Supervisor 是全仓唯一完整闭环（状态快照/心跳判死/指数退避/重启上限/RAII guard/Sink 抽象/shutdown_all）——TaskManager 的设计模板。

## 四、命令层即业务层

233 个 `#[tauri::command]` 全量注册于一个 `generate_handler!`。业务内联实例：
- `run_scenario_matrix`（scenarios.rs:480-865，385 行：行解析+并行引擎+统计+落库）
- `execute_request`（request.rs:96-314，218 行：含脚本头 reconcile 算法 355-384 与历史脱敏 429-474，均带独立单测——纯业务逻辑长在命令文件）
- `update_impl`（plugins.rs:575-745，170 行文件系统事务）
- `ai_chat_send`（ai.rs:268-473，205 行）+ `build_system_prompt`（638-733，95 行双语文案）
- agent.rs `set_mcp_port`（336-378：停服→写设置→重启→失败回滚——配置事务逻辑）

对照：commands/sidecar.rs 自述「只做编排与门禁」并把生命周期下沉独立 crate——正确的样板。

## 五、错误体系

主干单源且有编译期钉扎：AppError（23 变体，miyo-core/error.rs:5-90）→ CommandError/CommandErrorCode（35 码，tauri/error.rs:25-76，From 映射表 166-197）→ ts-rs union（前端）。

欠账：
- **OAuth2Error 旁路**（commands/oauth.rs:42-49 直连 CommandError，绕过 AppError）
- **AiError / AgentClientError 无 From**（命令层手拼错误码）
- **9 个 crate 冗余 thiserror 依赖**（源码零引用）
- 载荷型变体 9 个纯 String（语义无结构化）
- 前端 35 码中仅 2 码（CANCELLED/DECRYPT）被真正分支消费

## 六、网络连接管理：8 个构建点、7 个旁路

| Client | 位置 | 代理/TLS 设置生效？ |
|---|---|---|
| miyo-http 全局池（4 客户端组合） | http/client.rs:675-770 | ✅ 用户设置唯一终点 |
| oauth token_client | oauth/client.rs:47-54 | ❌（自证复制品） |
| miyo-ai | ai/client.rs:30-33 | ❌（隐式读系统环境代理） |
| miyo-java ×2 | java/client.rs:53-56、discover.rs:131-137 | ❌ 同上 |
| agent-control | agent/client.rs:45-52 | ❌ 同上 |
| SSE（tauri ws.rs） | ws.rs:287-292 | ❌ 显式 no_proxy |
| 插件下载 | plugins.rs:1168-1175 | ❌ 同上 |

三种代理语义并存（用户设置生效 / 隐式系统变量 / 显式直连）——**用户在设置页配置的代理/TLS 对 AI、OAuth 刷新、插件下载不生效**，是真实用户可见问题。

## 七、职责重复裁决

| 候选对 | 结论 |
|---|---|
| miyo-mock vs miyo-http | 名字撞车，功能零重复（合理分工） |
| miyo-smoke vs miyo-test vs miyo-backup | 职责清晰（冒烟/运行器/备份纯函数）；miyo-smoke 与 openapi::smoke.rs 命名混淆 |
| miyo-cli vs miyo-mcp | 合理分工（离线 CI 入口 vs AI 工具入口），工具清单已单源 |
| miyo-codegen vs miyo-openapi | 输入/输出均不同，无重复 |
| miyo-sync vs miyo-backup | 正确复用（BackupFile 单源） |

## 八、代码质量热点

- **巨型文件**：miyo-agent/server.rs 6692 行（auth/scenario/java/env/testcase/router 全在一个文件）、miyo-mcp/tools.rs 3501 行（call_tool 760 行 match + 46 处 expect）、miyo-storage/seed.rs 3105 行（单函数 2667 行）
- **锁跨 await**：agent.rs:56-74 ensure_started（写锁跨 DB/文件/bind）、java-attach discover/probe/attach（持锁做 launcher 子进程调用）
- **锁 poisoning**：plugins.rs:1232/1313 std Mutex `.lock().unwrap()`（市场功能 panic 面）；正面案例：state.rs:250/279 全库已系统性 `into_inner()` 容错
- **大 clone**：request.rs:148/332 每请求两次全量 VariableMap clone 进脚本沙箱；client.rs:220 body String→Vec 全量复制；state.rs:192 20MB 正文 clone
- **错误吞噬**：ai.rs:394/414 消息落库失败零日志（真吞噬）；settings.rs:100/217 代理/TLS 恢复失败静默
- **spawn 泄漏**：AI 循环无 abort 路径；12 处 spawn 仅 ws.rs 2 处持句柄

## 九、数据库

- **热路径治理良好**：历史落库后台化 + 20 次节流裁剪、seq 脏检查、settings 快照+代数失效、迁移前快照 + integrity_check、备份单事务 200 行批量
- **欠账**：AI 消息 N+1 逐条 INSERT（ai.rs:365-395）且无事务、`list_ai_messages` 无分页（长会话全量）、5 张表无清理（test_runs/ai_sessions/ai_messages/plugin_permission_audit/scenario_drafts——drafts 还每次保存新行）、delete_scenario 两步删无事务、plugin_permission_audit 无索引、4 处 N+1 候选
- **事务使用**：全仓 5 处显式事务（folders/ai_sessions/projects/backup/sync）；backup.rs:127 注释称 BEGIN IMMEDIATE 实为 DEFERRED

## 十、修复优先级

见计划 Phase 1（低风险质量包，审计 Top-10）→ Phase 2（错误收敛）→ Phase 3（miyo-tasks）→ Phase 4（分层落地）→ Phase 5（大文件拆分）→ Phase 6（索引迁移）→ Phase 7（验收）。
