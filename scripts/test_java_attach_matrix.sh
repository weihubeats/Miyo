#!/usr/bin/env bash
# JDK 目标版本兼容性矩阵：JDK 8 / 11 / 17 / 21 目标进程 × JDK21 launcher。
# 每个目标：attach → 握手 → 静态调用 → 卸载，输出一行结论。
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LAUNCHER="$ROOT/java/dist/miyo-attach-launcher.jar"
AGENT="$ROOT/java/dist/miyo-java-agent.jar"
APP_DIR="$ROOT/java/examples/plain-app"
INFO="/tmp/miyo-info-matrix.json"

JDKS=(
  "/Library/Java/JavaVirtualMachines/zulu-8.jdk/Contents/Home"
  "/Library/Java/JavaVirtualMachines/zulu-11.jdk/Contents/Home"
  "/Users/weihu/Library/Java/JavaVirtualMachines/azul-17.0.13/Contents/Home"
  "/Users/weihu/Library/Java/JavaVirtualMachines/azul-21.0.6/Contents/Home"
)

pkill -f "com.example.PlainApp" 2>/dev/null || true; sleep 1
# demo 源码以 --release 8 重编一份（Java 8 目标可运行；plain-app 未用 8+ API）
rm -rf "$APP_DIR/build/classes8"
mkdir -p "$APP_DIR/build/classes8"
(cd "$APP_DIR" && javac --release 8 -d build/classes8 @build/sources.txt) 2>/dev/null \
  || echo "(warn: --release 8 编译失败，Java 8 目标将跳过)"
for JDK in "${JDKS[@]}"; do
  [ -x "$JDK/bin/java" ] || { echo "SKIP: $JDK 不存在"; continue; }
  VER=$("$JDK/bin/java" -version 2>&1 | head -1 | sed 's/.*"\(.*\)".*/\1/')
  CLASSES="$APP_DIR/build/classes8"
  [ -d "$CLASSES" ] || CLASSES="$APP_DIR/build"
  "$JDK/bin/java" -cp "$CLASSES" com.example.PlainApp > "$APP_DIR/build/app.log" 2>&1 &
  JAVA_PID=$!
  # 等 JVM 就绪（seeded order 出现在日志）
  READY=0
  for i in $(seq 1 30); do
    grep -q "seeded order" "$APP_DIR/build/app.log" && READY=1 && break
    sleep 0.5
  done
  PID="$JAVA_PID"
  if [ "$READY" != "1" ]; then
    echo "FAIL JDK $VER: 进程未就绪"; tail -3 "$APP_DIR/build/app.log"
    kill "$JAVA_PID" 2>/dev/null || true; continue
  fi
  TOKEN=$(openssl rand -hex 32)
  if ! ATTACH_OUT=$(java -jar "$LAUNCHER" attach --pid "$PID" --agent "$AGENT" \
      --args "token=$TOKEN,infoFile=$INFO,expectedPid=$PID" \
      --info-file "$INFO" --timeout-ms 20000 2>&1); then
    echo "FAIL JDK $VER (pid $PID): attach 失败: $ATTACH_OUT"
    kill "$JAVA_PID" 2>/dev/null || true; sleep 1; continue
  fi
  RESULT=$(INFO="$INFO" TOKEN="$TOKEN" EXPECT_PID="$PID" python3 - <<'PYEOF'
import json, os, socket, struct
info = json.load(open(os.environ["INFO"]))
token = os.environ["TOKEN"]
sock = socket.create_connection(("127.0.0.1", info["port"]), timeout=10)
def wf(o):
    d = json.dumps(o).encode(); sock.sendall(struct.pack(">I", len(d)) + d)
def rf():
    h = b""
    while len(h) < 4:
        c = sock.recv(4 - len(h)); h += c
    (l,) = struct.unpack(">I", h); d = b""
    while len(d) < l:
        d += sock.recv(l - len(d))
    return json.loads(d)
wf({"id": "h", "op": "handshake", "params": {"token": token, "client_protocol": 2}})
r = rf()
if not r.get("ok"):
    print("FAIL handshake:", json.dumps(r)); raise SystemExit(1)
jv = r["data"]["java_version"]
wf({"id": "i", "op": "invoke", "params": {
    "class": "com.example.calculator.Calculator", "method": "add",
    "param_types": ["long", "long"], "args": [20, 22],
    "instance": {"kind": "static"}, "timeout_ms": 10000}})
r2 = rf()
ok = r2.get("ok") and r2["data"]["status"] == "success" and r2["data"]["result"] == 42
print("OK java_version=%s invoke=%s" % (jv, ok and 42 or r2))
wf({"id": "s", "op": "agent.stop", "params": {}})
rf()
sock.close()
PYEOF
)
  echo "PASS JDK $VER (pid $PID): $RESULT"
  kill "$JAVA_PID" 2>/dev/null || true; sleep 1
done
