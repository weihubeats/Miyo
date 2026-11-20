# 统一错误体系对照

> 主干：AppError（miyo-core，23 变体）→ CommandError（miyo-tauri，35 码）
> → ts-rs union（前端 types/generated/CommandErrorCode.ts）。
> 单源且有编译期钉扎（tauri/error.rs 一致性测试）。

## 用户 8 类模型 ↔ 现有错误码映射

| 用户模型 | 现有 AppError 变体 | IPC 错误码 |
|---|---|---|
| ValidationError | Validation | VALIDATION |
| NetworkError | Connection / Dns / Ssl / Http / NetworkTimeout | CONNECTION / DNS / SSL / HTTP / TIMEOUT |
| TimeoutError | NetworkTimeout | TIMEOUT |
| PermissionError | Forbidden | FORBIDDEN |
| StorageError | Database / SchemaTooNew / Io | DATABASE / DATABASE_SCHEMA_TOO_NEW / IO |
| PluginError | （无专用变体） | PLUGIN_NOT_INSTALLED / PLUGIN_DISABLED / PLUGIN_INCOMPATIBLE |
| RuntimeError | Test / ScriptError / WebSocket / Grpc / Mock / PortBind / OpenApi / OAuth2 / Sync / Json / Decryption | TEST / SCRIPT / WEBSOCKET / GRPC / MOCK / PORT_BIND / OPENAPI / OAUTH2 / SYNC / JSON / DECRYPT |
| InternalError | Internal（tauri 层手写 33 处） | INTERNAL |

35 码不删不改——新增码只增；细分变体是资产（前端可按需分支），8 类是
「按需归组」而非「强制扁平化」。

## 本轮收敛

| 收敛项 | 原状 | 新状 |
|---|---|---|
| OAuth2Error 旁路 | tauri 命令层 From<OAuth2Error> for CommandError 直拼 OAUTH2 码 | impl From<OAuth2Error> for AppError（oauth crate 本体）→ 经 From<AppError> 统一产生 |
| AiError 无 From | 命令层手拼 AI_PROVIDER / AI_NOT_CONFIGURED | impl From<AiError> for AppError（miyo-ai 补 core 依赖） |
| AgentClientError 无 From | attach 命令层逐分支手拼 | impl From<AgentClientError> for AppError（Connection→Connection / Auth→Forbidden / Timeout→NetworkTimeout / Agent→Test） |
| AppError::Database(sqlx::Error) | #[from]（Domain 层依赖 sqlx） | Database(String) 明细（storage 边界 db_err 显式映射） |
| AppError::Http(reqwest::Error) | #[from]（同上） | Http(String) 明细（http 边界 from_reqwest 显式分类映射） |
| 9 crate 冗余 thiserror | Cargo.toml 声明但源码零引用 | 删除 |

## 敏感信息

- `sanitize_message_urls()`：URL query 凭据脱敏（NetworkTimeout/Ssl/Dns/Connection/Validation 载荷统一处理）；
- IPC message 由 `user_message()` 产生中文提示；内部 detail（如 sqlx 分类）保留在
  AppError Debug 载荷（日志可见），前端 toast 只见 user_message。
