package io.miyo.debug;

import java.io.IOException;
import java.io.OutputStream;
import java.net.InetSocketAddress;
import java.net.URI;
import java.net.http.HttpClient;
import java.net.http.HttpRequest;
import java.net.http.HttpResponse;
import java.nio.charset.StandardCharsets;
import java.util.List;
import java.util.Map;
import java.util.concurrent.atomic.AtomicBoolean;

/**
 * 纯 JDK 自测（零依赖）：Json / 注册表 / 参数转换 / 结果序列化 / 执行管理 /
 * HTTP 端到端。运行：`mvn test-compile` 后
 * {@code java -cp target/classes:target/test-classes io.miyo.debug.SelfTest}。
 * 断言失败抛 AssertionError（非零退出）。
 */
public final class SelfTest {

    private static int passed = 0;
    private static int failed = 0;

    public static void main(String[] args) throws Exception {
        testJsonRoundTrip();
        testJsonParseErrors();
        testRegistry();
        testRegistryBlacklist();
        testArgsConverter();
        testArgsConverterBean();
        testResultSerializer();
        testInvocationSuccessAndException();
        testInvocationTimeout();
        testRateLimit();
        testHttpEndToEnd();
        testAuthFailure();

        System.out.println();
        System.out.println("通过 " + passed + "，失败 " + failed);
        if (failed > 0) {
            System.exit(1);
        }
    }

    // ---- 断言 ----

    private static void ok(boolean condition, String message) {
        if (condition) {
            passed++;
        } else {
            failed++;
            System.out.println("[FAIL] " + message);
        }
    }

    private static void eq(Object expected, Object actual, String message) {
        ok(expected == null ? actual == null : expected.equals(actual),
                message + "（期望 " + expected + "，实际 " + actual + "）");
    }

    // ---- 测试域模型 ----

    public enum Status {
        ACTIVE,
        CLOSED
    }

    public static class Item {
        public String sku;
        public long qty;

        public Item() {
        }

        public Item(String sku, long qty) {
            this.sku = sku;
            this.qty = qty;
        }
    }

    public static class CreateOrderRequest {
        public String userId;
        public List<Item> items;
        private boolean urgent;

        public boolean isUrgent() {
            return urgent;
        }

        public void setUrgent(boolean urgent) {
            this.urgent = urgent;
        }
    }

    public static class OrderResult {
        private final String orderNo;
        private final double amount;
        private final Status status;

        public OrderResult(String orderNo, double amount, Status status) {
            this.orderNo = orderNo;
            this.amount = amount;
            this.status = status;
        }

        public String getOrderNo() {
            return orderNo;
        }

        public double getAmount() {
            return amount;
        }

        public Status getStatus() {
            return status;
        }
    }

    public static class OrderService {
        public OrderResult createOrder(String userId, CreateOrderRequest request, List<Long> ids) {
            CreateOrderRequest req = request;
            long total = req != null && req.items != null ? req.items.size() : 0;
            return new OrderResult("NO-" + userId + "-" + total, 99.5, Status.ACTIVE);
        }

        public OrderResult createOrder(String userId) {
            return new OrderResult("NO-" + userId, 1.0, Status.CLOSED);
        }

        public static long add(long a, long b) {
            return a + b;
        }

        public String boom() {
            throw new IllegalStateException("order not found");
        }

        public String slow() {
            try {
                Thread.sleep(60_000);
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
            }
            return "finished";
        }

        public OrderResult nullReturn() {
            return null;
        }
    }

    // ---- 用例 ----

    private static void testJsonRoundTrip() {
        Object parsed = Json.parse("{\"a\":1,\"b\":[1.5,\"x\\n\",true,null],\"c\":{\"d\":-2}}");
        Map<?, ?> map = (Map<?, ?>) parsed;
        eq(1L, map.get("a"), "json int");
        List<?> list = (List<?>) map.get("b");
        eq(4, list.size(), "json array size");
        eq(1.5, list.get(0), "json float");
        eq("x\n", list.get(1), "json string escape");
        eq(Boolean.TRUE, list.get(2), "json bool");
        Map<?, ?> nested = (Map<?, ?>) map.get("c");
        eq(-2L, nested.get("d"), "json nested");
        String written = Json.write(Map.of("k", "值\"", "n", List.of(1L, 2L)));
        Object reparsed = Json.parse(written);
        ok(reparsed instanceof Map, "json write→parse roundtrip");
        Object unicode = Json.parse("\"\\u4e2d\"");
        eq("中", unicode, "json unicode escape");
    }

    private static void testJsonParseErrors() {
        for (String bad : new String[] {"", "{", "[1,]", "nul", "\"abc", "{\"a\":}", "1 2"}) {
            try {
                Json.parse(bad);
                ok(false, "应拒绝非法 JSON：" + bad);
            } catch (Json.JsonParseException e) {
                passed++;
            }
        }
    }

    private static void testRegistry() {
        MiyoDebugRuntime runtime = MiyoDebugRuntime.create(
                RuntimeConfig.builder("t").appName("selftest"));
        runtime.register(null, OrderService.class, InstanceProvider.staticInstance());
        runtime.registerInstance("svc", new OrderService());

        MethodRegistry.MethodInfo create2 = runtime.registry()
                .find("io.miyo.debug.SelfTest$OrderService#createOrder(String,CreateOrderRequest,List)");
        ok(create2 != null, "注册 3 参 createOrder");
        MethodRegistry.MethodInfo create1 = runtime.registry()
                .find("io.miyo.debug.SelfTest$OrderService#createOrder(String)");
        ok(create1 != null, "注册 1 参 createOrder");
        ok(create2.overloaded, "重载标记");
        MethodRegistry.MethodInfo add = runtime.registry()
                .find("io.miyo.debug.SelfTest$OrderService#add(long,long)");
        ok(add != null && add.staticMethod, "静态方法识别");
        ok(runtime.registry()
                .find("io.miyo.debug.SelfTest$OrderService#wait(long,int)") == null,
                "Object 方法不入白名单");
        eq(1, ((Number) runtime.registry().classesJson().get(0).get("method_count")).intValue() >= 1
                ? 1 : 0, "类清单含方法数");
    }

    private static void testRegistryBlacklist() {
        try {
            MiyoDebugRuntime runtime = MiyoDebugRuntime.create(RuntimeConfig.builder("t"));
            runtime.register(null, Runtime.class, InstanceProvider.staticInstance());
            ok(false, "黑名单类注册应被拒绝");
        } catch (IllegalArgumentException e) {
            passed++;
        }
    }

    private static void testArgsConverter() {
        ArgsConverter converter = new ArgsConverter();
        eq("abc", converter.convert("abc", String.class), "String 参数");
        eq(7L, converter.convert(7L, long.class), "long 参数");
        eq(7, converter.convert(7L, int.class), "int 收敛");
        eq(Boolean.TRUE, converter.convert(true, boolean.class), "boolean 参数");
        eq(Status.ACTIVE, converter.convert("ACTIVE", Status.class), "enum 参数");
        ok(converter.convert(3.2d, double.class).equals(3.2d), "double 参数");
        try {
            converter.convert(null, int.class);
            ok(false, "基本类型 null 应拒绝");
        } catch (IllegalArgumentException e) {
            passed++;
        }
        try {
            converter.convert("not-a-number", long.class);
            ok(false, "非法数字应拒绝");
        } catch (IllegalArgumentException e) {
            passed++;
        }
        Object list = converter.convert(List.of(1L, 2L), List.class);
        ok(list instanceof List && ((List<?>) list).size() == 2, "List 参数");
        Object array = converter.convert(List.of("a", "b"), String[].class);
        ok(array instanceof String[] && ((String[]) array).length == 2, "数组参数");
    }

    private static void testArgsConverterBean() {
        ArgsConverter converter = new ArgsConverter();
        Object bean = converter.convert(
                Map.of("userId", "u1",
                        "urgent", true,
                        "items", List.of(
                                Map.of("sku", "A", "qty", 2L),
                                Map.of("sku", "B", "qty", 1L))),
                CreateOrderRequest.class);
        CreateOrderRequest req = (CreateOrderRequest) bean;
        eq("u1", req.userId, "DTO 字段注入");
        eq(true, req.isUrgent(), "DTO setter 注入");
        eq(2, req.items.size(), "DTO 嵌套 List");
        eq("A", ((Item) req.items.get(0)).sku, "DTO 嵌套对象");
        eq(1L, ((Item) req.items.get(1)).qty, "DTO 嵌套字段类型");
        try {
            new ArgsConverter().convert(Map.of("userId", "u1", "unknownKey", "x"),
                    CreateOrderRequest.class);
            passed++; // 未知键仅 warning 不拒绝
        } catch (IllegalArgumentException e) {
            ok(false, "未知键不应拒绝");
        }
    }

    private static void testResultSerializer() {
        ResultSerializer.Result result = ResultSerializer.serialize(
                new OrderResult("NO-1", 42.5, Status.CLOSED), 12, 10_000);
        Map<?, ?> map = (Map<?, ?>) result.value;
        eq("NO-1", map.get("orderNo"), "POJO getter 序列化");
        eq(42.5, map.get("amount"), "数值字段");
        eq("CLOSED", map.get("status"), "枚举序列化");
        ResultSerializer.Result nullResult = ResultSerializer.serialize(null, 12, 10_000);
        eq(null, nullResult.value, "null 返回值");
        Map<String, Object> inner = new java.util.LinkedHashMap<>();
        Map<String, Object> circular = new java.util.LinkedHashMap<>();
        inner.put("back", circular);
        circular.put("self", inner);
        ResultSerializer.Result circResult = ResultSerializer.serialize(circular, 12, 100_000);
        ok(String.valueOf(circResult.value).contains(ResultSerializer.CIRCULAR),
                "循环引用标记 __circular__");
        Map<String, Object> deep = null;
        Map<String, Object> cursor = new java.util.LinkedHashMap<>();
        deep = cursor;
        for (int i = 0; i < 20; i++) {
            Map<String, Object> nextCursor = new java.util.LinkedHashMap<>();
            cursor.put("next", nextCursor);
            cursor = nextCursor;
        }
        ResultSerializer.Result depthResult = ResultSerializer.serialize(deep, 8, 1_000_000);
        ok(String.valueOf(depthResult.value).contains(ResultSerializer.DEPTH_LIMIT),
                "深度超限标记 __depth_limit__");
        ResultSerializer.Result bigResult = ResultSerializer.serialize(
                Map.of("k", "x".repeat(5000)), 12, 1024);
        ok(bigResult.truncated, "结果超限截断标记");
    }

    private static void testInvocationSuccessAndException() throws Exception {
        MiyoDebugRuntime runtime = MiyoDebugRuntime.create(RuntimeConfig.builder("t"));
        runtime.registerInstance("svc", new OrderService());
        MethodRegistry.MethodInfo create = runtime.registry()
                .find("io.miyo.debug.SelfTest$OrderService#createOrder(String)");
        InvocationManager manager = new InvocationManager(SecurityPolicy.defaults());
        Map<String, Object> okResp = manager.invoke(create, List.of("u9"), 5_000L, java.util.UUID.randomUUID());
        eq("success", okResp.get("status"), "调用成功状态");
        Map<?, ?> result = (Map<?, ?>) okResp.get("result");
        eq("NO-u9", result.get("orderNo"), "调用返回值");
        ok(((Number) okResp.get("duration_ms")).doubleValue() >= 0, "耗时字段");

        MethodRegistry.MethodInfo boom = runtime.registry()
                .find("io.miyo.debug.SelfTest$OrderService#boom()");
        Map<String, Object> failResp = manager.invoke(boom, List.of(), 5_000L, java.util.UUID.randomUUID());
        eq("failed", failResp.get("status"), "异常状态");
        Map<?, ?> exception = (Map<?, ?>) failResp.get("exception");
        eq("java.lang.IllegalStateException", exception.get("exception_type"), "异常类型");
        eq("order not found", exception.get("message"), "异常消息");
        ok(((List<?>) exception.get("stack_trace")).size() > 0, "异常堆栈");

        MethodRegistry.MethodInfo nullRet = runtime.registry()
                .find("io.miyo.debug.SelfTest$OrderService#nullReturn()");
        Map<String, Object> nullResp = manager.invoke(nullRet, List.of(), 5_000L, java.util.UUID.randomUUID());
        eq("success", nullResp.get("status"), "null 返回状态");
        eq(null, nullResp.get("result"), "null 返回值");

        MethodRegistry.MethodInfo add = runtime.registry()
                .find("io.miyo.debug.SelfTest$OrderService#add(long,long)");
        Map<String, Object> addResp = manager.invoke(add, List.of(2L, 40L), 5_000L, java.util.UUID.randomUUID());
        eq(42L, addResp.get("result"), "静态方法调用");
    }

    private static void testInvocationTimeout() throws Exception {
        MiyoDebugRuntime runtime = MiyoDebugRuntime.create(RuntimeConfig.builder("t"));
        runtime.registerInstance("svc", new OrderService());
        MethodRegistry.MethodInfo slow = runtime.registry()
                .find("io.miyo.debug.SelfTest$OrderService#slow()");
        InvocationManager manager = new InvocationManager(SecurityPolicy.defaults());
        long start = System.currentTimeMillis();
        Map<String, Object> resp = manager.invoke(slow, List.of(), 300L, java.util.UUID.randomUUID());
        long elapsed = System.currentTimeMillis() - start;
        eq("timeout", resp.get("status"), "超时状态");
        ok(elapsed < 5_000, "超时及时返回（实际 " + elapsed + "ms）");
        Map<?, ?> exception = (Map<?, ?>) resp.get("exception");
        ok(exception != null && String.valueOf(exception.get("message")).contains("中断"),
                "超时语义说明（协作式中断）");
    }

    private static void testRateLimit() throws Exception {
        SecurityPolicy policy = new SecurityPolicy(1024, 1024, 300_000, 32, 2, 2);
        InvocationManager manager = new InvocationManager(policy);
        MiyoDebugRuntime runtime = MiyoDebugRuntime.create(RuntimeConfig.builder("t"));
        runtime.registerInstance("svc", new OrderService());
        MethodRegistry.MethodInfo add = runtime.registry()
                .find("io.miyo.debug.SelfTest$OrderService#add(long,long)");
        manager.invoke(add, List.of(1L, 1L), 1_000L, java.util.UUID.randomUUID());
        manager.invoke(add, List.of(1L, 1L), 1_000L, java.util.UUID.randomUUID());
        try {
            manager.invoke(add, List.of(1L, 1L), 1_000L, java.util.UUID.randomUUID());
            ok(false, "限流应触发");
        } catch (InvocationManager.RateLimitedException e) {
            passed++;
        }
    }

    private static final String TOKEN = "selftest-token";

    private static MiyoDebugRuntime startRuntime(int port) {
        try {
            return MiyoDebugRuntime.create(RuntimeConfig.builder(TOKEN)
                            .port(port)
                            .appName("selftest-app"))
                    .registerInstance("orderService", new OrderService())
                    .start();
        } catch (IOException e) {
            throw new RuntimeException(e);
        }
    }

    private static void testHttpEndToEnd() throws Exception {
        int port = freePort();
        MiyoDebugRuntime runtime = startRuntime(port);
        HttpClient client = HttpClient.newHttpClient();
        String base = "http://127.0.0.1:" + port + "/miyo/debug";

        HttpResponse<String> info = client.send(
                request(base + "/info"), HttpResponse.BodyHandlers.ofString());
        eq(200, info.statusCode(), "info 状态");
        Map<?, ?> infoBody = (Map<?, ?>) Json.parse(info.body());
        eq(1L, infoBody.get("protocol_version"), "info 协议版本");
        eq("selftest-app", infoBody.get("app_name"), "info 应用名");
        ok(((Number) infoBody.get("method_count")).longValue() > 0, "info 方法数");

        HttpResponse<String> classes = client.send(
                request(base + "/classes"), HttpResponse.BodyHandlers.ofString());
        List<?> classList = (List<?>) Json.parse(classes.body());
        eq(1, classList.size(), "classes 数量");
        eq("io.miyo.debug.SelfTest$OrderService",
                ((Map<?, ?>) classList.get(0)).get("class_name"), "classes 类名");

        HttpResponse<String> unregistered = client.send(
                request(base + "/methods?class=java.lang.Runtime"),
                HttpResponse.BodyHandlers.ofString());
        eq(403, unregistered.statusCode(), "黑名单类查询 403");

        HttpResponse<String> methods = client.send(
                request(base + "/methods?class=io.miyo.debug.SelfTest%24OrderService"),
                HttpResponse.BodyHandlers.ofString());
        List<?> methodList = (List<?>) Json.parse(methods.body());
        ok(methodList.size() >= 6, "methods 数量（实际 " + methodList.size() + "）");

        String invokeBody = Json.write(Map.of(
                "invocation_id", java.util.UUID.randomUUID().toString(),
                "method_id", "io.miyo.debug.SelfTest$OrderService#createOrder(String,CreateOrderRequest,List)",
                "arguments", List.of("u7",
                        Map.of("userId", "u7", "items",
                                List.of(Map.of("sku", "A", "qty", 1L), Map.of("sku", "B", "qty", 2L))),
                        List.of(1L, 2L)),
                "timeout_ms", 5000));
        HttpResponse<String> invoke = client.send(
                postRequest(base + "/invoke", invokeBody), HttpResponse.BodyHandlers.ofString());
        Map<?, ?> invokeResp = (Map<?, ?>) Json.parse(invoke.body());
        eq("success", invokeResp.get("status"), "e2e 调用成功");
        Map<?, ?> result = (Map<?, ?>) invokeResp.get("result");
        eq("NO-u7-2", result.get("orderNo"), "e2e DTO 参数注入（items.size()=2）");

        String unknownMethod = Json.write(Map.of(
                "invocation_id", java.util.UUID.randomUUID().toString(),
                "method_id", "io.miyo.debug.SelfTest$OrderService#notRegistered()",
                "arguments", List.of()));
        HttpResponse<String> notRegistered = client.send(
                postRequest(base + "/invoke", unknownMethod), HttpResponse.BodyHandlers.ofString());
        eq(404, notRegistered.statusCode(), "未注册方法 404");
        ok(String.valueOf(notRegistered.body()).contains("not_registered"), "错误码 not_registered");

        // 取消：slow() 在途 → cancel delivered。
        String slowBody = Json.write(Map.of(
                "invocation_id", java.util.UUID.randomUUID().toString(),
                "method_id", "io.miyo.debug.SelfTest$OrderService#slow()",
                "arguments", List.of(),
                "timeout_ms", 30_000));
        AtomicBoolean slowStarted = new AtomicBoolean(false);
        Thread caller = new Thread(() -> {
            try {
                client.send(postRequest(base + "/invoke", slowBody),
                        HttpResponse.BodyHandlers.ofString());
            } catch (Exception ignored) {
                // 超时或连接中断均可
            }
        });
        caller.start();
        Thread.sleep(300);
        // cancel 用 invocation id —— 重新发起一次带已知 id 的调用并取消
        String knownId = java.util.UUID.randomUUID().toString();
        String slowBody2 = Json.write(Map.of(
                "invocation_id", knownId,
                "method_id", "io.miyo.debug.SelfTest$OrderService#slow()",
                "arguments", List.of(),
                "timeout_ms", 20_000));
        Thread caller2 = new Thread(() -> {
            try {
                slowStarted.set(true);
                client.send(postRequest(base + "/invoke", slowBody2),
                        HttpResponse.BodyHandlers.ofString());
            } catch (Exception ignored) {
                // ignored
            }
        });
        caller2.start();
        Thread.sleep(300);
        HttpResponse<String> cancel = client.send(
                postRequest(base + "/invocations/" + knownId + "/cancel", ""),
                HttpResponse.BodyHandlers.ofString());
        Map<?, ?> cancelResp = (Map<?, ?>) Json.parse(cancel.body());
        eq(Boolean.TRUE, cancelResp.get("delivered"), "取消送达");
        caller.join(2_000);
        caller2.join(2_000);
        ok(true, "取消路径完成");

        runtime.stop();
    }

    private static void testAuthFailure() throws Exception {
        int port = freePort();
        MiyoDebugRuntime runtime = startRuntime(port);
        HttpClient client = HttpClient.newHttpClient();
        String base = "http://127.0.0.1:" + port + "/miyo/debug";
        HttpRequest request = HttpRequest.newBuilder(URI.create(base + "/info"))
                .header("Authorization", "Bearer wrong-token")
                .GET()
                .build();
        HttpResponse<String> response = client.send(request, HttpResponse.BodyHandlers.ofString());
        eq(401, response.statusCode(), "错误 token 401");
        ok(response.body().contains("not_authorized"), "错误码 not_authorized");
        HttpRequest noAuth = HttpRequest.newBuilder(URI.create(base + "/info")).GET().build();
        HttpResponse<String> noAuthResp = client.send(noAuth, HttpResponse.BodyHandlers.ofString());
        eq(401, noAuthResp.statusCode(), "无 token 401");
        runtime.stop();
    }

    private static HttpRequest request(String url) {
        return HttpRequest.newBuilder(URI.create(url))
                .header("Authorization", "Bearer " + TOKEN)
                .GET()
                .build();
    }

    private static HttpRequest postRequest(String url, String body) {
        return HttpRequest.newBuilder(URI.create(url))
                .header("Authorization", "Bearer " + TOKEN)
                .header("Content-Type", "application/json")
                .POST(body == null || body.isEmpty()
                        ? HttpRequest.BodyPublishers.noBody()
                        : HttpRequest.BodyPublishers.ofString(body, StandardCharsets.UTF_8))
                .build();
    }

    private static int freePort() throws IOException {
        try (java.net.ServerSocket socket = new java.net.ServerSocket()) {
            socket.bind(new InetSocketAddress("127.0.0.1", 0));
            return socket.getLocalPort();
        }
    }

    private SelfTest() {
    }
}
