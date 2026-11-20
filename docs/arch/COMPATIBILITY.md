# 兼容性报告

## IPC 面（233 命令）

- **命令名**：233 个全部保留（新增 plugin_sidecar_* / plugin_update_from_path /
  plugin_rollback / plugin_market_index / plugin_versions / plugin_session_create /
  plugin_read_asset / java_component_* / plugin_inspect_from_url /
  plugin_install_from_cached 共 16 个——只增不删）；
- **入参/出参 serde 产物**：snake_case 契约零变化（ts-rs 绑定 diff CI 守卫）；
- **事件名**：全部保留（miyo:load-progress / scenario-progress / matrix-progress /
  ai-chat / ws-event / sse-event / grpc-event / agent-event）；test-progress 死通道已删除
  （原前端零监听——迁移时发现）。

## 错误码

- CommandErrorCode 35 码不删不改（serde 产物有一致性测试钉扎）；
- AppError::Database/Http 载荷类型变更（sqlx::Error→String、reqwest::Error→String）：
  IPC 面零影响（CommandError::From 映射产出的 code 与 message 格式不变）。

## 数据库迁移

- 0032 java_attach / 0033 plugin_security / 0034 db_indexes：全部追加式，
  不修改既有迁移文件（AGENTS.md 红线）；降级守卫测试（migration_downgrade_guard_test）
  覆盖 0033/0034。

## 前端

- 零 breaking change：所有 IPC 调用（useMiyoApi / 直接 invoke）签名与返回类型不变；
- TypeScript 类型：PluginManifest 新增 files/sidecar/platforms 可选字段（旧插件零改动可装）；
  PluginView 新增 files_verified/last_verified_at（旧前端字段缺失可容忍）；
  PackageInspectionView 新增 trust/files_verified；
- i18n：+64 键 zh/en 1:1（java2.* 53 + plugins 权限/市场/组件 11）。

## Rust API（crate 间）

- **AppError::Database/Http 载荷变更**：match 这两个变体的外部代码需适配（本轮已适配
  全仓使用点；下游如果直接构造 `AppError::Database(sqlx::Error)` 需改为 `AppError::Database(detail.to_string())`）；
- **PluginSessions**：新增 plugin_session_create 等 4 命令——std 族能力需要会话凭证；
- **TaskSupervisor**：新 pub API（start/cancel/status/history/shutdown_all）；
- **TaskHandle**：RAII Drop——正常完成的调用方必须调 mark_completed 否则 Drop 置 Cancelled。

## 已知限制（诚实清单）

1. macOS 无 Sidecar 内存硬限（Darwin 不支持 RLIMIT_AS——实测 EINVAL）；
2. Windows attach / sidecar 进程管理未经真机实测（代码就绪 + CI 编译验证）；
3. 命令层 36 文件全量瘦身：样板 4 个先行（plugins 安装管线 / ai prompt /
   scenarios 矩阵统计 / agent 端口切换），其余记入路线图；
4. miyo-smoke ↔ openapi::smoke.rs 命名混淆：路线图（改名 miyo-e2e）；
5. seed.rs 拆分 deferred：多行 use 块跨行导致机械拆分不安全（dev-only，无生产影响）；
6. server.rs（miyo-agent 6692 行）拆分 deferred：HTTP API 层自洽、拆分收益是可读性
   而非正确性——列路线图；
7. miyo-http→storage 解耦 / storage→plugin 解挂：deferred（需 trait 注入或服务层
   crate，牵动过大——见 ARCHITECTURE.md 迁移阻力表）。
