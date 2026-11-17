package io.miyo.debug;

import com.sun.net.httpserver.HttpExchange;
import com.sun.net.httpserver.HttpServer;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.net.InetSocketAddress;
import java.net.URLDecoder;
import java.nio.charset.StandardCharsets;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.UUID;
import java.util.concurrent.Executors;

/**
 * 调试协议 HTTP 端点（com.sun.net.httpserver，零依赖）。
 *
 * 路由（协议 v1，与 crates/miyo-java protocol.rs 对齐）：
 * - GET  /miyo/debug/info
 * - GET  /miyo/debug/classes
 * - GET  /miyo/debug/methods?class={FQN}
 * - POST /miyo/debug/invoke
 * - POST /miyo/debug/invocations/{id}/cancel
 *
 * 鉴权：每请求 Bearer Token 常数时间比较；协议版本头校验。
 * 错误体：{@code {"error":{"code":..,"message":..}}}，code 与 Miyo 客户端
 * 的 RuntimeErrorKind 逐一对应。
 */
final class HttpApi {

    static final String PROTOCOL_VERSION = "1";

    private final RuntimeConfig config;
    private final MethodRegistry registry;
    private final InvocationManager invocations;
    private HttpServer server;

    HttpApi(RuntimeConfig config, MethodRegistry registry, InvocationManager invocations) {
        this.config = config;
        this.registry = registry;
        this.invocations = invocations;
    }

    /** 启动监听（bind 地址取配置）。 */
    void start() throws IOException {
        HttpServer server = HttpServer.create(new InetSocketAddress(config.bindHost, config.port), 32);
        server.createContext("/miyo/debug", exchange -> {
            try {
                handle(exchange);
            } catch (Exception e) {
                respondError(exchange, 500, "internal", "Runtime 内部错误：" + e.getMessage());
            } finally {
                exchange.close();
            }
        });
        server.setExecutor(Executors.newFixedThreadPool(config.httpThreads, r -> {
            Thread thread = new Thread(r, "miyo-debug-http");
            thread.setDaemon(true);
            return thread;
        }));
        server.start();
        this.server = server;
    }

    void stop() {
        if (server != null) {
            server.stop(0);
        }
    }

    private void handle(HttpExchange exchange) throws IOException {
        String path = exchange.getRequestURI().getPath();
        String method = exchange.getRequestMethod();

        // 鉴权先行（info/classes/methods/invoke/cancel 全部需要）。
        String auth = exchange.getRequestHeaders().getFirst("Authorization");
        String expected = "Bearer " + config.token;
        if (auth == null || !constantTimeEquals(auth.trim(), expected)) {
            respondError(exchange, 401, "not_authorized", "token 不匹配或缺失");
            return;
        }
        // 协议版本校验（客户端带 X-Miyo-Protocol-Version；缺失视为 v1）。
        String version = exchange.getRequestHeaders().getFirst("X-Miyo-Protocol-Version");
        if (version != null && !PROTOCOL_VERSION.equals(version.trim())) {
            respondError(exchange, 400, "bad_request",
                    "协议版本不匹配：客户端 v" + version + "，Runtime v" + PROTOCOL_VERSION);
            return;
        }

        String route = path.substring("/miyo/debug".length());
        if ("GET".equals(method) && "/info".equals(route)) {
            handleInfo(exchange);
        } else if ("GET".equals(method) && "/classes".equals(route)) {
            handleClasses(exchange);
        } else if ("GET".equals(method) && "/methods".equals(route)) {
            handleMethods(exchange, queryParams(exchange));
        } else if ("POST".equals(method) && "/invoke".equals(route)) {
            handleInvoke(exchange);
        } else if ("POST".equals(method) && route.startsWith("/invocations/")
                && route.endsWith("/cancel")) {
            handleCancel(exchange, route);
        } else {
            respondError(exchange, 404, "bad_request", "未知端点：" + method + " " + path);
        }
    }

    private void handleInfo(HttpExchange exchange) throws IOException {
        Map<String, Object> info = new LinkedHashMap<>();
        info.put("protocol_version", Integer.parseInt(PROTOCOL_VERSION));
        info.put("runtime_version", RuntimeConfig.SDK_VERSION);
        info.put("app_name", config.appName);
        info.put("java_version", System.getProperty("java.version", ""));
        info.put("jvm_version", System.getProperty("java.vm.name", "") + " "
                + System.getProperty("java.vm.version", ""));
        info.put("uptime_ms", java.lang.management.ManagementFactory.getRuntimeMXBean().getUptime());
        info.put("class_count", registry.classCount());
        info.put("method_count", registry.methodCount());
        info.put("instance_provider", config.instanceProviderName);
        respondJson(exchange, 200, Json.write(info));
    }

    private void handleClasses(HttpExchange exchange) throws IOException {
        List<Map<String, Object>> classes = registry.classesJson();
        respondJson(exchange, 200, Json.write(classes));
    }

    private void handleMethods(HttpExchange exchange, Map<String, String> query) throws IOException {
        String className = query.get("class");
        if (className == null || className.trim().isEmpty()) {
            respondError(exchange, 400, "bad_request", "缺少 class 参数（类全限定名）");
            return;
        }
        className = className.trim();
        if (SecurityPolicy.blacklisted(className)) {
            respondError(exchange, 403, "not_allowed", className + " 属于受限类");
            return;
        }
        List<Map<String, Object>> result = new java.util.ArrayList<>();
        for (MethodRegistry.MethodInfo info : registry.all()) {
            if (info.className.equals(className)) {
                result.add(info.toJson());
            }
        }
        if (result.isEmpty()) {
            respondError(exchange, 404, "not_registered", "类未在白名单注册：" + className);
            return;
        }
        respondJson(exchange, 200, Json.write(result));
    }

    private void handleInvoke(HttpExchange exchange) throws IOException {
        String body = readBody(exchange, config.policy.maxArgsBytes);
        if (body == null) {
            respondError(exchange, 413, "bad_arguments",
                    "请求体超过 " + config.policy.maxArgsBytes + " 字节上限");
            return;
        }
        Object parsed;
        try {
            parsed = Json.parse(body);
        } catch (Json.JsonParseException e) {
            respondError(exchange, 400, "bad_request", e.getMessage());
            return;
        }
        if (!(parsed instanceof Map)) {
            respondError(exchange, 400, "bad_request", "请求体必须是 JSON 对象");
            return;
        }
        Map<?, ?> request = (Map<?, ?>) parsed;
        String methodId = str(request.get("method_id"));
        if (methodId == null || methodId.isEmpty()) {
            respondError(exchange, 400, "bad_request", "缺少 method_id");
            return;
        }
        Object argsRaw = request.get("arguments");
        List<Object> args = argsRaw instanceof List ? (List<Object>) argsRaw : new java.util.ArrayList<>();
        UUID invocationId;
        Object idRaw = request.get("invocation_id");
        if (idRaw instanceof String) {
            try {
                invocationId = UUID.fromString((String) idRaw);
            } catch (IllegalArgumentException e) {
                invocationId = UUID.randomUUID();
            }
        } else {
            invocationId = UUID.randomUUID();
        }
        Long timeoutMs = null;
        Object timeoutRaw = request.get("timeout_ms");
        if (timeoutRaw instanceof Number) {
            timeoutMs = ((Number) timeoutRaw).longValue();
        }

        MethodRegistry.MethodInfo info = registry.find(methodId);
        if (info == null) {
            respondError(exchange, 404, "not_registered",
                    "方法未在白名单注册：" + methodId);
            return;
        }

        Map<String, Object> response;
        try {
            response = invocations.invoke(info, args, timeoutMs, invocationId);
        } catch (InvocationManager.BadArgumentsException e) {
            respondError(exchange, 400, "bad_arguments", e.getMessage());
            return;
        } catch (InvocationManager.RateLimitedException e) {
            respondError(exchange, 429, "rate_limited", e.getMessage());
            return;
        } catch (IllegalArgumentException e) {
            respondError(exchange, 400, "bad_arguments", e.getMessage());
            return;
        } catch (Exception e) {
            respondError(exchange, 500, "internal", "执行调度失败：" + e.getMessage());
            return;
        }
        respondJson(exchange, 200, Json.write(response));
    }

    private void handleCancel(HttpExchange exchange, String route) throws IOException {
        // /invocations/{id}/cancel
        String trimmed = route.substring("/invocations/".length());
        String idPart = trimmed.substring(0, trimmed.length() - "/cancel".length());
        UUID invocationId;
        try {
            invocationId = UUID.fromString(idPart);
        } catch (IllegalArgumentException e) {
            respondError(exchange, 400, "bad_request", "invocation id 不是合法 UUID：" + idPart);
            return;
        }
        boolean delivered = invocations.cancel(invocationId);
        respondJson(exchange, 200, Json.write(Map.of("delivered", delivered)));
    }

    // ------------------------------------------------------------------
    // 辅助
    // ------------------------------------------------------------------

    private static String str(Object value) {
        return value instanceof String ? (String) value : null;
    }

    /** 读请求体（带字节上限；超限返回 null）。 */
    private static String readBody(HttpExchange exchange, int maxBytes) throws IOException {
        try (InputStream in = exchange.getRequestBody()) {
            byte[] buffer = new byte[8192];
            java.io.ByteArrayOutputStream out = new java.io.ByteArrayOutputStream();
            int read;
            int total = 0;
            while ((read = in.read(buffer)) > 0) {
                total += read;
                if (total > maxBytes) {
                    return null;
                }
                out.write(buffer, 0, read);
            }
            return new String(out.toByteArray(), StandardCharsets.UTF_8);
        }
    }

    private static Map<String, String> queryParams(HttpExchange exchange) {
        Map<String, String> params = new LinkedHashMap<>();
        String raw = exchange.getRequestURI().getRawQuery();
        if (raw == null || raw.isEmpty()) {
            return params;
        }
        for (String pair : raw.split("&")) {
            int eq = pair.indexOf('=');
            if (eq < 0) {
                params.put(URLDecoder.decode(pair, StandardCharsets.UTF_8), "");
            } else {
                params.put(
                        URLDecoder.decode(pair.substring(0, eq), StandardCharsets.UTF_8),
                        URLDecoder.decode(pair.substring(eq + 1), StandardCharsets.UTF_8));
            }
        }
        return params;
    }

    /** Bearer 常数时间比较（长度先行，等长逐字节异或）。 */
    private static boolean constantTimeEquals(String provided, String expected) {
        if (provided.length() != expected.length()) {
            return false;
        }
        int diff = 0;
        for (int i = 0; i < provided.length(); i++) {
            diff |= provided.charAt(i) ^ expected.charAt(i);
        }
        return diff == 0;
    }

    private static void respondJson(HttpExchange exchange, int status, String json) throws IOException {
        respond(exchange, status, json);
    }

    private static void respondError(HttpExchange exchange, int status, String code, String message)
            throws IOException {
        Map<String, Object> error = new LinkedHashMap<>();
        error.put("code", code);
        error.put("message", message == null ? "" : message);
        Map<String, Object> body = new LinkedHashMap<>();
        body.put("error", error);
        respond(exchange, status, Json.write(body));
    }

    private static void respond(HttpExchange exchange, int status, String body) throws IOException {
        byte[] bytes = body.getBytes(StandardCharsets.UTF_8);
        exchange.getResponseHeaders().set("Content-Type", "application/json; charset=utf-8");
        exchange.sendResponseHeaders(status, bytes.length);
        try (OutputStream out = exchange.getResponseBody()) {
            out.write(bytes);
        }
    }
}
