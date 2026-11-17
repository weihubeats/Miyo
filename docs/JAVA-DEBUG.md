# Java 方法实时调试（Java Debug）

Miyo 的 Java 方法调试能力：连接运行中的 Java 应用，发现并调用经过显式授权的
类 / 方法，查看返回值、异常堆栈与耗时，并可将 Java 方法调用纳入集成测试场景
与 MCP（AI）工作流。与 HTTP / WebSocket / SSE 调试相互独立——执行模型是
「Runtime 白名单方法调用」，不是伪装成 HTTP 的请求类型。

## 一、整体架构

```text
┌────────────────────────────┐  HTTP+JSON 调试协议 v1   ┌──────────────────────────────┐
│ Miyo 桌面端（Tauri）        │ ───────────────────────> │ Java Debug Runtime（SDK）      │
│ 前端 JavaDebugView          │  Bearer Token 鉴权        │ 嵌入目标 JVM 进程内             │
│ miyo-tauri java_debug 命令  │                          │ 白名单显式注册 + 反射受控调用     │
│ miyo-java 协议层 + 客户端    │ <─────────────────────── │ 结果 / 异常 / 耗时结构化回传     │
└────────────────────────────┘                          └──────────────────────────────┘
        │
        ├─ miyo-test 场景引擎：Java 步骤（method=JAVA + 执行器回调）与 HTTP 步骤混合编排
        └─ miyo-agent / miyo-mcp：9 个 list_java_* / run_java_method 工具供 AI 调用
```

### Runtime 方案选型

评估了三条路线后选择 **B：独立 Java Debug SDK（嵌入目标项目）**：

| 方案 | 结论 |
|---|---|
| A. Java Agent + Instrumentation | Attach 语义跨平台差异大、生命周期耦合高，MVP 不引入 |
| **B. 独立 SDK（已实现）** | 用户在目标项目里显式注册白名单并 start()，本进程内起 HTTP 端点；可运行、可维护、可扩展 |
| C. 独立 Worker + 受控 ClassLoader | 需要托管用户依赖树，复杂度高；为「执行不可信代码」场景预留（后续阶段） |

关键取舍：**Miyo 主进程不执行任何不可信 Java 代码**——反射调用全部发生在目标
应用自身进程与安全边界内；Miyo 侧只做协议客户端与安全预检（双保险）。

### 通信协议 v1（`crates/miyo-java/src/protocol.rs` + SDK `io.miyo.debug`）

| 端点 | 用途 |
|---|---|
| `GET  /miyo/debug/info` | 握手：协议版本 / 应用名 / Java 版本 / JVM / 白名单规模 |
| `GET  /miyo/debug/classes` | 白名单类清单（类名 / 包名 / Bean 名 / 方法数） |
| `GET  /miyo/debug/methods?class={FQN}` | 类的白名单方法（签名 / 参数 / 返回类型 / 重载标记） |
| `POST /miyo/debug/invoke` | 执行方法（同步返回结果 / 异常 / 耗时） |
| `POST /miyo/debug/invocations/{id}/cancel` | 取消在途调用（协作式中断） |

- 鉴权：`Authorization: Bearer <token>`（两端必配，常数时间比较）。
- 版本协商：`X-Miyo-Protocol-Version: 1`，握手不匹配即拒绝连接。
- 方法标识：`com.example.OrderService#createOrder(CreateOrderRequest,long)`
  ——重载消歧唯一依据，两端同规则生成 / 校验。
- 错误体：`{"error":{"code":"not_authorized|not_registered|not_allowed|bad_arguments|rate_limited|...","message":...}}`。

### 超时与取消语义（必须了解）

Java 方法执行**无法保证强制终止**。超时与取消通过线程中断实现——业务代码不
响应中断时方法仍会执行完（结果丢弃、调用按 timeout 返回）。需要强制终止 /
执行不可信代码的场景应采用独立进程隔离 + 终止 Worker 进程回收（后续阶段能力）。

## 二、模块与文件清单

### Rust 后端（根 workspace）

| 模块 | 职责 |
|---|---|
| `crates/miyo-java/`（新增） | 调试协议类型、Runtime HTTP 客户端、安全策略（黑名单 / 体积 / 超时 / 并发）、axum mock 集成测试、真实 Runtime E2E 测试（env 门控） |
| `crates/miyo-core/src/model.rs` | `JavaDebugProject` / `JavaInvocationRecord` / `ScenarioJavaInvocation` 模型；`HttpMethod::Java` 变体（wire 值 `JAVA`） |
| `crates/miyo-storage/migrations/0030_java_debug.sql` | `java_projects` / `java_invocations` 表（token AES-GCM 加密列） |
| `crates/miyo-storage/src/repository/java_debug.rs` | 项目 CRUD（token 加解密）、调用历史（滚动保留 200 条）、连接状态刷新 |
| `crates/miyo-test/src/runner.rs` | Java 步骤执行分支（`java_round`：渲染参数 → 执行器回调 → 合成响应 → 复用断言 / 提取管线）；`JavaStepRequest/Response` + `ScenarioOptions.java_invoke` 执行器注入 |
| `crates/miyo-test/src/materialize.rs` | `StepMaterialInput.java_invocation` 透传；Java 步骤跳过幂等键 / 全局参数注入 |
| `crates/miyo-agent/src/java_hook.rs` | 场景引擎 Java 执行器单源（tauri 与 agent 控制面共用：加载配置 → 调用 → 历史落库） |
| `crates/miyo-agent/src/registry.rs` | 9 个 MCP 工具元数据 + Schema（`Module::JavaDebug`） |
| `crates/miyo-agent/src/server.rs` | `/agent/java/*` 路由 + 处理器（出站统一脱敏） |
| `crates/miyo-agent/src/client.rs` | ControlClient Java 方法 |
| `crates/miyo-mcp/src/tools.rs` | `call_tool` / `validate_tool_call` 分发分支 |
| `crates/miyo-tauri/src/commands/java_debug.rs` | 12 个 Tauri 命令（项目 CRUD / 探测 / 发现 / 调用 / 历史） |
| `crates/miyo-tauri/src/commands/scenarios.rs` | 场景运行接线 `java_invoke` 执行器 + Java 步骤参数透传 |

### Java SDK（`java/`）

| 文件 | 职责 |
|---|---|
| `java/miyo-java-runtime/`（Maven，零第三方依赖） | 见下 |
| `MiyoDebugRuntime.java` | 入口：builder / register / start / stop（token 必配，空白名单拒绝启动） |
| `RuntimeConfig.java` | 配置：token / 端口 / 绑定地址 / 应用名 / 线程数 / 安全策略 |
| `MethodRegistry.java` | 白名单注册表：methodId 生成、重载标记、黑名单注册期拒绝、Object 方法过滤 |
| `HttpApi.java` | `com.sun.net.httpserver` 路由：鉴权 / 协议版本 / 5 端点 / 限流错误码 |
| `ArgsConverter.java` | JSON → Java 参数：基本类型 / 枚举 / 集合（泛型感知）/ Map / DTO（字段 + setter 注入，深度 12） |
| `ResultSerializer.java` | Java 对象 → JSON：getter 优先 / public 字段兜底、循环引用与深度截断、体积截断 |
| `InvocationManager.java` | 执行：并发信号量 / 分钟限流 / 超时（协作式中断）/ 异常全量结构化 |
| `SecurityPolicy.java` | 黑名单 + 上限（与 Rust 侧同规则双保险） |
| `InstanceProvider.java` | 实例解析接口（Spring Bean 容器钩子） |
| `Json.java` | 极简 JSON 解析 / 序列化 |
| `src/test/java/io/miyo/debug/SelfTest.java` | 纯 JDK 自测（76 断言：协议 / 转换 / 序列化 / 执行 / HTTP E2E / 鉴权） |
| `java/examples/demo-app/` | 演示应用（订单 + 支付 + 静态工具类）与 `run_demo.sh` 端到端联调脚本 |

### 前端（`frontend/src/`）

| 文件 | 职责 |
|---|---|
| `domains/java-debug/views/JavaDebugView.vue` | 工作台（顶栏连接管理 + 三栏 SplitPanes + 历史抽屉 + 项目弹窗） |
| `domains/java-debug/components/JavaClassBrowser.vue` | 类浏览器（搜索 / 收藏置顶 / 最近使用） |
| `domains/java-debug/components/JavaMethodPanel.vue` | 方法选择 + 参数编辑（可视化表单 ↔ JSON 双模式）+ 执行 |
| `domains/java-debug/components/JavaResultPanel.vue` | 结果区（返回结果 / 异常堆栈 / 执行信息 / 输入参数 四 Tab） |
| `domains/java-debug/components/JavaHistoryDrawer.vue` | 调用历史（重跑 / 复制参数 / 删除 / 清空） |
| `domains/java-debug/components/JavaProjectDialog.vue` | 项目注册 / 编辑 |
| `domains/java-debug/stores/javaDebug.ts` | 工作台状态（发现 / 参数草稿 / 执行 / 历史 / 收藏 / 最近） |
| `router/index.ts` | `/java-debug` 路由 |
| `domains/workspace/.../DashboardNav.vue` + WorkspaceView | 导航入口（侧边栏 + 顶栏工具组 / 溢出菜单） |
| `types/miyoApi.d.ts` | Java 类型手写镜像 + `ScenarioJavaInvocation` 生成类型 |
| `composables/useMiyoApi.ts` | 12 个 Java IPC 方法 |
| `i18n/zh.ts` / `en.ts` | `java.*` 命名空间 105 键（1:1） |

## 三、前后端接口清单

### Tauri 命令（`plugin:miyo|*`，12 个）

| 命令 | 用途 |
|---|---|
| `list_java_projects` / `save_java_project` / `delete_java_project` | 项目 CRUD |
| `java_test_connection` | 握手探测（刷新最近连接时间 + 信息缓存） |
| `java_list_classes` / `java_list_methods` | 类 / 方法发现（实时白名单） |
| `java_invoke` | 执行方法（预检 → 调用 → 历史落库） |
| `java_cancel_invocation` | 取消在途调用 |
| `list_java_history` / `get_java_invocation` / `delete_java_invocation` / `clear_java_history` | 历史 |

### Agent 控制面路由（`/agent/java/*`，9 个工具的 HTTP 后端）

`/agent/java/projects`、`/agent/java/projects/{id}`、`/agent/java/classes`、
`/agent/java/methods`、`/agent/java/method-detail`、
`/agent/java/invocations/validate`、`/agent/java/invoke`、
`/agent/java/invocations/{id}`、`/agent/java/history`。

## 四、MCP 工具清单（9 个，`Module::JavaDebug`）

| 工具 | 权限等级 | 说明 |
|---|---|---|
| `list_java_projects` | Read | Java 调试项目列表 |
| `get_java_project` | Read | 项目详情（含握手缓存） |
| `list_java_classes` | Read | Runtime 白名单类（可搜索） |
| `list_java_methods` | Read | 类的白名单方法 |
| `get_java_method` | Read | 按 methodId 查询签名详情 |
| `create_java_invocation` | Read | 构建并校验调用计划（不执行；参数个数 / 形状预检） |
| `run_java_method` | **NetworkExecute（默认关闭）** | 真实执行（历史落库 + 出站脱敏） |
| `get_java_execution_result` | Read | 按 invocationId 查询执行记录 |
| `list_java_execution_history` | Read | 项目执行历史 |

高风险门与 `send_endpoint` 同口径：`run_java_method` 属 NetworkExecute，
默认关闭，需用户在设置 → MCP 服务中显式启用；注册完整性测试（registry ↔
tools/list ↔ miyo-mcp 分发 1:1）拦截漏项。

## 五、数据库变更（迁移 `0030_java_debug.sql`）

- `java_projects`：id / project_id（可空关联 Miyo 项目，级联删除）/ name /
  runtime_endpoint / environment_id / **auth_json（token AES-GCM 加密）** /
  timeout_ms / max_result_bytes / max_concurrent / description /
  last_connected_at / last_info_json / created_at / updated_at。
- `java_invocations`：id / project_id（级联删除）/ class_name / method_id /
  method_signature / args_json / status / result_json（截断）/ result_truncated /
  exception_json / duration_ms / started_at / finished_at / created_at。
- 索引：`idx_java_projects_project`、`idx_java_invocations_project(project_id, created_at DESC)`。
- 历史清理：插入时滚动保留每项目最近 200 条（`prune_java_invocations`）。

## 六、安全设计

1. **显式白名单**：只有用户在目标项目里注册过的类 / 方法可调用；空白名单拒绝启动。
2. **危险类黑名单（两端各查一遍）**：`java.lang.Runtime` / `ProcessBuilder` /
   `ProcessHandle` / `System` / `ClassLoader` / `Thread` / 反射与脚本引擎 /
   `sun.misc.Unsafe` / 文件与网络套接字类（完整清单见 `SecurityPolicy`）。
3. **鉴权强制**：token 必配（SDK 拒绝空 token 启动），常数时间比较；
   Miyo 侧 token AES-256-GCM 加密落库，不进日志。
4. **体积限制**：请求体 1 MiB 硬顶、返回值默认 1 MiB（可配），超出截断并标记。
5. **超时上限**：单次调用上限 300s（与全局请求超时口径一致），到点发出中断。
6. **并发 / 频率**：默认并发 4（信号量）、每分钟 60 次（滑动窗口），双端限制。
7. **异常不吞**：业务异常全量结构化回传（类型 / 消息 / 50 帧堆栈 / 10 级 cause 链）。
8. **审计**：每次调用落 `java_invocations`（入参 / 状态 / 耗时 / 异常），滚动保留。
9. **出站脱敏**：agent 控制面返回前统一 `redact_json_text`（凭证形态字段 / URL）。
10. **MCP 高风险门**：`run_java_method` = NetworkExecute 默认关闭，副作用方法
    （删除 / 支付 / 改单）必须用户确认，AI 调用不绕过用户权限。
11. **环境隔离**：Runtime 端点与 token 按项目配置；生产环境建议不启用调试 SDK
    （目标应用侧决策）。

## 七、集成测试编排（Java 步骤）

`ScenarioStep` 三态互斥扩展（`endpoint_id` / `request` / `java_invocation`）：

```json
{
  "name": "创建订单",
  "java_invocation": {
    "java_project_id": "<uuid>",
    "class_name": "com.example.OrderService",
    "method_id": "com.example.OrderService#createOrder(CreateOrderRequest,long)",
    "arguments": [{"sku": "A"}, "{{userId}}"]
  },
  "extract": [{"name": "orderNo", "from": "body", "path": "$.orderNo"}],
  "assertions": [{"type": "jsonpath", "path": "$.status", "op": "eq", "expected": "CREATED"}]
}
```

- 前序 HTTP / Java 步骤的提取值共享同一变量上下文，`{{var}}` 在参数 JSON 内
  渲染（类型层还原数字）。
- Java 步骤返回值 JSON 作为「响应体」进入同一套断言 / 提取管线：成功映射 200、
  业务异常映射 500、链路失败（Runtime 不可达）报 request_error。
- 条件 / 循环 / 等待 / 重试 / 并行 / 失败即停对 Java 步骤全部生效；Blocked 判定
  与变量丢失语义与 HTTP 步骤一致（引擎级测试覆盖：成功链路 / 异常 / 未接执行器 /
  失败步阻塞后续）。

## 八、部署与使用

### 1. 目标 Java 应用（Runtime 侧）

```bash
cd java/miyo-java-runtime && mvn package
# jar 或源码包加入目标项目 classpath，应用启动时：
```

```java
MiyoDebugRuntime runtime = MiyoDebugRuntime.create(
        RuntimeConfig.builder(System.getenv("MIYO_DEBUG_TOKEN"))
                .appName("order-service").port(9918))
        .register("orderService", OrderService.class,
                () -> applicationContext.getBean(OrderService.class)) // Spring Bean
        .register(null, MathUtil.class, InstanceProvider.staticInstance())
        .start();
```

编译期建议加 `-parameters`（保留形参名，缺省回退 arg0..n）。

### 2. 验证联调

```bash
java/examples/demo-app/run_demo.sh        # 端到端：编译 + 启动 + 全协议走查
# 可选：真实 Rust 客户端 ↔ Java SDK 对接（env 门控）
MIYO_E2E_RUNTIME=http://127.0.0.1:9918 MIYO_E2E_TOKEN=<token> \
  cargo test -p miyo-java --test java_runtime_e2e_test
```

### 3. Miyo 桌面端

仪表板左侧导航「Java 方法调试」（或工作台顶栏工具组）→ 注册项目
（Runtime 地址 + token）→ 连接 → 浏览白名单类 → 选择方法（表单 / JSON 双模式
填参）→ 执行 → 查看 结果 / 异常 / 耗时 / 历史 → 可存历史重放。

### 4. AI（MCP）

在设置 → MCP 服务启用工具（`run_java_method` 需显式开启 NetworkExecute）。
AI 流程：`list_java_projects → list_java_classes → list_java_methods →
create_java_invocation（校验）→ run_java_method → get_java_execution_result`。

## 九、自动化测试结果

| 套件 | 结果 |
|---|---|
| `cargo test --workspace`（根 workspace 全量） | 全部通过（含新增：miyo-java 25 + mock 6 + E2E 2、storage java_debug 3、core 场景模型、miy-test Java 步骤引擎级 4、agent/mcp 注册完整性） |
| `cargo test`（miyo-tauri，独立 workspace） | 44 通过（含 Java 步骤入参反序列化 2） |
| `cargo clippy --workspace --all-targets` | 0 警告 |
| `cargo fmt --all` | 已格式化 |
| `cargo test --features ts`（bindings 导出） | `ScenarioJavaInvocation.ts` 生成、`ScenarioStep.ts` 更新（CI diff 校验所需文件已随提交） |
| Java SDK `SelfTest`（纯 JDK） | 76 断言全部通过 |
| `java/examples/demo-app/run_demo.sh` | 端到端全协议通过（info / classes / methods / invoke / 异常 / 404 / 403 / 401 / 取消） |
| `cargo test -p miyo-java --test java_runtime_e2e_test`（env 门控，真实 Java Runtime） | 2 通过（握手 / 发现 / DTO 注入调用 / 金额校验 / 异常堆栈 / 未注册拒绝 / 取消链路） |
| 前端 `vitest run` | 139 文件 1122 测试全绿（含 javaDebug 12） |
| 前端 `eslint .` / `vue-tsc --noEmit` / `vite build` | 干净 / 通过 / 成功 |

## 十、当前未实现（预留扩展）

- **方法链式执行**：多 Java 方法串联编排 UI（场景引擎已具备链路变量能力，
  缺独立工作台向导）。
- **JSONPath 断言之外的 Java 专属断言**（返回值字段匹配模板等）——现有
  body / jsonpath / response_time_ms 断言已覆盖主要场景。
- **Spring Bean 自动发现**：SDK 已支持经 `InstanceProvider` 接入 Bean 容器，
  自动扫描 `ApplicationContext` 注册白名单（第二阶段）。
- **参数模板 / 收藏的持久化**（当前收藏走 localStorage、最近使用走会话内存）。
- **多 Runtime 实例管理与实例选择**（当前一个 Java 项目对应一个 Runtime 端点）。
- **进程级强制终止**（独立 Worker 进程隔离 + 杀进程回收，针对不可信代码）。
- **CLI 支持 Java 步骤**（导出格式暂不含 Java 步骤，桌面端已支持）。
- **MCP `create_java_invocation` 落库为可复用模板**（当前为即时校验计划）。
