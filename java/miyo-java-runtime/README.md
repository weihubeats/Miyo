# Miyo Java Debug Runtime SDK

Java 方法实时调试 Runtime（零第三方依赖，纯 JDK 实现）。把它嵌入你的
Java 应用，显式注册允许调试的白名单类，Miyo 桌面端即可连接本进程发现并
调用这些方法——不需要写 HTTP Controller，也不需要 JVM Attach。

## 集成方式

### 方式 A：Maven 依赖

```bash
cd java/miyo-java-runtime && mvn package
# 把 target/miyo-java-runtime-0.1.0.jar 加入你的项目 classpath
```

### 方式 B：直接拷贝源码

把 `src/main/java/io/miyo/debug/` 整个包拷进项目（无外部依赖，无冲突）。

## 启动（三步）

```java
// 1. 构造（token 必配，建议环境变量注入，不要硬编码）
MiyoDebugRuntime runtime = MiyoDebugRuntime.create(
        RuntimeConfig.builder(System.getenv("MIYO_DEBUG_TOKEN"))
                .appName("order-service")
                .port(9918))
        // 2. 显式注册白名单（未注册的类一律拒绝；黑名单类注册即抛异常）
        .register("orderService", OrderService.class,
                () -> applicationContext.getBean(OrderService.class)) // Spring：经容器取 Bean
        .register(null, MathUtil.class, InstanceProvider.staticInstance())
        // 3. 启动
        .start();
```

Spring Boot 项目建议在 `ApplicationRunner` 里启动、`@PreDestroy` 里 `stop()`；
`InstanceProvider` 返回容器 Bean，AOP 代理 / 事务语义与正常业务调用一致。

### 本地自动发现（注册文件）

`start()` 时 SDK 会在 `~/.miyo/debug-runtimes/{app}-{port}.json` 写一条注册
记录（应用名 / 端口 / Java 版本 / PID / SDK 版本，**不含 token**），`stop()`
或进程退出钩子删除。Miyo 桌面端扫描该目录即可列出本机正在运行的
Runtime——「连接本地 Java 应用」入口无需手抄端口。

- 隐私边界：token 永不写入注册文件，连接鉴权仍由你在 Miyo 侧填写；
- 企业环境禁写用户目录：设环境变量 `MIYO_DEBUG_REGISTRY=off` 关闭；
  关闭后 Miyo 仍会对默认端口段（9918-9925）做指纹探测兜底；
- 进程崩溃残留的注册文件会被 Miyo 的端口活性探测自动过滤（标为无响应）。

## 协议（v1）

监听地址默认 `127.0.0.1:9918`，鉴权 `Authorization: Bearer <token>`，
版本头 `X-Miyo-Protocol-Version: 1`。

| 端点 | 用途 |
|---|---|
| `GET  /miyo/debug/info` | 运行时信息（协议版本 / 应用名 / JVM / 白名单规模） |
| `GET  /miyo/debug/classes` | 白名单类清单 |
| `GET  /miyo/debug/methods?class={FQN}` | 类的白名单方法（完整签名 / 参数 / 重载标记） |
| `POST /miyo/debug/invoke` | 执行方法（同步返回结果 / 异常 / 耗时） |
| `POST /miyo/debug/invocations/{id}/cancel` | 取消在途调用（协作式中断） |

方法标识（methodId）形如
`com.example.OrderService#createOrder(CreateOrderRequest,long)`——重载消歧
唯一依据，与 Rust 侧 `crates/miyo-java/src/protocol.rs` 同规则。

## 安全模型

- **显式白名单**：只有注册过的类 / 方法可调用；空白名单拒绝启动；
- **危险类黑名单**：`java.lang.Runtime` / `ProcessBuilder` / `ClassLoader` /
  `System` / 反射与脚本引擎等注册即拒绝（与 Miyo 侧双保险）；
- **鉴权强制**：token 常数时间比较，无 token / 错 token 一律 401；
- **体积限制**：请求体 / 返回值默认 1 MiB（超出截断并标记）；
- **超时上限**：单次调用默认上限 300s，超时返回 timeout 状态；
- **并发 / 频率**：默认并发 4、每分钟 60 次（可配置）；
- **异常不吞**：业务抛出的异常全量结构化回传（类型 / 消息 / 堆栈 / cause 链）。

### 超时与取消语义（重要）

Java 方法执行无法保证强制终止。超时与取消通过线程中断实现——
业务代码不响应中断时方法仍会执行完（结果丢弃、调用按 timeout 返回）。
需要强制终止 / 执行不可信代码的场景应使用独立进程隔离 + 终止 Worker
进程回收（Miyo 后续阶段能力）。

## 支持的参数类型

基本类型与包装类型、`String`/`char`、枚举、`BigDecimal`/`BigInteger`、
`List`/`Set`/`Map`/数组、自定义 DTO（no-arg 构造 + 字段 / setter 注入，
嵌套深度 12 层；泛型集合按字段声明类型递归注入）。未知 DTO 字段忽略并
记入 warnings。泛型擦除限制：无泛型提示的裸集合元素保留 JSON 原生形态。

## 测试

```bash
mvn test-compile
java -cp target/classes:target/test-classes io.miyo.debug.SelfTest
```

端到端联调（编译 SDK + demo 应用 + 完整协议走查）：

```bash
java/examples/demo-app/run_demo.sh
```
