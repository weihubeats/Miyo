package io.miyo.agent.ipc;

import java.io.BufferedOutputStream;
import java.io.DataInputStream;
import java.io.DataOutputStream;
import java.io.EOFException;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.net.Socket;
import java.nio.charset.StandardCharsets;
import java.util.LinkedHashMap;
import java.util.Map;

import io.miyo.agent.AgentState;
import io.miyo.agent.AgentVersion;
import io.miyo.agent.util.Json;

/**
 * 单条 IPC 连接：帧读写 + 鉴权握手 + 请求分发循环。
 *
 * 一条连接同一时刻串行处理请求（Miyo 侧单会话顺序发请求）；
 * invoke 类长操作在 {@link RequestDispatcher} 的 worker 池执行。
 */
final class IpcConnection {

    private final Socket socket;
    private final DataInputStream in;
    private final DataOutputStream out;
    private final RequestDispatcher dispatcher;
    private volatile boolean authorized;

    IpcConnection(Socket socket, RequestDispatcher dispatcher) throws IOException {
        socket.setTcpNoDelay(true);
        this.socket = socket;
        this.in = new DataInputStream(socket.getInputStream());
        this.out = new DataOutputStream(new BufferedOutputStream(socket.getOutputStream()));
        this.dispatcher = dispatcher;
    }

    /** 握手 + 服务循环（阻塞到连接结束）。返回连接是否经过合法握手。 */
    void serve() {
        try {
            socket.setSoTimeout(AgentVersion.HANDSHAKE_TIMEOUT_MS);
            handshake();
            socket.setSoTimeout(0); // 握手后取消读超时（长调用靠应用层取消）
            while (!AgentState.current().isStopped()) {
                Map<String, Object> request = readFrame();
                if (request == null) {
                    return;
                }
                Map<String, Object> response = dispatcher.dispatch(request, this);
                if (response == null) {
                    continue; // invoke 类：worker 池异步写回
                }
                writeFrame(response);
                if ("agent.stop".equals(request.get("op"))) {
                    // 停止指令：响应送出后延迟闭合，让字节真正落地。
                    try {
                        out.flush();
                        Thread.sleep(150);
                    } catch (InterruptedException ignored) {
                        Thread.currentThread().interrupt();
                    }
                    return;
                }
            }
        } catch (EOFException | java.net.SocketTimeoutException e) {
            // 对端断开 / 握手超时：静默收线。
        } catch (IOException e) {
            if (!socket.isClosed()) {
                // 网络级失败：日志到 stderr（不打扰目标应用 stdout）。
                System.err.println("[miyo-agent] IPC 连接异常: " + e.getMessage());
            }
        } catch (Throwable t) {
            // 兜底：处理循环内的意外错误必须留痕（否则连接静默死亡无法诊断）
            System.err.println("[miyo-agent] 连接处理异常: " + t);
            t.printStackTrace();
        }
    }

    boolean isAuthorized() {
        return authorized;
    }

    /** 握手：首帧必须携带正确 token；版本协商失败即拒绝。 */
    private void handshake() throws IOException {
        Map<String, Object> request = readFrame();
        if (request == null) {
            throw new EOFException("握手前连接关闭");
        }
        String op = String.valueOf(request.get("op"));
        Map<String, Object> params = Frames.paramsOf(request);
        String id = String.valueOf(request.get("id"));

        if (!"handshake".equals(op)) {
            writeError(id, Protocol.ERR_BAD_REQUEST, "首帧必须是 handshake");
            throw new IOException("首帧非 handshake");
        }
        String token = String.valueOf(params.get("token"));
        if (!AgentState.current().tokenMatches(token)) {
            writeError(id, Protocol.ERR_NOT_AUTHORIZED, "token 不匹配或缺失");
            throw new IOException("token 不匹配");
        }
        Number clientProtocol = Frames.asNumber(params.get("client_protocol"));
        int clientVersion = clientProtocol == null ? 0 : clientProtocol.intValue();
        if (clientVersion != AgentVersion.PROTOCOL_VERSION) {
            writeError(id, Protocol.ERR_PROTOCOL_MISMATCH,
                    "协议版本不匹配：Miyo v" + clientVersion + "，Agent v"
                            + AgentVersion.PROTOCOL_VERSION + "（请同步升级）");
            throw new IOException("协议版本不匹配");
        }
        authorized = true;

        Map<String, Object> data = new LinkedHashMap<>();
        data.put("protocol", AgentVersion.PROTOCOL_VERSION);
        data.put("agent_version", AgentVersion.VERSION);
        data.put("pid", AgentState.currentPid());
        data.put("vm_start_ms", AgentState.vmStartTime());
        data.put("java_version", System.getProperty("java.version", ""));
        data.put("java_vm_version", System.getProperty("java.vm.version", ""));
        data.put("jvm_vendor", System.getProperty("java.vendor", ""));
        data.put("java_home", System.getProperty("java.home", ""));
        data.put("max_concurrent", AgentState.current().maxConcurrent());
        data.put("max_timeout_ms", AgentState.current().maxTimeoutMs());
        data.put("uptime_ms", AgentState.current().uptimeMs());
        writeFrame(Frames.ok(id, data));
    }

    /** 读一帧（长度前缀 + JSON）；断开返回 null；超限抛异常断链。 */
    @SuppressWarnings("unchecked")
    private Map<String, Object> readFrame() throws IOException {
        int length = in.readInt();
        if (length <= 0 || length > AgentVersion.MAX_FRAME_BYTES) {
            throw new IOException("帧长度非法: " + length);
        }
        byte[] bytes = new byte[length];
        in.readFully(bytes);
        Object parsed = Json.parse(new String(bytes, StandardCharsets.UTF_8));
        if (!(parsed instanceof Map)) {
            throw new IOException("帧必须是 JSON 对象");
        }
        return (Map<String, Object>) parsed;
    }

    /** 写帧（invoke 响应来自 worker 线程，写侧加锁防交错）。 */
    void writeFrame(Map<String, Object> frame) throws IOException {
        byte[] bytes = Json.write(frame).getBytes(StandardCharsets.UTF_8);
        if (bytes.length > AgentVersion.MAX_FRAME_BYTES) {
            // 防御：理论上 dispatcher 已截断；到这里直接以错误帧替代。
            Map<String, Object> err = Frames.error(Frames.asText(frame.get("id")), Protocol.ERR_INTERNAL,
                    "响应帧超限（" + bytes.length + " 字节）");
            bytes = Json.write(err).getBytes(StandardCharsets.UTF_8);
        }
        synchronized (writeLock) {
            out.writeInt(bytes.length);
            out.write(bytes);
            out.flush();
        }
    }

    private final Object writeLock = new Object();

    private void writeError(String id, String code, String message) throws IOException {
        writeFrame(Frames.error(id, code, message));
    }


    void close() {
        try {
            socket.close();
        } catch (IOException ignored) {
            // 关闭失败无害
        }
    }
}
