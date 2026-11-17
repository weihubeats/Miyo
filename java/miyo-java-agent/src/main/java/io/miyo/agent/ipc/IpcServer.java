package io.miyo.agent.ipc;

import java.io.IOException;
import java.net.InetAddress;
import java.net.ServerSocket;
import java.net.Socket;
import java.util.Collections;
import java.util.Set;
import java.util.concurrent.ConcurrentHashMap;

import io.miyo.agent.AgentState;
import io.miyo.agent.AgentVersion;

/**
 * Agent IPC 服务：仅绑定 127.0.0.1 的随机端口，同一时刻至多一条活跃连接
 *（新连接替换旧连接——陈旧 / 失联的旧 Miyo 会话不该阻塞重连）。
 *
 * 线程模型：accept 守护线程 + 单连接处理线程（new 请求内部分发到 worker 池）。
 */
public final class IpcServer {

    /** 并发连接上限（Miyo 主会话 + 探测连接 + 余量；客户端断线残留不阻塞新连接）。 */
    static final int MAX_CONNECTIONS = 8;

    private final RequestDispatcher dispatcher;
    private ServerSocket serverSocket;
    private Thread acceptThread;
    private final Set<IpcConnection> connections = Collections.newSetFromMap(
            new ConcurrentHashMap<>());
    private volatile boolean started;
    private volatile boolean stopped;

    public IpcServer(RequestDispatcher dispatcher) {
        this.dispatcher = dispatcher;
    }

    /**
     * 绑定并开始 accept（后台线程）。返回绑定端口。
     * 仅接受回环连接：绑定 127.0.0.1，并逐连接校验远端地址。
     */
    public int start() throws IOException {
        ServerSocket socket = new ServerSocket(0, 4, InetAddress.getByName("127.0.0.1"));
        this.serverSocket = socket;
        AgentState.IpcPortHolder.port = socket.getLocalPort();
        this.started = true;
        this.acceptThread = new Thread(this::acceptLoop, "miyo-agent-ipc-accept");
        this.acceptThread.setDaemon(true);
        this.acceptThread.start();
        return socket.getLocalPort();
    }

    private void acceptLoop() {
        while (!stopped) {
            try {
                Socket socket = serverSocket.accept();
                // 双保险：回环外地址一律拒绝（绑定已限定，防御代理转发等场景）。
                InetAddress remote = socket.getInetAddress();
                if (remote != null && !remote.isLoopbackAddress()) {
                    try {
                        socket.close();
                    } catch (IOException ignored) {
                        // 忽略关闭失败
                    }
                    continue;
                }
                IpcConnection connection;
                try {
                    connection = new IpcConnection(socket, dispatcher);
                } catch (IOException e) {
                    try {
                        socket.close();
                    } catch (IOException ignored) {
                        // 忽略关闭失败
                    }
                    continue;
                }
                // 并发连接各自独立（鉴权 / 生命周期互不影响）；超限拒绝新连接。
                if (connections.size() >= MAX_CONNECTIONS) {
                    System.err.println("[miyo-agent] 连接数达到上限，拒绝新连接");
                    try {
                        socket.close();
                    } catch (IOException ignored) {
                        // 忽略关闭失败
                    }
                    continue;
                }
                connections.add(connection);
                Thread handler = new Thread(() -> {
                    try {
                        connection.serve();
                    } finally {
                        connections.remove(connection);
                        connection.close();
                    }
                }, "miyo-agent-ipc-conn");
                handler.setDaemon(true);
                handler.start();
            } catch (IOException e) {
                if (stopped) {
                    return; // 正常停机
                }
                System.err.println("[miyo-agent] accept 异常: " + e.getMessage());
            }
        }
    }

    /** 停机：停止 accept、断开全部活跃连接、关闭监听。 */
    public void stop() {
        stopped = true;
        started = false;
        for (IpcConnection connection : connections) {
            connection.close();
        }
        connections.clear();
        try {
            if (serverSocket != null) {
                serverSocket.close();
            }
        } catch (IOException ignored) {
            // 忽略关闭失败
        }
        if (acceptThread != null) {
            acceptThread.interrupt();
        }
    }

    public boolean isStarted() {
        return started;
    }
}
