# Java 方法实时调试 2.0（零源码侵入 + Attach）

Miyo 的 Java 方法调试能力：**自动发现本机 Java 进程 → 用户确认后动态注入调试
Agent → 浏览已加载类 / Spring Bean / 方法 → 受控反射调用**。目标应用
**零源码修改**——不引入 Miyo SDK、不改 pom.xml / build.gradle、不加注解、
不注册 Bean、不重启进程。

同时保留 v1 的「Runtime SDK」模式作为兼容连接方式（旧项目 / 远程调试）。
Java 调试历史、集成测试 Java 步骤、MCP Java 工具、AI 调试工作流全部沿用，
底层执行器按项目 `mode` 路由（`attach` / `runtime`）。

## 一、新旧架构对比

| 维度 | v1（Runtime SDK） | **v2（本地 JVM Attach，默认）** |
|---|---|---|
| 目标应用改动 | 引入 SDK + 显式白名单注册 + start() | **零改动**（正常启动即可） |
| 连接方式 | HTTP + Bearer Token（目标进程内嵌端点） | JVM Attach API → Agent 本地 IPC |
| 进程发现 | 端口段指纹探测 + 注册目录 | OS 进程扫描 + Attach listener 交叉 |
| 类 / 方法范围 | 用户显式注册的白名单 | 已加载类（可搜索 / 过滤 JDK 内部类） |
| Spring Bean | 手动经 InstanceProvider 接入 | 自动探测（策略梯子，分级支持） |
| 远程应用 | 支持（HTTP 可达即可） | 不支持（必须本地 Attach 或走 v1 / 远程 Agent） |
| 安全模型 | 显式注册 + 两端黑名单 | 注入确认 + token 鉴权 + 黑名单 + 可赋值性闸 |

## 二、整体架构

```text
┌────────────────────────┐ spawn 子进程    ┌──────────────────────────┐
│ Miyo（Rust / Tauri）    │ ─────────────> │ miyo-attach-launcher.jar │
│ miyo-java-attach crate │                │ JDK Attach API（短进程）   │
│ 命令层 java_attach.rs  │                │ list / properties / attach│
└──────────┬─────────────┘                └────────────┬─────────────┘
           │  TCP 127.0.0.1:随机端口                    │ VirtualMachine.attach
           │  长度前缀 JSON 协议 v2                     ▼ loadAgent(miyo-java-agent.jar)
           │  一次性 256-bit token 鉴权    ┌──────────────────────────────────┐
           ▼                              │ 目标 JVM（零源码修改）             │
┌────────────────────────────┐           │  miyo-java-agent（agentmain）     │
│ Miyo 前端（attach 工作台）   │ ←──────── │  · IPC 服务（回环 + token）        │
│ 进程列表 / 类 / Bean / 方法  │  Tauri IPC │  · Inspector（已加载类 / 方法）    │
│ 参数编辑 / 结果 / 历史      │            │  · InvocationEngine（受控反射）    │
└────────────────────────────┘            │  · Spring 上下文发现（策略梯子）    │
                                          └──────────────────────────────────┘
```

分层职责：

| 层 | 位置 | 职责 |
|---|---|---|
| Miyo Java Debug Plugin | `frontend/src/domains/java-debug/` | 双模式工作台 UI（attach 默认 / runtime 兼容） |
| JVM Process Discovery | `crates/miyo-java-attach/src/process.rs` | macOS `ps` / Linux `/proc` / Windows PowerShell 进程扫描 + 脱敏 |
| Attach Manager | `crates/miyo-java-attach/src/manager.rs` | 会话注册表 / attach 流程编排 / 重启身份识别 |
| Miyo Java Agent | `java/miyo-java-agent/` | agentmain + Instrumentation + IPC 服务（Java 8 字节码） |
| JVM Inspector | agent `inspect/` | 已加载类 / 方法元信息（重载 / 泛型 / synthetic / bridge） |
| Invocation Engine | agent `invoke/` | 并发闸 / 超时 watchdog / 取消 / 异常结构化 |
| Result Serializer | agent `invoke/ResultSerializer` | 循环引用 / 深度 / 体积截断 / 代理标注 |
| Agent Communication | 协议 v2（本文件 §四） | 握手 / 版本协商 / 请求 id / 错误码 |
| Security Policy | agent `invoke/RiskPolicy` + Miyo 侧预检 | 黑名单 + 可赋值性闸 + 风险分级 |
| Attach Launcher | `java/miyo-attach-launcher/` | list / properties / attach CLI（JDK 8 tools.jar 兼容） |

Agent 与 launcher 均编译为 **Java 8 字节码**（可注入 JDK 8+ 的任意 HotSpot 目标
进程）；Agent 内嵌 **shaded ASM**（重定位到 `io.miyo.agent.shaded`），不污染
目标应用 classpath。Miyo 主进程不执行任何不可信 Java 代码——Attach 工具以
短进程 sidecar（`java -jar launcher.jar`）运行。

## 三、本地 Java 进程自动发现

真实进程扫描（非端口猜测），覆盖规格全部条目：

| 条目 | 实现 |
|---|---|
| PID / 进程名 / 主类 | macOS `ps -axo pid,uid,etime,lstart,command`；Linux `/proc/*/cmdline + exe`；Windows `Get-CimInstance Win32_Process` |
| Java 版本 / 厂商 | `<home>/bin/java -version` 探测（按 home 缓存，不重复起进程） |
| JDK / JRE / 不可 attach | `bin/javac` 存在性 → jdk/jre；launcher `list`（Attach API 视角）交叉 → attachable |
| 启动参数（脱敏） | `redact_command_line`：password / secret / token / api_key 等键值 `***`，URL 内嵌凭证脱敏 |
| 运行时间 | etime（ps）/ CreationDate（Windows） |
| Java 可执行路径 | 命令行首个 `/bin/java` token；java_home 由路径推断 |
| 手动输入 PID | `java_probe_pid` 单进程探测（存在性 + Java 判定） |
| 自动刷新 | 前端 5s 轮询（可开关） |
| 多 JVM 并行发现 | 单次快扫全部进程 + 逐 home 版本探测（缓存） |
| 进程退出检测 | launcher `list` 消失 → attachable=false；会话命令失败 → 明确报错 |
| 重复 PID / 重启识别 | `vm_start_ms`（JVM 启动时刻）绑定——pid 复用时 vm_start 变化 → 重建会话，历史保持连续 |

**兼容性提示（诚实呈现）**：

- 不同用户 uid 的进程 → 「进程属于其他用户，当前用户无权 attach」；
- 非 HotSpot / `-XX:+DisableAttachMechanism` → launcher 错误码
  `attach_not_supported`；
- 无可用 JDK 工具链 → 「未发现可用 JDK（jlink 裁剪 / JRE 不含 jdk.attach）」；
- 容器 / PID namespace：Miyo 与目标必须在同一 PID 空间（容器内运行的应用
  需在容器内启动 Miyo，或挂载宿主 /proc——视环境而定，探测不到即如实展示）。

## 四、Agent 通信协议 v2

**传输**：TCP `127.0.0.1:<随机端口>`，仅回环；4 字节大端长度前缀 + UTF-8 JSON
（单帧上限 16 MiB）。协议版本 `2`（与 `miyo-java-attach::AGENT_PROTOCOL_VERSION`
对齐，握手不匹配即拒绝）。

**鉴权**：Miyo 生成一次性 256-bit token（`rand` 32 字节 hex）→ 经 launcher
loadAgent 参数传入 Agent（内存持有，不落盘）→ 握手首帧 `params.token` 常数
时间比较（`MessageDigest.isEqual`）。连接信息经 infoFile（0600）回传 Miyo
（不含 token）；token 换发 = 重复 attach 时旧 token 立即失效。

**身份绑定**：agent args 携带 `expectedPid`（不匹配拒绝启动）；握手响应回传
`pid` + `vm_start_ms`（Miyo 缓存比对 → 重启识别）。

**请求**：`{"id":"r-N","op":"...","params":{...}}`；
**响应**：`{"id":..,"ok":true,"data":{..}}` 或
`{"id":..,"ok":false,"error":{"code":"...","message":"..."}}`。
invoke 在 Agent 侧 worker 池执行——期间 cancel / status 可并发
（响应按 id 关联，乱序完成不影响匹配）。

| op | 说明 |
|---|---|
| `handshake` | token + 协议版本协商 → JVM 信息 / 上限 / 版本 |
| `status` | 在途 / 成功 / 失败 / 超时 / 取消计数 |
| `classes.list` | search / include / exclude / include_jdk / limit / offset |
| `class.detail` | 父类 / 接口 / loader / 修饰符 |
| `methods.list` | 方法清单（含 `risk` 分级与 `risk_reason`） |
| `beans.contexts` | Spring 状态（策略梯子 + 捕获状态，§六） |
| `beans.list` | Bean 清单（定义信息 + 已实例化单例真实类型） |
| `invoke` | invocation_id / class / method / param_types / args / instance{kind,bean_name,consent} / timeout_ms |
| `cancel` | 协作式中断（delivered / terminated=false） |
| `agent.stop` | 停服务 + 清 infoFile（类不可卸载——诚实语义） |
| `ping` | 心跳 / 链路探活 |

**错误码**：`not_authorized / protocol_mismatch / bad_request / bad_arguments /
not_allowed / not_found / rate_limited / unknown_invocation / consent_required /
unsupported / internal`。

**诚实边界**：v2 传输为回环 TCP（Java 8 无内置 UDS 支持；UDS / Named Pipe
作为后续增强——协议层传输无关，替换不动上层）。

## 五、Attach Launcher（sidecar）

`java/miyo-attach-launcher/target/miyo-attach-launcher.jar`（Java 8 字节码，
零依赖），Miyo 以 `java -jar` 短进程调用，单行 JSON stdout + 退出码：

| 命令 | 用途 |
|---|---|
| `capabilities` | 运行环境自检（bridge_available / tools.jar 位置 / java 版本） |
| `list` | Attach API 视角的可 attach JVM（`VirtualMachine.list()`） |
| `properties --pid N` | 目标 JVM 系统属性（预检版本 / 厂商，不加载 Agent） |
| `attach --pid N --agent PATH --args "k=v,..." --info-file PATH` | attach → loadAgent → 轮询 infoFile → 连接信息 |

- **JDK 8 运行时**：`com.sun.tools.attach` 经 URLClassLoader 反射桥
  （tools.jar 自动定位）；
- **JDK 9+ 运行时**：jdk.attach 模块直连；
- **JDK 工具链选择**（`ToolchainRegistry::select_for_target`）：目标进程
  版本探测 → 「同 major」优先 → 「更新 client」次之 → 最新可用兜底。
  跨版本 attach 的协议返回码偏斜（JDK21→JDK8 误报 AgentLoadException、
  JDK8→JDK17「Non-numeric value found」）由 launcher 的 **infoFile 轮询兜底**
  吸收：loadAgent 报异常仍继续轮询，Agent 实际加载成功即按成功处理。

## 六、Spring Boot 支持（策略梯子，分级支持）

**不能假设动态 Attach 后能通过静态字段拿到 ApplicationContext**——Boot 不保留
静态引用。Agent 用三条互补策略，UI 按「已验证 / 条件 / 暂不支持」分级展示：

| 策略 | 原理 | 级别 |
|---|---|---|
| 捕获插桩（默认启用） | ASM 插桩 `AbstractApplicationContext.publishEvent / getBean` 方法体入口 → `SpringCapture.capture(this)` 静态弱引用收集（attach 后**下一次**应用活动命中——发一次 HTTP 请求 / 定时任务即可） | 条件支持（应用有活动即命中） |
| LiveBeansView 静态表 | 启动时设 `-Dspring.liveBeansView.mbeanDomain=<值>` 的应用，Spring 自行注册上下文到静态集合 | 条件支持（需启动参数） |
| ContextLoader | 经典 Spring MVC（非 Boot）`getCurrentWebApplicationContext()` | 条件支持（应用形态） |

- **不可用时如实展示原因**（`beans.contexts.note`）——绝不伪造 Bean 数据；
- 全部策略纯反射 / 插桩，不改目标应用行为；
- **关键实现细节**：Boot fat-jar 的 Spring 类加载在 `LaunchedURLClassLoader`
  里，Agent 线程的 system classloader 看不到——类探测一律走
  `Instrumentation.getAllLoadedClasses()`（已加载类视角）+ 以
  `AbstractApplicationContext` 的实际加载器为锚反射 Spring 类；
- Bean 清单：默认只列 `ROLE_APPLICATION`；**不为取类型而实例化 Bean**——
  已实例化单例经 `beanFactory().containsSingleton` 安全回读真实类型
  （含 JDK 动态代理 / CGLIB 标注），未实例化的只报定义信息并标 `lazy`
  （调用其方法会触发创建，UI 明示）；
- 插桩安全兜底：任何解析失败返回 null（原字节码放行）；捕获钩子
  try/catch 全包（绝不影响目标应用热路径）。

**实测**（Spring Boot 3.2.5 / JDK 17 / Zulu，`java/examples/spring-boot-demo`）：
attach → `curl /ping` → 上下文捕获命中（155 Bean 定义）→ Bean 清单
（priceService，instantiated=true，proxy=false）→ Bean 方法调用
`quote("SKU-1", 2, 0.1)` → success + 完整结果。

## 七、Java 方法执行引擎（受控反射）

- **static / 实例 / Bean 方法**：实例解析三态——`static`（直调）/ `bean`
  （Spring 上下文按名或类型）/ `new`（无参构造临时实例，**必须 UI consent**，
  Agent 侧 `consent_required` 硬闸）；
- **参数**：JSON → Java（基本类型 / String / Enum / List / Set / Map / 数组 /
  嵌套 DTO（字段 + setter）/ 泛型集合感知 / null）；重载以擦除参数表
  （FQCN）精确定位，缺省要求方法名唯一；
- **结果**：返回值 + 类型 + 耗时；异常全量结构化（类型 / 消息 / 50 帧堆栈 /
  10 级 cause 链）；循环引用（`__circular__`）/ 深度截断 / 体积截断；
  JDK 代理不穿透 InvocationHandler（`__proxy__` 标注 + 接口清单）；
  Throwable 结果标注 `__throwable__`；
- **超时 / 取消（协作式，诚实语义）**：watchdog 到点 / 用户取消 →
  `Thread.interrupt`——业务代码不响应中断时方法继续跑完（结果丢弃）；
  响应恒带 `terminated: false` + `interrupt_sent`，UI 必须区分
  「已请求取消」与「实际终止」；
- **上限**：并发信号量（默认 4）/ 单次超时 ≤ 300s / 参数 1 MiB /
  结果序列化 1 MiB（双端同口径）。

## 八、安全模型

| 控制点 | 实现 |
|---|---|
| 目标进程身份 | expectedPid 校验 + vm_start_ms 绑定 + pid 一致性回读 |
| Attach 前确认 | JavaAttachConfirmDialog（注入说明 / 回环 / token / 卸载语义） |
| Agent 注入确认 | 同上（确认即注入，audit 级日志） |
| 仅调试选定进程 | 会话按 pid 注册；未 attach 的 pid 所有命令 NOT_FOUND |
| 包 / 类过滤 | 类清单默认排除 `java.* javax.* jdk.* sun.* com.sun.*` 等内部类；include / exclude 前缀可配 |
| 危险类硬闸 | 黑名单前缀（Runtime / ProcessBuilder / ClassLoader / reflect / script…）+ **可赋值性闸**（ClassLoader / Runtime / Thread 的任意子类按类型拒绝，防名字绕过） |
| 方法风险提示 | 方法名破坏性动词（delete / drop / pay…）→ destructive 分级 → UI 二次确认 |
| 调用权限 | 插件门禁（PLUGIN_NOT_INSTALLED / PLUGIN_DISABLED）+ MCP NetworkExecute 高风险门（沿用） |
| 体积上限 | 参数 1 MiB / 结果 1 MiB / 帧 16 MiB |
| 超时 | 单次 ≤ 300s（双端收敛） |
| 并发 | Agent 信号量 4（可配 1..32） |
| 审计 | attach / unload / invoke 落插件 audit 日志（不含参数原文）；调用历史 `java_invocations` 滚动 200 条 |
| 敏感信息 | 进程命令行脱敏回传；token 不落盘不进日志 |
| 卸载 | agent.stop 停服务 + 清 infoFile；已加载类不可卸载（明确告知） |
| 默认不开放 | 任意表达式执行 / Shell 执行 / 不受限代码执行（无此类入口） |

## 九、前端 UI（attach 工作台）

七步交互（`/java-debug`，模式切换「本地进程（默认）/ Runtime (SDK)」）：

1. 自动发现本机 Java 进程（左栏，5s 自动刷新 + 搜索 + 手动 PID）；
2. 选择目标（状态徽章：可连接 / 已连接 / 不可连接 + 原因）；
3. 点击 Attach → **确认对话框**（注入说明）；
4. 类浏览器（中列左：已加载类，JDK 内部类默认排除 + 收藏）；
5. 选择方法（中列右：签名 / 重载徽标 / 风险提示 / 实例来源选择）；
6. 填写参数（可视化表单 ↔ JSON 双模式 + 超时覆盖）；
7. 执行并展示结果（右列：返回值 / 异常堆栈 / 执行信息 / 输入参数）。

顶栏：会话状态 + PID / Java 版本 / agent 版本 chips + 断开 + 卸载 Agent。
左栏第二页签为 **Spring Bean 视图**（支持分级 + Bean 类型 / scope / 代理 /
实例化状态），点击 Bean 直接定位其实现类。

组件复用：Agent 数据在 store 内归一为 v1 形状——JavaClassBrowser /
JavaMethodPanel / JavaResultPanel / JavaHistoryDrawer 原样复用；差异点
（风险 / 实例解析 / consent）以独立 props / 状态暴露。历史与 v1 同表
（attach 会话自动建 `java_projects(mode=attach)` 行）。

首次使用引导：无 JDK / 缺 jar 时左栏给出可操作诊断；JDK 21+ 动态 agent
告警（目标进程 stderr）在确认对话框中预先说明。

## 十、与现有 Miyo 集成

| 能力 | 状态 |
|---|---|
| Java 调试历史 | 同表复用（attach 会话有 `java_projects(mode=attach)` 行） |
| 方法调用记录 / 重放 | 完全沿用（method_id 同键） |
| 集成测试 Java 步骤 | `miyo-agent::java_hook` 按项目 mode 路由：attach 项目经会话表路由（会话未建立显式报错，不隐式注入）；runtime 项目走 v1 客户端 |
| MCP Java 工具（9 个） | 沿用（执行器同上路由）；`run_java_method` 仍走 NetworkExecute 高风险门 |
| AI 调试工作流 | Agent 控制面（AgentState 注入 AttachManager）与 tauri 场景引擎共用单源执行器 |
| v1 SDK 模式 | 保留为兼容连接方式（`mode='runtime'` 项目），文档与接入向导不变 |

远程普通 Java 应用**不能**仅凭本地 Attach 零侵入连接——UI 明确要求
「可访问的远程 Agent 或其他受控调试通道」（v1 Runtime / 自部署 Agent）。

## 十一、兼容性矩阵（实测环境：macOS 15 arm64）

| 目标 JVM | 结论 | 验证 |
|---|---|---|
| JDK 8（Zulu 1.8.0_372） | ✅ 全链路（attach / 握手 / 调用 / 卸载） | `scripts/test_java_attach_matrix.sh` |
| JDK 11（Zulu 11.0.12） | ✅ 全链路 | 同上 |
| JDK 17（Zulu 17.0.13） | ✅ 全链路 | 同上 |
| JDK 21（Zulu 21.0.6） | ✅ 全链路（动态 agent 告警已提示） | 同上 + E2E |
| launcher 运行于 JDK 8（tools.jar 桥） | ✅ attach JDK 17 目标（协议偏斜由兜底吸收） | 实测 |
| launcher 运行于 JDK 21 → JDK 8 目标 | ✅（AgentLoadException 误报由 infoFile 兜底吸收） | 实测 |
| Spring Boot 3.2.5 / Spring 6.1.6 | ✅ 上下文捕获 + Bean 清单 + Bean 调用 | `java/examples/spring-boot-demo` 实测 |
| JDK 22 / 25 / Windows / Linux / 容器 | ⚠️ 未实测——本机无对应环境；协议与字节码按 Java 8 目标设计，理论兼容，**不标记完全支持** | — |

## 十二、自动化测试结果

| 套件 | 结果 |
|---|---|
| Agent 协议冒烟（`java/examples/plain-app/agent_smoke.py`，28 项） | ✅ 全过（鉴权 / 版本协商 / 类 / 方法 / 重载 / 黑名单 / 静态 + DTO 调用 / consent 闸 / 业务异常 / 超时 / 取消 / 重复 attach 换发 / 卸载） |
| JDK 目标矩阵（`scripts/test_java_attach_matrix.sh`） | ✅ JDK 8 / 11 / 17 / 21 全过 |
| Rust `cargo test --workspace`（根 workspace 全量） | ✅ 全绿（新增 miyo-java-attach 13 单测：脱敏 / 版本选择 / 错误映射 / 帧协议 / 握手映射 + storage attach 项目 4 测） |
| Rust Attach E2E（`miyo-java-attach/tests/attach_e2e_test.rs`，env 门控） | ✅ 10 步全链路（发现 → attach → 发现 → 静态调用 → 黑名单 → consent → 取消 → detach → 重连识别 → 卸载） |
| Rust `cargo test`（miyo-tauri 独立 workspace） | ✅ 43 + 契约 3（14 个新命令过三向契约） |
| Spring Boot 实测 | ✅（§六） |
| 前端 `vitest run` | ✅ 149 文件 / 1201 测试（新增 javaAttach 7） |
| 前端 eslint / vue-tsc / vite build | ✅ 干净（attach 代码在独立 chunk，主 chunk 零字节） |
| i18n 门禁 | ✅（java2.* 55 键中英 1:1；白名单补 Java 语义词 primary / private） |

## 十三、安装包体积变化

| 产物 | 体积 |
|---|---|
| `miyo-java-agent.jar`（含 shaded ASM） | ~340 KiB |
| `miyo-attach-launcher.jar` | ~19 KiB |
| Rust 侧新增 crate（miyo-java-attach） | 编译进主二进制（< 200 KiB 增量） |
| 安装包净增 | **< 0.6 MiB**（纯增量，无第三方运行时引入） |

## 十四、已知限制（诚实清单）

1. **类不可卸载**：JVM 不支持运行时卸载 Agent 类——卸载 = 停服务 + 清
   连接信息，已加载的类随 JVM 退出回收（UI 明示）；
2. **超时 / 取消为协作式**：Java 无法强制终止同 JVM 业务线程
   （`terminated` 恒为 false）；
3. **Spring 上下文捕获为条件支持**：attach 晚于应用启动时需一次应用活动
   （HTTP 请求 / 定时任务）；彻底无活动 + 未设 LiveBeansView 属性 →
   显示等待状态与两种解决路径；
4. **实例来源**：无 Spring 时实例方法只能「新建临时实例」（需 consent）；
   无法枚举任意存量对象（纯 Java 无堆遍历能力，JVM TI 需原生库——后续增强）；
5. **传输为回环 TCP**（Java 8 无内置 UDS / Named Pipe）；token 鉴权 +
   仅回环绑定达到同等防护，UDS 作为后续增强；
6. **远程应用**：本地 Attach 不适用——远程需走 v1 Runtime SDK 或受控
   远程 Agent 通道；
7. **懒加载类不可见**：Instrumentation 只能看到已加载类——未触发的类
   需先在应用中触发使用（UI 提示）；
8. **JDK 22+ / Windows / Linux / 容器未实测**（协议按 Java 8 兼容设计）；
9. boot 版本探测：`SpringBootVersion` 未加载时显示 unknown（诚实降级）。

## 十五、模块与文件清单

```text
java/
├── miyo-java-agent/            # 动态注入 Agent（Java 8 + shaded ASM）
│   └── src/main/java/io/miyo/agent/
│       ├── AgentMain.java      # agentmain / premain / boot / shutdown
│       ├── AgentArgs.java      # loadAgent options 解析（URL 编码 k=v）
│       ├── AgentState.java     # 单例 / token 换发 / infoFile（0600）
│       ├── ipc/                # IpcServer / IpcConnection / Frames / RequestDispatcher / Protocol
│       ├── inspect/            # ClassInspector / ClassFilter（已加载类视角）
│       ├── invoke/             # InvocationEngine / ArgsConverter / ResultSerializer / RiskPolicy / InstanceResolver
│       ├── spring/             # ContextDiscovery（策略梯子）/ ContextTransformer（ASM）/ SpringCapture
│       └── util/Json.java
├── miyo-attach-launcher/       # Attach sidecar CLI（JDK8 tools.jar 反射桥 + JDK9+ 直连）
│   └── src/main/java/io/miyo/launcher/{LauncherMain,AttachBridge}.java
└── examples/
    ├── plain-app/              # 零 SDK 验收 demo（Calculator / OrderService）+ agent_smoke.py
    └── spring-boot-demo/       # Spring Boot 3.2.5 验收 demo（PriceService）

crates/miyo-java-attach/        # Rust 侧 Attach 层（进程发现 / JDK 工具链 / launcher runner / Agent IPC client / 会话管理）
├── src/{lib,model,process,jdk,launcher,client,manager}.rs
└── tests/attach_e2e_test.rs    # 全链路 E2E（env 门控）

crates/miyo-tauri/src/
├── commands/java_attach.rs     # 14 个新命令（发现 / attach / 类 / 方法 / Bean / 调用 / 取消）
└── state.rs                    # AppState.java_attach（resource dir / dev java/dist 候选）

crates/miyo-agent/src/java_hook.rs   # 场景 / MCP 单源执行器：按项目 mode 路由

frontend/src/domains/java-debug/
├── stores/javaAttach.ts        # attach store（Agent 数据归一复用 v1 组件）
├── components/{JavaProcessPanel,JavaAttachConfirmDialog,JavaBeansPanel}.vue
└── views/JavaDebugView.vue     # 双模式壳（attach 默认 / runtime 兼容）

scripts/
├── build_java_debug.sh         # agent + launcher 构建 → java/dist + resources
├── ensure-java-jars.sh         # tauri bundle.resources 预置（CI / 打包）
└── test_java_attach_matrix.sh  # JDK 8/11/17/21 目标矩阵

crates/miyo-storage/migrations/0032_java_attach.sql  # java_projects + mode/attach_pid/vm_start_ms
```

### 使用（E2E 复现）

```bash
# 1. 构建 agent / launcher jar（java/dist + tauri resources）
scripts/build_java_debug.sh

# 2. 协议冒烟（28 项）
(cd java/examples/plain-app && javac --release 8 -d build/classes8 @build/sources.txt)
(cd java/examples/plain-app && java -cp build/classes8 com.example.PlainApp &)
python3 java/examples/plain-app/agent_smoke.py <pid> $(openssl rand -hex 32)

# 3. JDK 目标矩阵
scripts/test_java_attach_matrix.sh

# 4. Rust 全链路 E2E
cargo test -p miyo-java-attach --test attach_e2e_test -- --ignored

# 5. Spring Boot 验收
(cd java/examples/spring-boot-demo && mvn package && java -jar target/*.jar &)
# → Miyo → Java 调试 → 本地进程 → Attach → curl /ping 一次 → Bean 页签
```

v1 Runtime SDK 模式的部署与协议文档见 `java/miyo-java-runtime/README.md`
（兼容连接方式，未改动）。
