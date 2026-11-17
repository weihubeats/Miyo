#!/usr/bin/env python3
"""Miyo Java Agent 协议冒烟客户端（独立验证脚本；Rust 客户端实现前的协议基准）。

用法: python3 agent_smoke.py <port> <token>
覆盖: handshake / classes.list / methods.list / invoke(静态+异常+慢方法超时) /
      cancel / 重复 attach（换发 token）/ agent.stop
"""
import json
import socket
import struct
import subprocess
import sys
import time

import os

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(
    os.path.abspath(__file__)))))
JAR = os.path.join(REPO, "java/miyo-attach-launcher/target/miyo-attach-launcher.jar")
AGENT = os.path.join(REPO, "java/miyo-java-agent/target/miyo-java-agent.jar")
INFO_FILE = "/tmp/miyo-agent-info-smoke.json"


def read_frame(sock):
    header = b""
    while len(header) < 4:
        chunk = sock.recv(4 - len(header))
        if not chunk:
            raise EOFError("connection closed")
        header += chunk
    (length,) = struct.unpack(">I", header)
    data = b""
    while len(data) < length:
        chunk = sock.recv(min(65536, length - len(data)))
        if not chunk:
            raise EOFError("connection closed mid-frame")
        data += chunk
    return json.loads(data.decode("utf-8"))


def write_frame(sock, obj):
    data = json.dumps(obj).encode("utf-8")
    sock.sendall(struct.pack(">I", len(data)) + data)


import collections
_PENDING = collections.defaultdict(collections.deque)  # id → deque of frames


def call(sock, op, params=None, request_id=None):
    request_id = request_id or f"r-{time.time()}"
    write_frame(sock, {"id": request_id, "op": op, "params": params or {}})
    # 先查缓存（早前到达的响应）
    if _PENDING[request_id]:
        return _PENDING[request_id].popleft()
    while True:
        resp = read_frame(sock)
        if resp.get("id") == request_id:
            return resp
        _PENDING[resp.get("id")].append(resp)  # 异步响应（如 invoke）缓存等待领取


def check(name, cond, detail=""):
    status = "✅" if cond else "❌"
    print(f"{status} {name}" + (f" — {detail}" if detail and not cond else ""))
    if not cond:
        raise SystemExit(f"FAILED: {name} {detail}")


def attach(pid, token):
    subprocess.run(
        ["java", "-jar", JAR, "attach", "--pid", str(pid), "--agent", AGENT,
         "--args", f"token={token},infoFile={INFO_FILE},expectedPid={pid}",
         "--info-file", INFO_FILE, "--timeout-ms", "20000"],
        check=True, capture_output=True)
    with open(INFO_FILE) as f:
        info = json.load(f)
    assert info["ok"], info
    return info


def main():
    pid = int(sys.argv[1])
    token = sys.argv[2]

    info = attach(pid, token)
    check("attach: infoFile 写出", info["ok"] and info["pid"] == pid,
          json.dumps(info))
    port = info["port"]

    sock = socket.create_connection(("127.0.0.1", port), timeout=10)
    resp = call(sock, "handshake", {"token": "WRONG", "client_protocol": 2})
    check("鉴权: 错误 token 被拒", resp["error"]["code"] == "not_authorized")
    sock.close()

    sock = socket.create_connection(("127.0.0.1", port), timeout=10)
    resp = call(sock, "handshake", {"token": token, "client_protocol": 2})
    check("握手: 正确 token + 协议 v2",
          resp.get("ok") and resp["data"]["protocol"] == 2
          and resp["data"]["pid"] == pid,
          json.dumps(resp))
    check("握手: JVM 信息回传", resp["data"].get("java_version", "") != "")

    # 版本协商失败
    sock2 = socket.create_connection(("127.0.0.1", port), timeout=10)
    resp = call(sock2, "handshake", {"token": token, "client_protocol": 1})
    check("版本协商: v1 客户端被拒", resp["error"]["code"] == "protocol_mismatch")
    sock2.close()

    # 类清单
    resp = call(sock, "classes.list", {"search": "com.example", "limit": 100})
    names = [c["name"] for c in resp["data"]["items"]]
    check("类发现: com.example 类可见", "com.example.order.OrderService" in names, str(names))
    check("类发现: JDK 类默认排除",
          not any(n.startswith("java.") for n in names), str(names[:5]))

    # 方法清单
    resp = call(sock, "methods.list", {"class": "com.example.calculator.Calculator"})
    methods = resp["data"]["methods"]
    check("方法发现: Calculator 方法列出", len(methods) >= 6, str(len(methods)))
    greet_overloads = [m for m in methods if m["method_name"] == "greet"]
    check("方法发现: 重载标记", len(greet_overloads) == 2
          and all(m["overloaded"] for m in greet_overloads))
    fizz = [m for m in methods if m["method_name"] == "fizzBuzz"][0]
    check("方法发现: 泛型签名", fizz["return_generic"] == "java.util.List<java.lang.String>",
          fizz["return_generic"])

    # 黑名单类拒绝
    resp = call(sock, "methods.list", {"class": "java.lang.Runtime"})
    check("安全: Runtime 黑名单", resp["error"]["code"] == "not_allowed")

    # 静态方法调用
    resp = call(sock, "invoke", {
        "class": "com.example.calculator.Calculator", "method": "add",
        "param_types": ["long", "long"], "args": [2, 40],
        "instance": {"kind": "static"}, "timeout_ms": 10000})
    check("调用: 静态 add(2,40)=42", resp["ok"] and resp["data"]["status"] == "success"
          and resp["data"]["result"] == 42, json.dumps(resp))
    check("调用: 耗时字段", resp["data"]["duration_ms"] >= 0)

    # 嵌套 DTO + 枚举调用（新建实例需 consent）
    resp = call(sock, "invoke", {
        "class": "com.example.order.OrderService", "method": "createOrder",
        "param_types": ["com.example.order.CreateOrderRequest"],
        "args": [{"customerName": "冒烟客户", "channel": "RETAIL", "items":
                  [{"sku": "S1", "productName": "商品", "unitPrice": 500, "quantity": 3}]}],
        "instance": {"kind": "new"}, "timeout_ms": 10000})
    check("安全: 新建实例无 consent 被拒",
          resp["error"]["code"] == "consent_required", json.dumps(resp))

    resp = call(sock, "invoke", {
        "class": "com.example.order.OrderService", "method": "createOrder",
        "param_types": ["com.example.order.CreateOrderRequest"],
        "args": [{"customerName": "冒烟客户", "channel": "RETAIL", "items":
                  [{"sku": "S1", "productName": "商品", "unitPrice": 500, "quantity": 3}]}],
        "instance": {"kind": "new", "consent": True}, "timeout_ms": 10000})
    check("调用: 新建实例 + DTO 注入",
          resp["ok"] and resp["data"]["status"] == "success"
          and resp["data"]["result"]["orderNo"].startswith("SO-")
          and resp["data"]["result"]["total"] == 1500, json.dumps(resp))

    # 业务异常
    resp = call(sock, "invoke", {
        "class": "com.example.calculator.Calculator", "method": "divide",
        "param_types": ["double", "double"], "args": [1, 0],
        "instance": {"kind": "static"}, "timeout_ms": 5000})
    check("异常: 除零业务异常结构化",
          resp["data"]["status"] == "failed"
          and resp["data"]["exception"]["exception_type"] == "java.lang.ArithmeticException"
          and len(resp["data"]["exception"]["stack_trace"]) > 0, json.dumps(resp))

    # 慢方法超时（agent 侧 watchdog 中断；协作式）
    t0 = time.time()
    resp = call(sock, "invoke", {
        "class": "com.example.order.OrderService", "method": "slowOperation",
        "param_types": ["long"], "args": [60],
        "instance": {"kind": "new", "consent": True}, "timeout_ms": 1500})
    elapsed = time.time() - t0
    check("超时: watchdog 中断返回 timeout",
          resp["data"]["status"] == "timeout" and elapsed < 10, f"{elapsed:.1f}s")
    check("超时: terminated=false（诚实语义）",
          resp["data"]["terminated"] is False, json.dumps(resp["data"]))

    # 取消
    invoke_id = "cancel-me-1"
    write_frame(sock, {"id": "slow-invoke", "op": "invoke", "params": {
        "invocation_id": invoke_id,
        "class": "com.example.order.OrderService", "method": "slowOperation",
        "param_types": ["long"], "args": [30],
        "instance": {"kind": "new", "consent": True}, "timeout_ms": 60000}})
    time.sleep(1.0)
    resp = call(sock, "cancel", {"invocation_id": invoke_id}, request_id="cancel-1")
    check("取消: delivered=true", resp["data"]["delivered"] is True)
    cancel_resp = call(sock, "cancel", {"invocation_id": invoke_id}, request_id="cancel-2")
    check("取消: 已结束调用 delivered=false", cancel_resp["data"]["delivered"] is False)
    # 等 invoke 响应（可能已被 call() 缓存；被取消后应以 cancelled/timeout 收尾）
    if _PENDING["slow-invoke"]:
        frame = _PENDING["slow-invoke"].popleft()
    else:
        while True:
            frame = read_frame(sock)
            if frame.get("id") == "slow-invoke":
                break
    check("取消: 调用状态 cancelled/timeout",
          frame["data"]["status"] in ("cancelled", "timeout"), json.dumps(frame))

    # 重复 attach（换发 token）
    token2 = "b" * 64
    info2 = attach(pid, token2)
    check("重复 attach: 单服务（端口不变）", info2["port"] == port)
    sock3 = socket.create_connection(("127.0.0.1", port), timeout=10)
    resp = call(sock3, "handshake", {"token": token2, "client_protocol": 2})
    check("重复 attach: 新 token 生效", resp.get("ok") is True)
    sock3.close()
    sock4 = socket.create_connection(("127.0.0.1", port), timeout=10)
    resp = call(sock4, "handshake", {"token": token, "client_protocol": 2})
    check("重复 attach: 旧 token 失效", resp["error"]["code"] == "not_authorized")
    sock4.close()

    # agent.stop
    resp = call(sock, "agent.stop", {}, request_id="stop-1")
    check("卸载: stop 响应", resp["data"]["stopping"] is True)
    time.sleep(0.5)
    try:
        s = socket.create_connection(("127.0.0.1", port), timeout=2)
        s.close()
        check("卸载: 端口已关闭", False)
    except (ConnectionRefusedError, OSError):
        check("卸载: 端口已关闭", True)
    import os
    check("卸载: infoFile 已清理", not os.path.exists(INFO_FILE))
    sock.close()
    print("\nALL SMOKE TESTS PASSED")


if __name__ == "__main__":
    main()
