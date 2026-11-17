package io.miyo.agent.ipc;

/**
 * 协议级业务异常：handler 抛出 → dispatcher 归一为错误响应帧。
 * {@link #code} 取 Protocol.ERR_*；message 面向用户（中文）。
 */
public final class IpcException extends RuntimeException {

    private static final long serialVersionUID = 1L;

    private final String code;

    public IpcException(String code, String message) {
        super(message);
        this.code = code;
    }

    public IpcException(String code, String message, Throwable cause) {
        super(message, cause);
        this.code = code;
    }

    public String code() {
        return code;
    }
}
