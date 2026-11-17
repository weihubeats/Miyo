package io.miyo.agent;

import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.atomic.AtomicInteger;

/**
 * Agent worker 线程池持有者（独立小类避免 RequestDispatcher 依赖膨胀）。
 * 线程全部 daemon：不阻碍目标 JVM 正常退出。
 */
public final class IpcWorkers {

    private final ExecutorService pool;

    public IpcWorkers(int size) {
        final AtomicInteger seq = new AtomicInteger();
        this.pool = Executors.newFixedThreadPool(size, r -> {
            Thread t = new Thread(r, "miyo-agent-worker-" + seq.incrementAndGet());
            t.setDaemon(true);
            return t;
        });
    }

    public ExecutorService pool() {
        return pool;
    }
}
