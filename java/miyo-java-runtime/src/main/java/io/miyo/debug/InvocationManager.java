package io.miyo.debug;

import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.UUID;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.Semaphore;
import java.util.concurrent.atomic.AtomicLong;

/**
 * 调用执行管理：并发信号量 + 分钟级滑动限流 + 超时 + 协作式取消。
 *
 * 超时语义（重要）：{@code Future.get(timeout)} 到点后以
 * {@code cancel(true)} 发出线程中断——Java 方法执行无法保证强制终止，
 * 业务代码不响应中断时线程仍会跑完（此时结果丢弃、调用按 timeout 返回）。
 * 需要强制终止 / 执行不可信代码的场景应使用独立进程隔离 + 杀进程回收
 *（Miyo 后续阶段能力），Runtime 本身不做进程级隔离。
 */
final class InvocationManager {

    /** 一次在途调用的追踪句柄。 */
    static final class InFlight {
        final UUID invocationId;
        final java.util.concurrent.Future<?> future;

        InFlight(UUID invocationId, java.util.concurrent.Future<?> future) {
            this.invocationId = invocationId;
            this.future = future;
        }
    }

    private final SecurityPolicy policy;
    private final Semaphore concurrency;
    private final java.util.concurrent.ExecutorService executor;
    private final ConcurrentHashMap<UUID, InFlight> inFlight = new ConcurrentHashMap<>();
    private final AtomicLong windowStart = new AtomicLong(System.currentTimeMillis());
    private final AtomicLong windowCount = new AtomicLong();

    InvocationManager(SecurityPolicy policy) {
        this.policy = policy;
        this.concurrency = new Semaphore(policy.maxConcurrent);
        this.executor = java.util.concurrent.Executors.newCachedThreadPool(r -> {
            Thread thread = new Thread(r, "miyo-debug-invoke");
            thread.setDaemon(true);
            return thread;
        });
    }

    /** 分钟级限流（超过每分钟上限返回 false）。 */
    synchronized boolean tryAcquireRate() {
        long now = System.currentTimeMillis();
        long start = windowStart.get();
        if (now - start >= 60_000L) {
            windowStart.set(now);
            windowCount.set(0);
            return true;
        }
        return windowCount.incrementAndGet() <= policy.maxCallsPerMinute;
    }

    /** 取消在途调用（协作式中断；返回是否找到在途任务）。 */
    boolean cancel(UUID invocationId) {
        InFlight task = inFlight.remove(invocationId);
        if (task == null) {
            return false;
        }
        task.future.cancel(true);
        return true;
    }

    /**
     * 执行一次方法调用（同步阻塞直至完成 / 超时 / 被取消）。
     *
     * @param info       白名单方法
     * @param rawArgs    JSON 参数数组（协议形状）
     * @param timeoutMs  本次调用超时（毫秒；null 用上限一半）
     * @return 响应 JSON（协议 InvokeResponse 形状，由 HttpApi 包装）
     */
    Map<String, Object> invoke(MethodRegistry.MethodInfo info, List<Object> rawArgs,
                               Long timeoutMs, UUID invocationId) throws Exception {
        if (!tryAcquireRate()) {
            throw new RateLimitedException("调用频率超过每分钟 " + policy.maxCallsPerMinute + " 次上限");
        }
        if (rawArgs.size() != info.method.getParameterCount()) {
            throw new BadArgumentsException("参数个数不匹配：期望 "
                    + info.method.getParameterCount() + "，实际 " + rawArgs.size());
        }
        if (!concurrency.tryAcquire()) {
            throw new RateLimitedException("并发调用已达上限 " + policy.maxConcurrent);
        }
        try {
            return executeWithTimeout(info, rawArgs, timeoutMs, invocationId);
        } finally {
            concurrency.release();
        }
    }

    private Map<String, Object> executeWithTimeout(MethodRegistry.MethodInfo info,
                                                   List<Object> rawArgs, Long timeoutMs,
                                                   UUID invocationId)
            throws Exception {
        String startedAt = Timestamps.now();
        long effectiveTimeout = timeoutMs == null
                ? policy.maxTimeoutMs / 2
                : Math.min(timeoutMs, policy.maxTimeoutMs);

        ArgsConverter converter = new ArgsConverter();
        Object[] args = new Object[rawArgs.size()];
        for (int i = 0; i < rawArgs.size(); i++) {
            args[i] = converter.convert(rawArgs.get(i), info.method.getParameterTypes()[i]);
        }

        java.util.concurrent.Callable<Object> task = () -> {
            Object instance = info.registration.provider.resolve();
            if (instance == null && !info.method.getDeclaringClass().isEnum()) {
                boolean isStatic = info.staticMethod;
                if (!isStatic && instance == null) {
                    // provider 显式返回 null = 静态执行；非静态方法无实例即错误。
                    if (!java.lang.reflect.Modifier.isStatic(info.method.getModifiers())) {
                        throw new IllegalStateException("实例提供器返回 null（非静态方法）");
                    }
                }
            }
            try {
                return info.method.invoke(instance, args);
            } catch (java.lang.reflect.InvocationTargetException e) {
                Throwable cause = e.getCause() != null ? e.getCause() : e;
                if (cause instanceof Exception) {
                    throw (Exception) cause;
                }
                throw (Error) cause; // 非 Exception 的 Throwable（Error）无需声明
            }
        };

        java.util.concurrent.Future<Object> future = executor.submit(task);
        inFlight.put(invocationId, new InFlight(invocationId, future));
        long wallStart = System.nanoTime();
        Map<String, Object> response;
        try {
            Object result;
            try {
                result = future.get(effectiveTimeout, java.util.concurrent.TimeUnit.MILLISECONDS);
            } catch (java.util.concurrent.TimeoutException e) {
                future.cancel(true);
                response = baseResponse(invocationId, info, startedAt, Timestamps.now());
                response.put("status", "timeout");
                response.put("exception", exceptionJson(new TimeoutError(
                        "方法执行超过 " + effectiveTimeout + "ms（已发出线程中断；"
                                + "不响应中断的方法仍会执行完——协作式语义）")));
                return response;
            } catch (java.util.concurrent.CancellationException e) {
                response = baseResponse(invocationId, info, startedAt, Timestamps.now());
                response.put("status", "cancelled");
                return response;
            }
            long wallMs = (long) ((System.nanoTime() - wallStart) / 1_000_000.0);
            ResultSerializer.Result serialized =
                    ResultSerializer.serialize(result, 12, policy.maxResultBytes);
            response = baseResponse(invocationId, info, startedAt, Timestamps.now());
            response.put("status", "success");
            response.put("result", serialized.value);
            response.put("result_truncated", serialized.truncated);
            response.put("duration_ms", wallMs);
            return response;
        } catch (Throwable thrown) {
            // 业务异常：不吞、全量结构化回传（类型 + 消息 + 堆栈 + cause 链）。
            // Future.get 把 Callable 抛出的异常包成 ExecutionException——解包
            // 取真实 cause（ExecutionException 自身无 cause 时原样使用）。
            Throwable real = thrown instanceof java.util.concurrent.ExecutionException
                    && thrown.getCause() != null ? thrown.getCause() : thrown;
            long wallMs = (long) ((System.nanoTime() - wallStart) / 1_000_000.0);
            response = baseResponse(invocationId, info, startedAt, Timestamps.now());
            response.put("status", "failed");
            response.put("exception", exceptionJson(real));
            response.put("duration_ms", wallMs);
            return response;
        } finally {
            inFlight.remove(invocationId);
        }
    }

    private Map<String, Object> baseResponse(UUID invocationId, MethodRegistry.MethodInfo info,
                                             String startedAt, String finishedAt) {
        Map<String, Object> response = new LinkedHashMap<>();
        response.put("invocation_id", invocationId.toString());
        response.put("method_id", info.methodId);
        response.put("result", null);
        response.put("result_truncated", false);
        response.put("exception", null);
        response.put("started_at", startedAt);
        response.put("finished_at", finishedAt);
        return response;
    }

    /** 异常结构化（协议 ExceptionInfo 形状；堆栈 / cause 链有界）。 */
    static Map<String, Object> exceptionJson(Throwable thrown) {
        Map<String, Object> exception = new LinkedHashMap<>();
        exception.put("exception_type", thrown.getClass().getName());
        exception.put("message", thrown.getMessage());
        List<String> frames = new ArrayList<>();
        StackTraceElement[] stack = thrown.getStackTrace();
        int cap = Math.min(stack.length, 50);
        for (int i = 0; i < cap; i++) {
            frames.add(stack[i].toString());
        }
        exception.put("stack_trace", frames);
        List<String> causes = new ArrayList<>();
        Throwable cause = thrown.getCause();
        int causeCap = 0;
        while (cause != null && causeCap < 10) {
            StringBuilder line = new StringBuilder("Caused by: ")
                    .append(cause.getClass().getName());
            if (cause.getMessage() != null) {
                line.append(": ").append(cause.getMessage());
            }
            causes.add(line.toString());
            cause = cause.getCause();
            causeCap++;
        }
        exception.put("cause_chain", causes);
        return exception;
    }

    /** 超时占位异常（TimeoutException 不带消息上下文，用语义化文案）。 */
    private static final class TimeoutError extends RuntimeException {
        TimeoutError(String message) {
            super(message);
        }
    }

    static final class BadArgumentsException extends RuntimeException {
        BadArgumentsException(String message) {
            super(message);
        }
    }

    static final class RateLimitedException extends RuntimeException {
        RateLimitedException(String message) {
            super(message);
        }
    }

    /** RFC3339 时间戳（UTC）。 */
    static final class Timestamps {
        private Timestamps() {
        }

        static String now() {
            return java.time.Instant.now().toString();
        }
    }
}
