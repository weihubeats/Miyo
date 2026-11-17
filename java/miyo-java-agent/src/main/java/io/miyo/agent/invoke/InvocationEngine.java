package io.miyo.agent.invoke;

import java.lang.reflect.InvocationTargetException;
import java.lang.reflect.Method;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.Semaphore;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicLong;

import io.miyo.agent.AgentState;
import io.miyo.agent.inspect.ClassInspector;
import io.miyo.agent.ipc.Frames;
import io.miyo.agent.ipc.IpcException;
import io.miyo.agent.ipc.Protocol;
import io.miyo.agent.util.Json;

/**
 * 受控反射调用引擎。
 *
 * 语义（必须诚实传达给 UI）：
 * - 超时 / 取消通过 Thread.interrupt 实现，是协作式的——业务代码不响应中断时
 *   线程会继续跑完（结果丢弃）；terminated 恒为 false（Java 无法强制终止
 *   同 JVM 内的业务线程）。
 * - 业务异常不是链路错误：failed 状态 + 结构化异常回传。
 * - 并发上限 / 超时上限 / 参数体积由 AgentState 策略收敛。
 *
 * 执行模型：invoke 请求在 dispatcher 的 worker 池执行，连接线程保持可响应
 * cancel / status 等短操作（否则长调用会阻塞取消通道）。
 */
public final class InvocationEngine {

    /** 单方法参数个数上限。 */
    private static final int MAX_PARAMS = 32;
    /** 参数 JSON 字符数上限（1 MiB）。 */
    private static final int MAX_ARGS_CHARS = 1_048_576;

    /** 在途调用控制块（取消 / 超时双标记，状态判定不依赖中断位——中断位会被业务代码清除）。 */
    private static final class Control {
        volatile boolean cancelRequested;
        volatile boolean timeoutTriggered;
        final Thread worker;

        Control(Thread worker) {
            this.worker = worker;
        }
    }

    private final ConcurrentHashMap<String, Control> activeCalls = new ConcurrentHashMap<>();
    /** 并发信号量（策略上限）。 */
    private final Semaphore semaphore;
    private final ClassInspector inspector;
    private final InstanceResolver resolver;
    private final AtomicLong cancelledCount = new AtomicLong();
    private final AtomicLong timeoutCount = new AtomicLong();
    private final AtomicLong successCount = new AtomicLong();
    private final AtomicLong failedCount = new AtomicLong();

    public InvocationEngine(ClassInspector inspector, InstanceResolver resolver,
                            AgentState state) {
        this.inspector = inspector;
        this.resolver = resolver;
        this.semaphore = new Semaphore(state.maxConcurrent());
    }

    /**
     * 执行一次方法调用（阻塞至完成或失败；由 dispatcher 放在 worker 线程上调用）。
     *
     * params: invocation_id, class, method, param_types[], args[](JSON 值数组),
     *         instance:{kind, bean_name?, consent?}, timeout_ms
     * 返回: invocation 响应（status/result/exception/duration…，形状与调试协议 v1 对齐）。
     */
    public Map<String, Object> invoke(Map<String, Object> params) {
        AgentState state = AgentState.current();
        String invocationId = Frames.asText(params.get("invocation_id"));
        if (invocationId.isEmpty()) {
            invocationId = java.util.UUID.randomUUID().toString();
        }
        String className = Frames.asText(params.get("class"));
        String methodName = Frames.asText(params.get("method"));
        if (className.isEmpty() || methodName.isEmpty()) {
            throw new IpcException(Protocol.ERR_BAD_ARGUMENTS, "class / method 不能为空");
        }
        long timeoutMs = 30_000L;
        Number timeoutNum = Frames.asNumber(params.get("timeout_ms"));
        if (timeoutNum != null && timeoutNum.longValue() > 0) {
            timeoutMs = timeoutNum.longValue();
        }
        if (timeoutMs > state.maxTimeoutMs()) {
            throw new IpcException(Protocol.ERR_RATE_LIMITED,
                    "超时 " + timeoutMs + "ms 超过上限 " + state.maxTimeoutMs() + "ms");
        }
        Object argsRaw = params.get("args");
        // 缺省 args = 空参（零参方法无需显式传 []）；给了就必须是数组
        List<Object> args = argsRaw == null
                ? new ArrayList<>()
                : requireList(argsRaw);
        if (args.size() > MAX_PARAMS) {
            throw new IpcException(Protocol.ERR_BAD_ARGUMENTS,
                    "参数个数 " + args.size() + " 超过上限 " + MAX_PARAMS);
        }
        String argsJson = Json.write(args);
        if (argsJson.length() > MAX_ARGS_CHARS) {
            throw new IpcException(Protocol.ERR_RATE_LIMITED,
                    "参数体积 " + argsJson.length() + " 字符超过上限 " + MAX_ARGS_CHARS);
        }

        // 目标方法定位（class + method + 擦除参数表；未给参数表时要求方法名唯一）
        Class<?> targetClass = inspector.findLoadedClass(className);
        String blocked = RiskPolicy.blockedTypeReason(targetClass);
        if (blocked != null) {
            throw new IpcException(Protocol.ERR_NOT_ALLOWED, blocked);
        }
        Method method = locateMethod(targetClass, methodName,
                asStringList(params.get("param_types")));

        @SuppressWarnings("unchecked")
        Map<String, Object> instanceSpec =
                params.get("instance") instanceof Map ? (Map<String, Object>) params.get("instance")
                        : new LinkedHashMap<String, Object>();
        String kind = Frames.asText(instanceSpec.get("kind"));
        if (kind.isEmpty()) {
            kind = java.lang.reflect.Modifier.isStatic(method.getModifiers())
                    ? InstanceResolver.KIND_STATIC : InstanceResolver.KIND_BEAN;
        }
        boolean consent = Boolean.TRUE.equals(instanceSpec.get("consent"));

        boolean acquired = false;
        try {
            acquired = semaphore.tryAcquire(3, TimeUnit.SECONDS);
            if (!acquired) {
                throw new IpcException(Protocol.ERR_RATE_LIMITED,
                        "并发调用达到上限（" + state.maxConcurrent() + "），请稍后重试");
            }
            state.incrementRequests();
            return execute(invocationId, targetClass, method, kind,
                    Frames.asText(instanceSpec.get("bean_name")), consent,
                    args, timeoutMs);
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            throw new IpcException(Protocol.ERR_RATE_LIMITED, "等待并发额度时被中断");
        } finally {
            if (acquired) {
                semaphore.release();
            }
        }
    }

    private Map<String, Object> execute(String invocationId, Class<?> targetClass,
                                        Method method, String kind, String beanName,
                                        boolean consent, List<Object> args, long timeoutMs) {
        Thread worker = Thread.currentThread();
        Control control = new Control(worker);
        activeCalls.put(invocationId, control);
        long startNanos = System.nanoTime();
        String startedAt = java.time.Instant.now().toString();
        Map<String, Object> out = new LinkedHashMap<>();
        out.put("invocation_id", invocationId);
        out.put("method_id", targetClass.getName() + "#" + method.getName()
                + paramTypesSuffix(method));
        java.util.Timer timer = null;
        try {
            InstanceResolver.Resolved resolved =
                    resolver.resolve(kind, targetClass, beanName, consent);
            method.setAccessible(true);
            Object[] javaArgs = convertArgs(method, args);
            // 超时 watchdog：到点打标 + 中断（协作式）
            timer = scheduleWatchdog(control, timeoutMs);
            try {
                long invokeStart = System.nanoTime();
                Object result = method.invoke(resolved.instance, javaArgs);
                long durationMs = (System.nanoTime() - invokeStart) / 1_000_000L;
                successCount.incrementAndGet();
                out.put("status", "success");
                out.put("duration_ms", durationMs);
                out.put("started_at", startedAt);
                out.put("finished_at", java.time.Instant.now().toString());
                out.put("instance_kind", resolved.kind);
                out.put("instance_type", resolved.actualType);
                if (resolved.beanName != null && !resolved.beanName.isEmpty()) {
                    out.put("bean_name", resolved.beanName);
                }
                out.put("terminated", false);
                serializeResult(out, result, method);
            } catch (InvocationTargetException e) {
                long durationMs = (System.nanoTime() - startNanos) / 1_000_000L;
                failedCount.incrementAndGet();
                // 业务异常 vs 中断（取消 / 超时把阻塞中的方法打断成
                // InvocationTargetException(InterruptedException)）：按标记分型
                out.put("status", statusOf(control));
                out.put("duration_ms", durationMs);
                out.put("started_at", startedAt);
                out.put("finished_at", java.time.Instant.now().toString());
                out.put("instance_kind", resolved.kind);
                out.put("instance_type", resolved.actualType);
                Throwable cause = e.getCause() != null ? e.getCause() : e;
                if (isInterrupt(cause)) {
                    out.put("exception", exceptionInfo(cause));
                } else {
                    out.put("exception", exceptionInfo(cause));
                }
                out.put("terminated", false);
                out.put("interrupt_sent", control.timeoutTriggered || control.cancelRequested);
            } finally {
                if (timer != null) {
                    timer.cancel();
                }
            }
        } catch (IpcException e) {
            throw e; // 链路级错误（实例解析失败 / 方法定位失败等）：dispatcher 归一
        } catch (IllegalAccessException e) {
            failedCount.incrementAndGet();
            throw new IpcException(Protocol.ERR_NOT_ALLOWED,
                    "方法不可访问（反射访问被拒绝）：" + e.getMessage());
        } catch (Throwable t) {
            // resolve / convertArgs / 序列化等准备阶段的意外错误
            failedCount.incrementAndGet();
            if (t instanceof InterruptedException || isInterrupt(t)) {
                timeoutCount.incrementAndGet();
                out.put("status", "timeout");
            } else {
                out.put("status", "failed");
            }
            out.put("duration_ms", (System.nanoTime() - startNanos) / 1_000_000L);
            out.put("started_at", startedAt);
            out.put("finished_at", java.time.Instant.now().toString());
            out.put("exception", exceptionInfo(t));
            out.put("terminated", false);
            out.put("interrupt_sent", control.timeoutTriggered || control.cancelRequested);
        } finally {
            activeCalls.remove(invocationId);
            if (timer != null) {
                timer.cancel();
            }
        }
        return out;
    }

    /** 状态判定：取消 > 超时 > failed（取消请求先于超时判定——用户意图优先）。 */
    private static String statusOf(Control control) {
        if (control.cancelRequested) {
            return "cancelled";
        }
        if (control.timeoutTriggered) {
            return "timeout";
        }
        return "failed";
    }

    private static boolean isInterrupt(Throwable t) {
        while (t != null) {
            if (t instanceof InterruptedException) {
                return true;
            }
            t = t.getCause();
        }
        return false;
    }

    /** 取消在途调用：置标记 + 线程中断（协作式；不保证业务方法停止）。 */
    public Map<String, Object> cancel(String invocationId) {
        Control control = activeCalls.get(invocationId);
        Map<String, Object> out = new LinkedHashMap<>();
        if (control == null) {
            out.put("delivered", false);
            out.put("terminated", false);
            out.put("reason", "调用已结束或不存在（in-flight 表无此 id）");
            return out;
        }
        control.cancelRequested = true;
        cancelledCount.incrementAndGet();
        control.worker.interrupt();
        out.put("delivered", true);
        out.put("terminated", false);
        out.put("note", "中断已发送；业务代码不响应中断时会继续执行到自然结束"
                + "（UI 必须区分「已请求取消」与「实际终止」）");
        return out;
    }

    private java.util.Timer scheduleWatchdog(final Control control, long timeoutMs) {
        if (timeoutMs <= 0) {
            return null;
        }
        final String marker = "wd-" + System.nanoTime();
        java.util.Timer timer = new java.util.Timer("miyo-agent-watchdog", true);
        timer.schedule(new java.util.TimerTask() {
            @Override
            public void run() {
                control.timeoutTriggered = true;
                timeoutCount.incrementAndGet();
                control.worker.interrupt();
            }
        }, timeoutMs);
        return timer;
    }

    /** 按形参类型转换 JSON 参数（泛型感知）。 */
    private Object[] convertArgs(Method method, List<Object> args) {
        Class<?>[] types = method.getParameterTypes();
        if (args.size() != types.length) {
            throw new IpcException(Protocol.ERR_BAD_ARGUMENTS,
                    "参数个数不匹配：期望 " + types.length + "，收到 " + args.size()
                            + "（method: " + method.getName() + "）");
        }
        java.lang.reflect.Type[] generics = method.getGenericParameterTypes();
        ArgsConverter converter = new ArgsConverter();
        Object[] out = new Object[types.length];
        for (int i = 0; i < types.length; i++) {
            try {
                out[i] = converter.convert(args.get(i), types[i],
                        generics.length == types.length ? generics[i] : null);
            } catch (IllegalArgumentException e) {
                throw new IpcException(Protocol.ERR_BAD_ARGUMENTS,
                        "第 " + (i + 1) + " 个参数转换失败：" + e.getMessage());
            }
        }
        return out;
    }

    /** 结果序列化（策略深度 + 字符上限；void 方法返回 null）。 */
    private void serializeResult(Map<String, Object> out, Object result, Method method) {
        AgentState state = AgentState.current();
        if (method.getReturnType() == void.class) {
            out.put("result", null);
            out.put("result_truncated", false);
            out.put("void", true);
            return;
        }
        ResultSerializer.Result serialized =
                ResultSerializer.serialize(result, state.maxDepth(), 1_048_576);
        out.put("result", serialized.value);
        out.put("result_truncated", serialized.truncated);
        out.put("void", false);
    }

    /** 定位方法：paramTypes 给定则精确匹配；否则要求方法名在类内唯一。 */
    private Method locateMethod(Class<?> targetClass, String methodName, List<String> paramTypes) {
        List<Method> candidates = new ArrayList<>();
        Class<?> cursor = targetClass;
        int depth = 0;
        while (cursor != null && cursor != Object.class && depth < 12) {
            for (Method m : cursor.getDeclaredMethods()) {
                if (m.isSynthetic() || !m.getName().equals(methodName)) {
                    continue;
                }
                candidates.add(m);
            }
            cursor = cursor.getSuperclass();
            depth++;
        }
        if (candidates.isEmpty()) {
            throw new IpcException(Protocol.ERR_NOT_FOUND,
                    "方法不存在（含继承链）： " + targetClass.getName() + "#" + methodName);
        }
        if (paramTypes != null && !paramTypes.isEmpty()) {
            for (Method m : candidates) {
                Class<?>[] types = m.getParameterTypes();
                if (types.length == paramTypes.size()) {
                    boolean match = true;
                    for (int i = 0; i < types.length; i++) {
                        if (!types[i].getName().equals(paramTypes.get(i))) {
                            match = false;
                            break;
                        }
                    }
                    if (match) {
                        return m;
                    }
                }
            }
            throw new IpcException(Protocol.ERR_NOT_FOUND,
                    "重载方法无法按参数表定位：" + methodName + "(" + paramTypes + ")"
                            + "；候选 " + candidates.size() + " 个");
        }
        if (candidates.size() > 1) {
            throw new IpcException(Protocol.ERR_BAD_ARGUMENTS,
                    "方法存在 " + candidates.size() + " 个重载，必须提供 param_types 精确匹配");
        }
        return candidates.get(0);
    }

    @SuppressWarnings("unchecked")
    private static List<Object> requireList(Object raw) {
        if (!(raw instanceof List)) {
            throw new IpcException(Protocol.ERR_BAD_ARGUMENTS, "args 必须是 JSON 数组");
        }
        return (List<Object>) raw;
    }

    private static List<String> asStringList(Object raw) {
        if (!(raw instanceof List)) {
            return null;
        }
        List<?> list = (List<?>) raw;
        List<String> out = new ArrayList<>();
        for (Object item : list) {
            out.add(Frames.asText(item));
        }
        return out;
    }

    private static String paramTypesSuffix(Method m) {
        Class<?>[] types = m.getParameterTypes();
        StringBuilder sb = new StringBuilder();
        sb.append('(');
        for (int i = 0; i < types.length; i++) {
            if (i > 0) {
                sb.append(',');
            }
            sb.append(types[i].getName());
        }
        sb.append(')');
        return sb.toString();
    }

    /** 异常结构化（类型 / 消息 / 堆栈 50 帧 / cause 链 10 级）。 */
    private static Map<String, Object> exceptionInfo(Throwable t) {
        Map<String, Object> out = new LinkedHashMap<>();
        out.put("exception_type", t.getClass().getName());
        out.put("message", t.getMessage());
        List<String> frames = new ArrayList<>();
        StackTraceElement[] stack = t.getStackTrace();
        int limit = Math.min(stack.length, 50);
        for (int i = 0; i < limit; i++) {
            frames.add(stack[i].toString());
        }
        out.put("stack_trace", frames);
        out.put("stack_trace_truncated", stack.length > limit);
        List<String> causes = new ArrayList<>();
        Throwable cause = t.getCause();
        int chain = 0;
        while (cause != null && chain < 10) {
            causes.add(cause.getClass().getName() + ": " + cause.getMessage());
            cause = cause.getCause();
            chain++;
        }
        out.put("cause_chain", causes);
        return out;
    }

    // 统计（agent.status 展示）
    public long successCount() {
        return successCount.get();
    }

    public long failedCount() {
        return failedCount.get();
    }

    public long timeoutCount() {
        return timeoutCount.get();
    }

    public long cancelledCount() {
        return cancelledCount.get();
    }

    public int inFlightCount() {
        return activeCalls.size();
    }
}
