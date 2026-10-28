#!/usr/bin/env bash
# 录制 README/官网用的快速上手演示 GIF：
# 搜索「开放演示」项目 → 打开 → 选中「文章列表」→ 发送 → 公网 JSON 响应。
#
# 前置条件：
#   1. cliclick（brew install cliclick）；
#   2. Python3 + Pillow（装配 GIF 帧）；
#   3. 开发应用已在运行：cd frontend && npm run tauri dev（另开终端）；
#   4. 网络可达 jsonplaceholder.typicode.com（演示走公网真实接口）。
#
# 稳定性设计：
#   - 全程 AX（辅助功能）定位元素坐标后 cliclick 点击，每步轮询验证目标
#     出现才走下一步——固定 sleep + 盲点坐标是早期脚本的两大翻车点；
#   - 键入一律走剪贴板粘贴（合成按键会被中文输入法截获）；
#   - 演示后端为公网真实服务，无需本地 Mock / 环境切换。
#
# 用法：bash scripts/make_demo_gif.sh
# 产物：docs/imags/demo-debug.gif（720×400，逐帧步进式演示）

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="$ROOT/docs/imags/demo-debug.gif"
FRAMES="$(mktemp -d)"

cleanup() { rm -rf "$FRAMES"; }
trap cleanup EXIT

say() { echo "==> $*"; }

# ---------- AX 工具 ----------
# 轮询等待名称含 $1 的元素出现（$2 = 超时秒）。
wait_el() {
  local name="$1" timeout="${2:-10}" script
  script="tell application \"System Events\" to tell process \"miyo\"
    set els to entire contents of window 1
    repeat with e in els
      try
        if name of e contains \"$name\" then return true
      end try
    end repeat
    return false
  end tell"
  local deadline=$((SECONDS + timeout))
  while (( SECONDS < deadline )); do
    if osascript -e "$script" 2>/dev/null | grep -q true; then return 0; fi
    sleep 1
  done
  return 1
}

# 输出名称含 $1 的第一个元素的点击中心坐标（"x,y"），找不到返回空。
ax_pos() {
  local name="$1" script
  script="tell application \"System Events\" to tell process \"miyo\"
    set els to entire contents of window 1
    repeat with e in els
      try
        if role of e is not \"AXTextField\" and name of e contains \"$name\" then
          set {px, py} to position of e
          set {sx, sy} to size of e
          return (px + (sx div 2) as string) & \",\" & ((py + (sy div 2)) as string)
        end if
      end try
    end repeat
    return \"\"
  end tell"
  osascript -e "$script" 2>/dev/null
}

# AX 定位坐标后 cliclick 点击中心（带轮询重试）。
ax_click() { # $1=名称子串 $2=超时秒
  local pos="" deadline=$((SECONDS + ${2:-10})) attempt=1
  while (( SECONDS < deadline )); do
    pos=$(ax_pos "$1")
    if [[ -n "$pos" ]]; then
      cliclick c:"$pos"
      return 0
    fi
    sleep 1
    attempt=$((attempt + 1))
  done
  return 1
}

shot() { screencapture -R0,25,1440,800 "$FRAMES/$1.png"; }

# ---------- 前置：窗口定位 ----------
osascript -e 'tell application "System Events" to tell process "miyo" to set frontmost to true' \
          -e 'tell application "System Events" to tell process "miyo" to set position of window 1 to {0, 25}' \
          -e 'tell application "System Events" to tell process "miyo" to set size of window 1 to {1440, 800}'
sleep 1

say "回到仪表盘（点品牌 Logo，AX 定位）"
ax_click "Miyo API 调试工具" 8 || cliclick c:184,59
sleep 1.5

say "帧 1：项目列表（首页仪表盘）"
shot f1

say "搜索过滤：开放演示"
cliclick c:330,330
sleep 0.5
osascript -e 'set the clipboard to "开放演示"'
osascript -e 'tell application "System Events" to keystroke "a" using command down'
sleep 0.2
osascript -e 'tell application "System Events" to keystroke "v" using command down'
sleep 1.2

say "帧 2：点击开放演示项目卡（AX 定位）"
ax_click "开放演示" 12 || { echo "✗ 未找到开放演示项目卡" >&2; exit 1; }
wait_el "文章列表" 12 || { echo "✗ 工作台未就绪" >&2; exit 1; }
shot f2

say "帧 3：选中接口（文章列表，AX 定位）"
ax_click "文章列表" 10 || { echo "✗ 未找到文章列表接口" >&2; exit 1; }
wait_el "发送" 10 || { echo "✗ 编辑器未就绪" >&2; exit 1; }
sleep 1
shot f3

say "帧 4：发送请求（真实公网接口，AX 确认响应格式化按钮出现）"
osascript <<'OSA' 2>&1
tell application "System Events" to tell process "miyo"
  set els to entire contents of window 1
  repeat with e in els
    try
      if role of e is "AXButton" and name of e is "发送" then
        click e
        return "sent"
      end if
    end try
  end repeat
  return "发送 not found"
end tell
OSA
for attempt in 1 2 3; do
  if wait_el "格式化" 12; then break; fi
  say "  重试发送（${attempt}）"
  osascript <<'OSA' 2>&1
tell application "System Events" to tell process "miyo"
  set els to entire contents of window 1
  repeat with e in els
    try
      if role of e is "AXButton" and name of e is "发送" then
        click e
        exit repeat
      end if
    end try
  end repeat
end tell
OSA
  sleep 2
done
wait_el "格式化" 10 || { echo "✗ 响应未出现" >&2; exit 1; }
sleep 0.8
shot f4

say "帧 5：镜头停在响应 JSON"
sleep 1.5
shot f5

# ---------- 装配 ----------
say "装配 GIF → $OUT"
python3 - "$FRAMES" "$OUT" <<'PY'
import sys, glob
from PIL import Image
frames_dir, out = sys.argv[1], sys.argv[2]
files = sorted(glob.glob(f"{frames_dir}/f*.png"))
assert files, "没有捕获到任何帧"
frames = []
for f in files:
    im = Image.open(f).convert("RGB").resize((720, 400), Image.LANCZOS)
    frames.append(im)
durations = [1400, 1000, 1000, 900, 2400][: len(frames)]
if len(durations) < len(frames):
    durations += [2000] * (len(frames) - len(durations))
frames[0].save(out, save_all=True, append_images=frames[1:], duration=durations, loop=0)
print(f"frames={len(frames)} -> {out}")
PY
say "完成：$OUT"
