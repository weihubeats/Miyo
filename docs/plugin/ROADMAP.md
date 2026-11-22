# 插件系统路线图（Roadmap）

> 本文记录已识别但未立项的演进项；每项含动机与设计要点。落地时移入对应
> 专题文档并同步 ARCHITECTURE.md。

## 1. Java 调试全量 sidecar 化（里程碑级，分阶段进行中）

**动机**：让「原生调试能力」可经市场运行时安装/更新，主程序二进制彻底不含
任何具体调试能力。Rust 无法运行时加载原生代码——sidecar 进程是唯一通道。

**进度**：✅ 第 1-2 步完成（2026-11）——
1. `miyo-java-attach` 新增 `java-debug-sidecar` 二进制：AttachManager 全编排
   （会话表 / AgentClient / 心跳 / launcher spawn / infoFile / 工具链）经
   stdio 协议对外（req/res 原语已够，attach 会话以 pid 参数多路复用）；
   jar/infoFile 目录经环境变量注入或 data_dir 约定推导（MIYO_DATA_DIR 同优先级）；
   EOF → shutdown_all 优雅清理。协议 e2e（tests/sidecar_e2e.rs）。
2. `SidecarSpec.extra_env` 透传（limits env_clear 白名单之上追加）+
   **Supervisor 全链路 e2e**（tests/sidecar_supervisor_e2e.rs）：start →
   握手 → sessions/tools_ready/未知方法错误帧 → stop 优雅终止——「宿主命令
   层经 Supervisor 转发」的核心机制已验证。

**进度补记**：✅ 第 3 步完成（2026-11）——`miyo-java-attach` 新增
`backend.rs::JavaBackend`（Local / Sidecar 双路分发，`AgentOp` 参数化 agent
调用面）；miyo-tauri 的 14 个 attach/agent 命令全部迁移到 backend 调用面。
模式开关 = env `MIYO_JAVA_SIDECAR_BIN`（指向二进制即启用 sidecar 转发，
缺省 = Local 宿主编排，行为与历史版本完全一致）。会话态单源：全部 attach
会话操作走同一 backend。Backend e2e（sidecar_supervisor_e2e.rs 第二用例）
覆盖 resolve → ensure_started（Supervisor 托管）→ sessions / agent_call /
discover 全调用面。

**下一阶段**：插件 manifest 迁移 kind=ui-sidecar（平台二进制入包 + 市场
分发）+ e2e 矩阵切换 + 灰度（sidecar 模式先以 env 开关在开发环境验证）。

**现状差距**（对照 `crates/miyo-sidecar` 与 `crates/miyo-java-attach/src/manager.rs`）：
- Supervisor 会话模型 =「每插件一个常驻进程、单会话、stdin RPC」；Java attach
  需要 **per-pid 多目标会话**（上限 8）、launcher/agent 双 jar、JDK 工具链
  注册表（本机 JDK 发现与版本匹配）、infoFile 落盘交接、25s attach 超时、
  目标 JVM 死亡即清理——协议帧（hello/req/res/evt/log/ping/pong）没有对应原语；
- launcher jar 已是「短命令型 JVM sidecar」（单命令即退，进程隔离动机成立），
  但 **Attach 编排（会话表 / AgentClient 长连 / 心跳清扫）在宿主进程内**；
- `SidecarSpec.executable` 语义是包内固定可执行文件，与「本机任一 JDK 动态
  选择」不匹配。

**设计草案**：
1. sidecar 协议 v2 增帧：`attach.open/close`（目标会话多路复用，帧带
   session_id）、`toolchain.list/select`（宿主侧 JDK 发现下沉）；
2. 新增独立 Rust 二进制 `miyo-java-debug-sidecar`：整体迁移 `AttachManager`
   （会话表 / AgentClient / 心跳 / infoFile / ToolchainRegistry）；宿主侧
   仅保留命令编排 + 门禁转发；
3. 分发：按平台构建产物走 plugin 包 `bin/`（files 校验已支持可执行文件），
   安装即获得原生能力；min_app_version 锁协议版本；
4. 迁移步骤：先双轨（feature `java-sidecar` 并行保留宿主编排）→ e2e 矩阵
   对齐（scripts/test_java_attach_matrix.sh）→ 切默认 → 宿主代码删除。

## 2. 官方插件原生层 feature-gate（已落地，记录边界）

- miyo-tauri feature `java`（默认开）：`--no-default-features` 核心构建裁掉
  AttachManager / 心跳 / 30 条 IPC 命令（stub 返回 `FEATURE_UNAVAILABLE`）；
- **诚实边界**：Rust 无法运行时加载原生代码——市场安装永远不能把 Rust 逻辑
  注入主程序；官方插件原生层随主程序发版（feature 只是构建矩阵维度），
  运行时安装原生能力的唯一途径是第 1 项的 sidecar 化。

## 3. miyo-agent 控制面 java 能力 feature 化

- 现状：MCP 控制面（miyo-agent）共享 mi-java 客户端（v1 runtime 协议），
  全部走插件门禁；AttachManager 注入已是 Option（无注入时 attach 模式步骤
  优雅失败）。经评估**暂不 gate**：registry TOOL_META / miyo-mcp 分发 /
  注册完整性测试三者 1:1 锁定，gate 需同步三处 cfg + 测试矩阵重构，
  而运行成本（AttachManager / 心跳 / IPC 命令面）已由 miyo-tauri feature
  gate 全部消除，此处仅剩协议客户端代码；
- 路线：`miyo-agent` feature `java`（registry TOOL_META java 段 / server
  handler 段 / miyo-mcp call_tool 分支 / 注册一致性测试四区 cfg）。

## 4. 市场演进

- **索引签名**：✅ 已落地（2026-11）——schema v2 增 `signature` 字段，
  `miyo_plugin::market::verify_market_index` 验签（signed-but-foreign 硬失败），
  `scripts/sign_plugin.mjs sign-index` 签名；正式发布时由 CI 以官方种子签名，
  市场页展示信任徽章；
- **远端索引**：`plugin.market.url` 设置项拉取同 schema 远端 JSON
  （https-only + 缓存 + 签名校验；MARKETPLACE.md 已预留 schema）；
- **分类与搜索**：MarketEntryView 已有 category 字段，前端市场页签补
  分类过滤（python/db/redis/kafka 模板上架时需要）。

## 5. SDK 能力扩展

- `registerCommand` / `registerSettings`（SDK.md 已知边界）：插件向宿主
  注册自定义命令面板项与设置页节（权限：新目录项评审后加入 18 项目录）；
- **RPC 请求级取消**已落地（v1 增量 cancel 帧，bridge.ts handleCancel）——
  后续：sidecar request 通道透传 abort（真正中断 sidecar 处理）；
- per-plugin 文件目录 API（`plugin_dir_get` + `ctx.paths.data()`）已落地；
  后续补：配额上限（data 目录体积审计）、cache 清理宿主动作。

## 6. 模板插件规划（市场首批垂直能力）

| 插件 | 形态 | 关键依赖 | 备注 |
|---|---|---|---|
| Python 调试 | ui-sidecar（python 进程，runtime_deps 声明 python） | debugpy 协议或自带 stdio 协议 | 复用 echo-sidecar 脚手架 |
| 数据库调试 | ui-sidecar（连接驱动内置） | database.connect 权限（当前登记恒拒——需评审开放 + 后端命令面） | SQL 网关在 sidecar 内实现，宿主只转发 |
| Redis / Kafka | ui-sidecar | std.network 开放（同上） | 连接配置存 plugin-data |
| gRPC 扩展 | ui（宿主已有 grpc 核心） | 现有 grpc 命令族白名单化暴露 | 核心能力插件化视图 |
| 自定义协议工具 | ui-sidecar | runtime.process.start/stop | echo-sidecar 直接扩展 |
