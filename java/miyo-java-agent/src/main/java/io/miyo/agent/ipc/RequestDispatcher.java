package io.miyo.agent.ipc;

import java.util.LinkedHashMap;
import java.util.Map;
import java.util.concurrent.ExecutorService;

import io.miyo.agent.AgentState;
import io.miyo.agent.inspect.ClassClassifier;
import io.miyo.agent.inspect.ClassInspector;
import io.miyo.agent.invoke.InvocationEngine;
import io.miyo.agent.spring.ContextDiscovery;

/**
 * 请求分发：op → handler。invoke 类长操作投递 worker 池（连接线程保持
 * 可响应 cancel / status），其余同步执行。响应由完成线程直接写回
 *（IpcConnection 写侧有锁），id 关联——乱序完成不影响 Miyo 匹配。
 */
public final class RequestDispatcher {

    private final AgentState state;
    private final ClassInspector inspector;
    private final ContextDiscovery discovery;
    private final InvocationEngine engine;
    private final ExecutorService workers;

    public RequestDispatcher(AgentState state, ClassInspector inspector,
                             ContextDiscovery discovery, InvocationEngine engine) {
        this.state = state;
        this.inspector = inspector;
        this.discovery = discovery;
        this.engine = engine;
        this.workers = new io.miyo.agent.IpcWorkers(state.maxConcurrent() + 2).pool();
    }

    /**
     * 处理一帧请求（origin = 请求来源连接）。返回 null = 已投递 worker 池
     * （invoke 响应由完成线程写回 origin；连接已断则丢弃——调用方已有超时语义）。
     */
    Map<String, Object> dispatch(Map<String, Object> request, IpcConnection origin) {
        String id = Frames.asText(request.get("id"));
        String op = Frames.asText(request.get("op"));
        Map<String, Object> params = Frames.paramsOf(request);
        try {
            if ("invoke".equals(op)) {
                final Map<String, Object> fParams = params;
                final String fId = id;
                final IpcConnection fOrigin = origin;
                workers.execute(() -> {
                    Map<String, Object> response;
                    try {
                        response = Frames.ok(fId, engine.invoke(fParams));
                    } catch (IpcException e) {
                        response = Frames.error(fId, e.code(), e.getMessage());
                    } catch (Throwable t) {
                        response = Frames.error(fId, Protocol.ERR_INTERNAL,
                                "调用执行异常：" + t.getClass().getName()
                                        + (t.getMessage() != null ? ": " + t.getMessage() : ""));
                    }
                    writeResponse(fOrigin, response);
                });
                return null;
            }
            // 类发现 / 统计同样是「可能遍历上万类」的重操作：投递 worker 池，
            // 既避免占用目标 JVM 的业务线程，也让连接线程保持可响应（cancel/status）。
            if (isWorkerOp(op)) {
                final Map<String, Object> fParams = params;
                final String fId = id;
                final String fOp = op;
                final IpcConnection fOrigin = origin;
                workers.execute(() -> {
                    Map<String, Object> response;
                    try {
                        response = Frames.ok(fId, handle(fOp, fParams));
                    } catch (IpcException e) {
                        response = Frames.error(fId, e.code(), e.getMessage());
                    } catch (Throwable t) {
                        response = Frames.error(fId, Protocol.ERR_INTERNAL,
                                "请求处理异常：" + t.getClass().getName()
                                        + (t.getMessage() != null ? ": " + t.getMessage() : ""));
                    }
                    writeResponse(fOrigin, response);
                });
                return null;
            }
            return Frames.ok(id, handle(op, params));
        } catch (IpcException e) {
            return Frames.error(id, e.code(), e.getMessage());
        } catch (Throwable t) {
            return Frames.error(id, Protocol.ERR_INTERNAL,
                    "请求处理异常：" + t.getClass().getName()
                            + (t.getMessage() != null ? ": " + t.getMessage() : ""));
        }
    }

    /** 重操作 op（类索引刷新 / 统计 / 全量重建）：走 worker 池，不占连接线程。 */
    private static boolean isWorkerOp(String op) {
        return "classes.list".equals(op) || "classes.stats".equals(op)
                || "classes.reindex".equals(op);
    }

    private void writeResponse(IpcConnection connection, Map<String, Object> response) {
        try {
            connection.writeFrame(response);
        } catch (Exception e) {
            System.err.println("[miyo-agent] 响应写回失败: " + e.getMessage());
        }
    }

    private Map<String, Object> handle(String op, Map<String, Object> params) throws Exception {
        switch (op) {
            case "ping": {
                Map<String, Object> out = new LinkedHashMap<>();
                out.put("pong", System.currentTimeMillis());
                return out;
            }
            case "status":
                return status();
            case "classes.list":
                return inspector.listClasses(listQuery(params));
            case "classes.stats":
                return inspector.stats(businessPackagesOf(params));
            case "classes.reindex":
                return inspector.rebuildIndex(businessPackagesOf(params));
            case "class.detail":
                return inspector.classDetail(Frames.asText(params.get("class")));
            case "methods.list":
                return methodsList(params);
            case "beans.contexts":
                return discovery.status();
            case "cancel":
                return engine.cancel(Frames.asText(params.get("invocation_id")));
            case "beans.list":
                return discovery.listBeans(
                        Frames.asText(params.get("search")),
                        (int) longOr(params.get("limit"), 500),
                        Frames.asText(params.get("context_id")));
            case "agent.stop": {
                Map<String, Object> out = new LinkedHashMap<>();
                out.put("stopping", true);
                new Thread(() -> {
                    try {
                        Thread.sleep(200);
                    } catch (InterruptedException ignored) {
                        Thread.currentThread().interrupt();
                    }
                    io.miyo.agent.AgentMain.shutdown();
                }, "miyo-agent-stop").start();
                return out;
            }
            default:
                throw new IpcException(Protocol.ERR_BAD_REQUEST, "未知 op：" + op);
        }
    }

    /**
     * classes.list 参数解析（新旧字段并存）。
     *
     * 新：category / package_prefix / search / include_dependencies / include_jdk /
     *     limit / offset / all / refresh_index / business_packages / stats_only。
     * 旧：include / exclude / include_jdk（语义原样保留）。
     */
    /**
     * 业务包前缀参数（stats / reindex 共用）。
     *
     * 与 `classes.list` 的 `package_prefix`（**筛选**）不是一回事：这里是用户配置的
     * **分类弱证据**（`business_packages`）。两者混用会让 reindex 用筛选值重建索引。
     */
    private static String businessPackagesOf(Map<String, Object> params) {
        String v = Frames.asText(params.get("business_packages"));
        if (v.isEmpty()) {
            v = Frames.asText(params.get("businessPackages"));
        }
        return v;
    }

    private static ClassInspector.ListQuery listQuery(Map<String, Object> params) {
        ClassInspector.ListQuery q = new ClassInspector.ListQuery();
        q.search = Frames.asText(params.get("search"));
        q.include = Frames.asText(params.get("include"));
        q.exclude = Frames.asText(params.get("exclude"));
        q.packagePrefix = Frames.asText(params.get("package_prefix"));
        if (q.packagePrefix.isEmpty()) {
            q.packagePrefix = Frames.asText(params.get("packagePrefix"));
        }
        q.includeDependencies = truthy(params.get("include_dependencies"))
                || truthy(params.get("includeDependencies"));
        q.includeJdk = truthy(params.get("include_jdk")) || truthy(params.get("includeJdk"));
        q.all = truthy(params.get("all"));
        q.statsOnly = truthy(params.get("stats_only"));
        q.businessPackages = Frames.asText(params.get("business_packages"));
        if (q.businessPackages.isEmpty()) {
            q.businessPackages = Frames.asText(params.get("businessPackages"));
        }
        Object refresh = params.get("refresh_index");
        if (refresh == null) {
            refresh = params.get("refreshIndex");
        }
        q.refreshIndex = refresh == null || truthy(refresh);
        q.limit = (int) Math.min(longOr(params.get("limit"), 200), 1000);
        if (q.limit <= 0) {
            q.limit = 200;
        }
        q.offset = (int) Math.max(0, longOr(params.get("offset"), 0));
        Object category = params.get("category");
        java.util.Collection<String> raw = new java.util.ArrayList<String>();
        if (category instanceof java.util.Collection) {
            for (Object item : (java.util.Collection<?>) category) {
                raw.add(Frames.asText(item));
            }
        } else if (category != null) {
            for (String part : Frames.asText(category).split(",")) {
                raw.add(part);
            }
        }
        for (String item : raw) {
            ClassClassifier.Category parsed = ClassClassifier.Category.parse(item);
            if (parsed != null) {
                q.categories.add(parsed);
            }
        }
        return q;
    }

    private static boolean truthy(Object value) {
        if (value instanceof Boolean) {
            return (Boolean) value;
        }
        String text = Frames.asText(value).trim();
        return "true".equalsIgnoreCase(text) || "1".equals(text);
    }

    private Map<String, Object> methodsList(Map<String, Object> params) throws Exception {
        String className = Frames.asText(params.get("class"));
        if (className.trim().isEmpty()) {
            throw new IpcException(Protocol.ERR_BAD_ARGUMENTS, "class 不能为空");
        }
        if (io.miyo.agent.invoke.RiskPolicy.blacklisted(className)) {
            throw new IpcException(Protocol.ERR_NOT_ALLOWED,
                    "类命中黑名单，禁止查看 / 调用：" + className);
        }
        Map<String, Object> out = new LinkedHashMap<>();
        boolean includePrivate = Boolean.TRUE.equals(params.get("include_private"));
        out.put("class_name", className);
        out.put("methods", inspector.listMethods(className, includePrivate));
        // 实例解析能力提示（UI 据此决定调用前确认策略）
        Class<?> cls = inspector.findLoadedClass(className);
        out.put("can_instantiate", io.miyo.agent.invoke.InstanceResolver.canInstantiate(cls));
        out.put("proxy", io.miyo.agent.invoke.InstanceResolver.isProxy(cls));
        return out;
    }

    private Map<String, Object> status() {
        Map<String, Object> out = new LinkedHashMap<>();
        out.put("pid", AgentState.currentPid());
        out.put("vm_start_ms", AgentState.vmStartTime());
        out.put("uptime_ms", state.uptimeMs());
        out.put("requests_total", state.requestsTotal());
        out.put("in_flight", engine.inFlightCount());
        out.put("success_count", engine.successCount());
        out.put("failed_count", engine.failedCount());
        out.put("timeout_count", engine.timeoutCount());
        out.put("cancelled_count", engine.cancelledCount());
        out.put("stopped", state.isStopped());
        return out;
    }

    private static long longOr(Object value, long fallback) {
        Number n = Frames.asNumber(value);
        return n == null ? fallback : n.longValue();
    }
}
