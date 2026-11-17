#!/usr/bin/env bash
# Miyo Java Debug Runtime 端到端联调：
# 编译 SDK → 编译 demo 应用 → 启动 Runtime → 用 curl 走完整协议（info /
# classes / methods / invoke / cancel / 鉴权拒绝）→ 清理。
#
# 前置：JDK 11+（本机 Java 21 验证通过）。Maven 可选——无 Maven 时直接 javac。
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
SDK_DIR="$ROOT/java/miyo-java-runtime"
DEMO_DIR="$ROOT/java/examples/demo-app"
BUILD="$DEMO_DIR/build"
TOKEN="demo-e2e-token-$(date +%s)"
PORT="${MIYO_DEBUG_PORT:-9918}"

rm -rf "$BUILD"
mkdir -p "$BUILD/sdk" "$BUILD/demo"

echo "==> 编译 SDK（零依赖，javac 直编）"
find "$SDK_DIR/src/main/java" -name '*.java' > "$BUILD/sources.txt"
javac -d "$BUILD/sdk" @"$BUILD/sources.txt"

echo "==> 编译 demo 应用"
find "$DEMO_DIR/src" -name '*.java' > "$BUILD/demo-sources.txt"
javac -cp "$BUILD/sdk" -d "$BUILD/demo" @"$BUILD/demo-sources.txt"

echo "==> 启动 Runtime（端口 ${PORT}，token ${TOKEN}）"
MIYO_DEBUG_TOKEN="$TOKEN" MIYO_DEBUG_PORT="$PORT" \
  java -cp "$BUILD/sdk:$BUILD/demo" com.example.demo.DemoApplication \
  > "$BUILD/runtime.log" 2>&1 &
RUNTIME_PID=$!
trap 'kill "$RUNTIME_PID" 2>/dev/null || true' EXIT

for _ in $(seq 1 30); do
  if curl -s -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:$PORT/miyo/debug/info" > "$BUILD/info.json" 2>/dev/null; then
    if grep -q protocol_version "$BUILD/info.json"; then break; fi
  fi
  sleep 0.3
done

BASE="http://127.0.0.1:${PORT}/miyo/debug"
AUTH="Authorization: Bearer $TOKEN"

echo "==> info"
cat "$BUILD/info.json"; echo
grep -q '"app_name":"demo-order-app"' "$BUILD/info.json" || { echo "FAIL: info"; exit 1; }

echo "==> 错误 token 应 401"
CODE=$(curl -s -o /dev/null -w '%{http_code}' -H "Authorization: Bearer wrong" "$BASE/info")
[ "$CODE" = "401" ] || { echo "FAIL: 期望 401 实际 $CODE"; exit 1; }

echo "==> classes"
CLASSES=$(curl -s -H "$AUTH" "$BASE/classes")
echo "$CLASSES" | head -c 400; echo
echo "$CLASSES" | grep -q 'com.example.demo.DemoApplication$OrderService' || { echo "FAIL: classes"; exit 1; }

echo "==> methods?class=OrderService"
METHODS=$(curl -s -H "$AUTH" --get --data-urlencode 'class=com.example.demo.DemoApplication$OrderService' "$BASE/methods")
echo "$METHODS" | grep -q 'createOrder' || { echo "FAIL: methods"; exit 1; }
echo "$METHODS" | grep -q '"overloaded":false' || true

echo "==> 未注册类查询应 404"
CODE=$(curl -s -o /dev/null -w '%{http_code}' -H "$AUTH" --get --data-urlencode 'class=com.example.demo.Secret' "$BASE/methods")
[ "$CODE" = "404" ] || { echo "FAIL: 期望 404 实际 $CODE"; exit 1; }

echo "==> invoke MathUtil#add(long,long)"
ADD=$(curl -s -H "$AUTH" -H 'Content-Type: application/json' -X POST "$BASE/invoke" -d '{
  "invocation_id": "11111111-2222-3333-4444-555555555555",
  "method_id": "com.example.demo.DemoApplication$MathUtil#add(long,long)",
  "arguments": [2, 40],
  "timeout_ms": 5000
}')
echo "$ADD"; echo
echo "$ADD" | grep -q '"status":"success"' || { echo "FAIL: add"; exit 1; }
echo "$ADD" | grep -q '"result":42' || { echo "FAIL: add 结果"; exit 1; }

echo "==> invoke OrderService#createOrder（DTO 注入）"
ORDER=$(curl -s -H "$AUTH" -H 'Content-Type: application/json' -X POST "$BASE/invoke" -d '{
  "invocation_id": "22222222-3333-4444-5555-666666666666",
  "method_id": "com.example.demo.DemoApplication$OrderService#createOrder(String,CreateOrderRequest)",
  "arguments": ["u-42", {"userId": "u-42", "items": [{"sku": "SKU-1", "qty": 2}, {"sku": "SKU-2", "qty": 1}], "coupon": "NEW10"}],
  "timeout_ms": 5000
}')
echo "$ORDER" | head -c 300; echo
echo "$ORDER" | grep -q '"status":"success"' || { echo "FAIL: createOrder"; exit 1; }

echo "==> invoke 异常路径（订单不存在）"
FAIL=$(curl -s -H "$AUTH" -H 'Content-Type: application/json' -X POST "$BASE/invoke" -d '{
  "invocation_id": "33333333-4444-5555-6666-777777777777",
  "method_id": "com.example.demo.DemoApplication$OrderService#find(String)",
  "arguments": ["NO-404"],
  "timeout_ms": 5000
}')
echo "$FAIL" | head -c 300; echo
echo "$FAIL" | grep -q '"status":"failed"' || { echo "FAIL: 异常路径"; exit 1; }
echo "$FAIL" | grep -q '订单不存在' || { echo "FAIL: 异常消息"; exit 1; }

echo "==> invoke 未注册方法应 404"
CODE=$(curl -s -o /dev/null -w '%{http_code}' -H "$AUTH" -H 'Content-Type: application/json' -X POST "$BASE/invoke" -d '{
  "invocation_id": "44444444-5555-6666-7777-888888888888",
  "method_id": "com.example.demo.DemoApplication$OrderService#notThere()",
  "arguments": []
}')
[ "$CODE" = "404" ] || { echo "FAIL: 期望 404 实际 $CODE"; exit 1; }

echo "==> 黑名单方法应 403"
CODE=$(curl -s -o /dev/null -w '%{http_code}' -H "$AUTH" -H 'Content-Type: application/json' -X POST "$BASE/invoke" -d '{
  "invocation_id": "55555555-6666-7777-8888-999999999999",
  "method_id": "java.lang.Runtime#exec(String)",
  "arguments": ["ls"]
}')
[ "$CODE" = "404" ] || [ "$CODE" = "403" ] || { echo "FAIL: 期望 403/404 实际 $CODE"; exit 1; }

echo
echo "=========================================="
echo "Java Runtime 端到端联调全部通过 ✓"
echo "=========================================="
