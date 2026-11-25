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
| `classes.list` | search / category / package_prefix / include_dependencies / include_jdk / business_packages / refresh_index / limit / offset（旧 include / exclude 仍生效）。响应：`total / returned / offset / items[] / stats / launch / incremental` |
| `classes.stats` | 分类统计（业务 / Bean / 依赖 / JDK / 生成 / 未识别 + 角色分布 + 启动信息证据），走索引缓存不重扫 |
| `classes.reindex` | 全量重建类索引（业务包前缀变更后让分类按新配置重算） |
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

## 四之二、类发现与分类（业务代码优先）

### 4.2.1 旧实现的问题

| 问题 | 后果 |
|---|---|
| `ClassFilter.isDefaultExcluded` 用包名黑名单（`java.` `javax.` `org.springframework.` …）判「JDK 内部类」 | 业务代码只要不在这张表里就一律当业务类——`io.netty.*` 等依赖被当成业务类；业务项目自己叫 `com.github.*` / `io.miyo.*` 又被误杀 |
| `include_jdk=false` 只是「不显示黑名单里的类」 | 依赖（`BOOT-INF/lib`）与 JDK 混在一个列表里，前端无法分栏，只能靠搜索 |
| 每次 `classes.list` 都 `getAllLoadedClasses()` + 逐类 `getName()` 判断 | 5 万类时一次刷新 = 5 万次字符串前缀比较；无任何缓存，分页/搜索/翻页重复全量遍历 |
| `findLoadedClassOrNull` 每次线性扫描全部类 | 类详情 / 方法详情各扫一遍（O(已加载类数)） |
| 类行数据只有 name / package / simple_name | 前端无法解释「为什么这个类是业务类」，出错时无从排查 |
| 大列表一次性返回（limit 上限 1000） | 目标 JVM 上万个类时，一个「业务类 + 依赖」列表必然截断 |

### 4.2.2 新分类算法（多维度，按证据强度）

`ClassOrigin`（第一手证据）+ `ClassClassifier`（判定）+ `ClassIndex`（缓存/增量）：

1. **动态生成**：名字命中 `$$EnhancerBySpringCGLIB` / `$ByteBuddy$` / `jdk.proxy` /
   `$Proxy` / `$$Lambda$` / `$$_javassist_` / Hibernate / Mockito 等，或
   `Proxy.isProxyClass` → `GENERATED`（代理类同时具有业务与依赖特征，单独成组）。
2. **框架 / 容器自身实现包**（压过 codeSource）：`org.springframework.boot.loader.*`、
   `org.springframework.boot.devtools.*`、`sun.reflect.*`、`jdk.internal.reflect.*`、
   `com.sun.proxy.*` → `DEPENDENCY`。**为什么需要**：Spring Boot 3.2 起 loader 的类与
   app.jar 共享 codeSource（由父加载器加载），只看 codeSource 会把 `ManifestInfo`
   这类框架实现判成业务代码（端到端实测）。
3. **codeSource（强证据）**：
   - `jrt:` / bootstrap（loader=null）→ `JDK_INTERNAL`
   - `BOOT-INF/lib` / `WEB-INF/lib` / `META-INF/lib` / `~/.m2` / `~/.gradle/caches` /
     `~/.ivy2` / `/lib/` / `/repository/` → `DEPENDENCY`
   - `target/classes` / `build/classes` / `out/production`（含 `classes!/` 尾段）→ `APPLICATION`
   - `BOOT-INF/classes` / `WEB-INF/classes`（嵌套目录）→ `APPLICATION`
   - 其它普通 jar（路径无仓库特征）→ `APPLICATION`（应用自身产物）
   - 注意：codeSource 判定为「应用目录 / 应用 jar」时**不会被包名根覆盖**，
     避免业务包恰好叫 `io.miyo` / `com.github` 被误降级。
   - **嵌套 URL 两种形态都认**：Spring Boot 2 的 `jar:file:/app.jar!/BOOT-INF/lib/x.jar!/`
     与 Spring Boot 3.2+ 的 `nested:/app.jar/!BOOT-INF/lib/x.jar`（`!` 后无 `/`）。
     3.2 形态若只按 `!/` 切分，所有依赖都会落到 UNKNOWN（端到端实测：logback 等
     3000+ 个类全为未识别）。
   - **Miyo agent 自身类**（`io.miyo.agent.*`，清单从 agent jar 条目枚举）→ `DEPENDENCY`：
     agent 以 `-javaagent` / attach 注入，在 Spring Boot 里由容器类加载器追加加载，
     codeSource 与 app.jar 同源，否则会污染业务代码列表。
4. **弱证据（仅 codeSource 无法定论：无 codeSource / 未知协议 / 加密加载器）**：
   - Spring 上下文 `ROLE_APPLICATION` Bean 的实现类 → `SPRING_BEAN`
     （只读 `BeanDefinition` 的 `getBeanClassName` / `getResolvableType`，**绝不实例化 Bean**）。
     **三类命中**都算 Bean（类型级，不看是否已实例化）：① `@Component`/`@Service` 等
     声明的类；② `@Bean` 工厂方法的返回类型（这类 BeanDefinition 的 `getBeanClassName()`
     返回 null，只有 resolvable type——真实项目里最常见，只读 beanClassName 会让
     「Spring Beans」组永远为 0）。**Boot 2.x 还有第二层坑**：这类定义的
     `getResolvableType()` 在 Bean 未实例化时也常解析不到，只剩
     `getFactoryMethodName()`；且 `getFactoryBeanName()` 返回的是**工厂 Bean 的名字**
     （实测 `"channelConfig"`，不是类名）。因此降级链是
     `beanClassName → resolvableType → 经 BeanFactory 把工厂名解析成工厂类 → 读工厂方法返回类型`
     （`ClassClassifier.factoryMethodReturnType`，纯元数据反射，不实例化，
     支持 CGLIB `$$EnhancerBySpringCGLIB$$` 后缀剥离）；③ 与 Bean 同源的实现类（同一 `factoryMethodName`，
     或包根 + 去后缀简单名相同，如 `Foo`/`FooImpl`/`DefaultFoo`）。
     不做「同包根一律算 Bean」的猜测——那会把 DTO / 枚举一起吞进 Bean 组。
     Bean 名回填的映射方向是「实现类全名 → Bean 名」（端到端实测曾写反：业务列表
     里 `bean_name` 恒为 null）。`springPresent` 只缓存**正**结果——负结果缓存会在
     agent 早期一次探测后永久关闭 Bean 分类（实测 `spring_bean` 恒为 0 而
     `beans.list` 有 144 个 Bean）。
     **上下文方法一律走 `invokeResolved`**（本类 → 接口 → 父类链），不能用
     `getClass().getMethod(...)`：Boot 2.x 的上下文常是代理/内部实现类，
     直接 `getMethod("getBeanFactory")` 拿不到 → factory 为 null → 整轮 Bean
     证据为空（真实用户现象：「Spring 已就绪」但 Bean 与 Spring Beans 组都为空）。
   - 用户配置的业务包前缀（`com.foo`、`com.foo.*`、逗号/换行/分号分隔）
   - JVM 启动信息推导的包根（`sun.java.command` 主类 / `-jar` 的 Manifest `Start-Class` / `Main-Class`）
   - 依赖容器 ClassLoader（`LaunchedClassLoader` 等；仅辅助）
   - 已知 JDK / 框架包根（保守白名单，如 `java.` `javax.` `org.springframework.`）
5. **否则 `UNKNOWN`**——绝不猜测：前端显示「{N} 个类无法从代码来源判定归属」并给出
   配置业务包前缀的入口。

业务子类型（Controller / Service / Manager / Repository / DTO / Config / 其它）
由类名后缀 + 所在包名推导，仅用于 `APPLICATION` / `SPRING_BEAN` 的分组。

### 4.2.2.1 jar 优先级（「重编无用」的隐性坑）

attach 的 jar 目录候选是**按序取第一个就绪的**，顺序即优先级。曾经的顺序把
数据目录缓存（dev 模式为 `~/Library/Application Support/Miyo-dev/java`）排在仓库
构建产物之前，后果是：

- `scripts/build_java_debug.sh` 重编多少次都无效——attach 永远命中缓存里那份旧副本；
- 界面停在旧行为（扁平列表、出现 `[Lcom.sun.org.apache.xerces...;` 数组类，
  而新 agent 在 `listClasses` 里直接跳过数组类），与「代码没生效」难以区分；
- 排查只能靠 `lsof -p <目标pid> | grep agent.jar` 看进程实际打开的是哪个文件。

现顺序：**dev 构建产物（`java/dist`，`option_env!(CARGO_MANIFEST_DIR)` 推导）
→ 下载缓存 → 随包资源**。官方安装包在运行期没有构建机路径，`ready()` 探测失败
自动落到随包资源，行为不变。`resolve_tools` 选中目录时会打一条 debug 日志
（`attach 使用 jar 目录：…`），避免再次靠 fd 排查；回归用例
`resolve_tools_prefers_first_ready_candidate` 锁住「顺序决定优先级」。

另外 `tauri.conf.json` 的 `beforeDevCommand` / `beforeBuildCommand` 现在都先跑
`scripts/ensure-java-jars.sh`——打包前必定把最新 jar 同步进 `resources/java`，
不会再出现「安装包里是旧 jar」而无人察觉。

### 4.2.2.2 Tauri 命令参数命名（`missing field` 的坑）

`java_agent_invoke` 的入参是嵌套结构体 `JavaAgentInvokeArgs`，前端按驼峰传
（`methodId` / `paramTypes` / `timeoutMs`），但命令上的
`#[tauri::command(rename_all = "camelCase")]` **只作用于顶层命令参数**
（`pid`、`args`），不会下渗到结构体字段——结构体缺 `rename_all` 时 serde 只认
snake_case，调用直接报：

```
invalid args `args` for command `java_agent_invoke`: missing field `method_id`
```

修法：结构体加 `#[serde(rename_all = "camelCase")]`，并对协议原文补
`#[serde(alias = "...")]`（历史回放 / 外部脚本仍可传 snake_case）。
回归用例：`invoke_args_tests`（驼峰可解析、蛇形可解析、缺 `method_id` 仍报错）。

同一类问题的第二处：`instance` 由 `InvocationEngine` **按 snake_case 读**
`bean_name`，前端类型却写 `beanName`；API 层已在发送前归一
（`bean_name: args.instance.bean_name ?? args.instance.beanName`）。

**新增命令必须同时登记三处**，否则运行时被拒或打包权限缺失（两个测试会拦住）：
`build.rs` 的 `COMMANDS`、`permissions/default.toml` 的 `default.permissions`
（kebab-case，如 `allow-java-agent-classes-stats`）、`invoke_handler` 注册。

### 4.2.2.3 实例解析（`not_found` / `Bean 获取失败` 的两个坑）

**坑一：`getBean(Class)` 被解析成 `getBean(String)`。**
`ContextHandle.getBean(name, type)` 在 `name == null` 时本应走 `getBean(Class)`
按类型取（UI 的 `auto → bean` 从不传 Bean 名），但 `resolveMethod` 曾忽略
「同名不同形参」——`getBean(Class)` 与 `getBean(String)` 形参个数都是 1，
`getMethod` 在代理 / 桥接方法上会返回错的那个，结果是
`IllegalArgumentException: argument type mismatch` → 「Bean 获取失败」。
现在 `resolveMethod` 对每一层（本类 / 接口 / 父类链）都做
`paramTypesMatch` 精确校验；按名取时优先 `getBean(String, Class)`，
`NoSuchMethodException` 再降级 `getBean(String)`。

**坑二：`bean` 策略对「不是 Bean 的类」直接失败。**
UI 实例策略 `auto` 对非静态方法一律发 `bean`，而普通 Service / 工具类 / 未注册
实现并不是 Bean，用户只能自己猜「改切 new」。现在 `bean` 是
**Bean 优先 + 有闸降级**：

1. 先按名（有名字时）/ 按类型取 Bean —— 命中即用容器里的真单例；
2. 取不到时报 `not_found` 并**明确说明**「未找到 Spring Bean」+ 下一步（切 `new`
   或 `static`）；
3. **降级为临时实例必须过 consent 闸**：构造器会执行、可能有副作用，与显式
   `new` 等价，不能借 `bean` 绕开用户确认。

上下文本身不可用（未捕获 / 非 Spring）时**不降级**，如实上抛
`unavailableReason()`——环境问题不该伪装成「这个类不是 Bean」。

回归用例：`InstanceResolverTest`（11 条，假 ApplicationContext 覆盖按类型取、
按名取、无 consent 拒绝降级、有 consent 降级、无无参构造、无上下文、显式 new、
未知 kind、空 Bean 名）。`SpringCapture.resetForTests()` 供测试清空进程级捕获表。

### 4.2.2.4 执行反馈与参数录入（两处高频摩擦）

**「点了执行没反应」**：attach 分支的中列右侧**只有方法面板**，结果区在
runtime 分支才渲染——调用确实执行了，但返回值 / 异常 / 耗时没有落点，且
`attachRun` 不 catch，链路错误只进 console。现在：结果区（`JavaResultPanel`
与 runtime 同组件）常驻中列右侧；`attachRun` 成功 / 超时 / 失败 / 抛错各有
toast，成功时附带耗时与实例来源。

**「参数每次都要点编辑」**：复杂参数（集合 / Map / DTO）原先只渲染摘要 +
「编辑 JSON」弹层，改一个字段要两次点击加一次确认。现在：

| 改进 | 说明 |
|---|---|
| 内联直改 | 复杂参数改为文本框，输入即生效；非法 JSON 只标红不丢字符 |
| 骨架默认 | `utils/argsSkeleton.ts`：集合 → `[]`、Map → `{}`、数组 → `[]`、`Optional<T>` 递归内层、常见 DTO（`PageQuery` / `DateRange` / `Money` / `Result`）给键齐全的骨架 |
| 参数记忆 | `pid\|method_id → 上次参数`（localStorage，最近 200 条）；切走再切回自动回填，类型不兼容的位置回落到骨架 |
| 弹层保留 | 「展开编辑」仍提供多行 JSON（粘大段结构用） |

骨架只覆盖「结构必然如此」的情形，**不猜业务字段**：认不出的 DTO 仍给 `null`。

### 4.2.2.5 「断开后仍显示已连接」（两处状态源打架）

1. **面板读错了状态源**：`JavaProcessPanel` 只读扫描结果里的 `p.attached`，
   而它是后端进程扫描快照——断开后要等下一轮 5s 轮询才变，自动刷新关掉时
   永远不变。现在判定只认 `attachedPids`（由 store 会话表派生，`detach()`
   返回即更新）。**不要**改成 `attachedPids.includes(pid) || p.attached`：
   断开瞬间会话表已空、快照仍是旧值，OR 会把 `true` 带回来（本条已用用例锁住）。
2. **会话被扫描结果重建**：`loadProcesses` 原先「扫描说 attached 且本地无会话
   → 拉 `javaAttachSessions()` 写回本地」，在断开瞬间会把刚移除的 pid 塞回
   会话表，徽标在下一轮轮询复活。现在一律以 `javaAttachSessions()` 为唯一
   真相来源，并清理后端已不存在的本地会话；扫描结果的 `attached` 不参与会话重建。

回归用例：`processPanel.spec.ts`（会话表优先、快照滞后不倒退、连接按钮恢复）、
`javaAttach.spec.ts` 的「断开后会话不复活」。`detach()` 结束时会主动
`loadProcesses()`，不依赖轮询间隔。

### 4.2.3 性能

- **索引缓存**：按 `Class<?>` 身份分段缓存判定（32 段 `ConcurrentHashMap`），同一类只分类一次；
  类被卸载后由下一轮扫描清扫。
- **增量刷新**：每轮只分类「新出现」的类（O(新增)）；类集合与分类上下文都没变时，
  刷新退化为一次 `getAllLoadedClasses()` 遍历，统计/摘要直接复用上一轮。
- **不阻塞业务线程**：`classes.list` / `classes.stats` / `classes.reindex` 投递到 agent 的
  worker 池（与 `invoke` 同池），连接线程保持可响应（cancel / status 不受影响）；
  索引刷新只读 `ProtectionDomain` / `BeanDefinition`，不触发类初始化、不实例化 Bean。
- **分页**：默认 200（上限 1000），前端滚动到底增量加载，按类别 → 包名 → 类名稳定排序。

### 4.2.4 兼容性

- 协议版本仍为 2（仅新增参数字段与 `classes.stats` / `classes.reindex` 两个新 op）。
- 老调用方传 `include` / `exclude` / `include_jdk` 语义不变；`include` 命中即保留，
  不受类别筛选限制。
- `ClassFilter` 的老 API（`matches` / `isDefaultExcluded` / `searchMatches`）保留（二进制兼容），
  但默认过滤不再使用它们。
- runtime（SDK 白名单）模式不经过本分类：前端组件对缺失 `category` 的行退回扁平列表。
- Java 8 字节码、无 Java 9+ API、无目标应用改动、不强制 SDK、不影响 invoke / 历史 / 插件架构。

### 4.2.5 端到端实测（真实目标进程，非 mock）

用 `java/dist/miyo-java-agent.jar` + 真实 attach 直连 agent 协议验证（脚本在本地
临时目录，未入库）：

| 目标 | JDK | 业务 | Bean | 依赖 | JDK | 生成 | 未识别 |
|---|---|---|---|---|---|---|---|
| `spring-boot-demo`（Boot 3.2 fat-jar） | 21 | 3 | 2 | 3450 | 2429 | 830 | **0** |
| `plain-app`（classpath 目录，无 Spring） | 17 | 8 | 0 | 49 | 997 | 23 | **0** |
| `plain-app` | 21 | 8 | 0 | 51 | 1112 | 27 | **0** |
| `spring-boot2-demo`（Boot 2.7 / Spring 5.3，接口式业务 + `@Bean`） | 17 | 3 | 3 | 3478 | 2329 | 751 | **0** |

Spring demo 的业务类是 `DemoApplication` / `DemoApplication$PingController`（controller）
/ `PriceService`（service）——与源码一一对应；`spring_bean` 要等一次 HTTP 请求触发
上下文捕获插桩后才有值（设计内降级路径，`beans.contexts` 会给出触发方法）。

增量刷新：第二次 `classes.list` `added=0`、`elapsed_ms≈8`（7219 个类，一次
`getAllLoadedClasses()` 遍历）；`classes.reindex` 全量重建 ≈20ms。

这轮实测修掉了 5 个单测/编译都拦不住的 bug，已各自补上回归用例
（`EndToEndRegressionTest`）：Boot 3.2 `nested:` URL、agent 自身类、Boot loader 类、
Bean 名方向、`springPresent` 负缓存。

用户第一轮反馈（UI 仍显示旧扁平列表、`-1 个方法`）又暴露两个前端缺陷，已修：
`categorized` 的 Boolean prop 默认值让行推断兜底失效（组件回归用例
`显式 categorized 时即使只有业务类也渲染分类树`）、`CollapsibleSection` 同型
陷阱（`open` 受控/非受控判别失效）。同时补上 `@Bean` 返回类型与同源实现类的
Spring 证据收集（`SpringBeanEvidenceTest` 5 个用例）。

### 4.2.6 前端

- 左侧浏览器变为可折叠分组树：业务代码（按角色细分）→ Spring Bean → 第三方依赖（默认折叠）
  → JDK（默认隐藏，勾选后显示）→ 动态生成 → 未识别（默认折叠）。
- 类别 tab 筛选 + 包前缀输入 + 「含依赖 / 含 JDK」开关 + 业务包前缀配置（持久化到
  `localStorage`，改变后触发 `classes.reindex`）+ 「恢复默认」。
- **分类视图开关由调用方显式给出**（`categorized`），不能只靠「返回行里有没有
  `category`」推断：attach 首屏可能一个 Bean 都没有（Spring 上下文要等目标应用访问
  一次才捕获），早期实现用 `??` 做行推断兜底，而 `defineProps` 把
  `categorized?: boolean` 编译成 Boolean prop + `default: false`，兜底永不生效——
  结果组标题 / 类别 tab / 开关整块消失，界面退回显示「-1 个方法」的扁平旧列表
  （真实用户截图复现，已加组件回归用例）。
- 统计行常驻显示：「业务代码 N · Spring Bean N · 第三方依赖 N · JDK / JVM N」。

## 五、Attach Launcher（sidecar）

`java/miyo-attach-launcher/target/miyo-attach-launcher.jar`（Java 8 字节码，
零依赖），Miyo 以 `java -jar` 短进程调用，单行 JSON stdout + 退出码：

| 命令 | 用途 |
|---|---|
| `capabilities` | 运行环境自检（bridge_available / tools.jar 位置 / java 版本） |
| `list` | Attach API 视角的可 attach JVM（`VirtualMachine.list()`） |
| `properties --pid N` | 目标 JVM 系统属性（预检版本 / 厂商，不加载 Agent） |


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
| 调用权限 | 插件门禁 **28/28 命令全覆盖**（v1 `java_*` 14 条 + attach `java_attach_*` / `java_agent_*` 14 条：未安装 → PLUGIN_NOT_INSTALLED / 停用 → PLUGIN_DISABLED；进程发现同样受门禁——未装插件不启动 launcher sidecar）+ MCP NetworkExecute 高风险门（沿用） |
| 组件自动获取 | agent / launcher jar 按需从官方 Release 下载（`releases/latest/download` 稳定链接 + sha256 sidecar 校验 + 原子落盘 `{data_dir}/java/`）；插件启用后工作台挂载即自动检查，缺失自动下载，ProcessPanel 可手动重试——**安装包不再强制携带 jar** |
| 会话上限 | 单实例 attach 会话 ≤ 8（超出需先断开；幂等重连不受限） |
| 会话健康 | 心跳 30s（ping 内置 5s 超时）——目标 JVM 静默死亡后 stale 会话自动移除并清理 infoFile |
| 临时文件 | 会话移除（detach / 卸载 / 失联）即时删自己的 infoFile；心跳循环清扫 >24h 残留（兜底 Miyo 崩溃 / 目标 JVM 静默死亡）；应用退出（ExitRequested）尽力 agent.stop 全部会话 |
| 体积上限 | 参数 1 MiB / 结果 1 MiB / 帧 16 MiB |
| 超时 | 单次 ≤ 300s（双端收敛） |
| 并发 | Agent 信号量 4（可配 1..32） |
| 审计 | attach / unload / invoke 落插件 audit 日志（不含参数原文）；调用历史 `java_invocations` 滚动 200 条 |
| 敏感信息 | 进程命令行脱敏回传；token 不落盘不进日志、**不经子进程 argv**（attach 参数经 launcher stdin 传入，`ps` 不可见） |
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
| i18n 门禁 | ✅（java2.* 53 键中英 1:1；白名单补 Java 语义词 primary / private） |

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
    ├── spring-boot-demo/       # Spring Boot 3.2.5 验收 demo（PriceService）
    └── spring-boot2-demo/      # Spring Boot 2.7 / Spring 5.3 验收 demo（接口 + 实现类、@Bean）

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
