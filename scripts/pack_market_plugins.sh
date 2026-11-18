#!/usr/bin/env bash
# 打包市场远程安装产物（Release 附件；被 release.yml 调用）。
# 用法：scripts/pack_market_plugins.sh <输出目录>
# - hello-world：平台无关（纯 UI），始终打包；
# - echo-sidecar：含平台二进制——仅当 bin/echo-sidecar 存在时打包（CI 无矩阵
#   构建，开发者可先按 DEV-GUIDE 构建后手动打包）。
# 签名：设置 MIYO_PLUGIN_SIGNING_SEED 后自动签名；否则产出未签名包（安装时
# 需用户显式信任）。
set -euo pipefail

out_dir="${1:?用法: pack_market_plugins.sh <输出目录>}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
mkdir -p "$out_dir"

pack_one() {
  local dir="$1"
  # 在副本上签名打包（不污染仓库源目录）
  local tmp
  tmp="$(mktemp -d)/$(basename "$dir")"
  cp -R "$dir" "$tmp"
  mkdir -p "$(dirname "$tmp")"
  cp -R "$dir" "$tmp" 2>/dev/null || true
  if [[ -n "${MIYO_PLUGIN_SIGNING_SEED:-}" ]]; then
    node "$ROOT/scripts/sign_plugin.mjs" sign "$tmp"
  fi
  local out
  out="$out_dir/$(node -e "const m=require('$tmp/manifest.json');console.log(m.id+'-'+m.version+'.miyo')")"
  tar --format=ustar -czf "$out" -C "$tmp" .
  rm -rf "$(dirname "$tmp")"
  echo "已打包：$out"
}

if [[ -d "$ROOT/examples/plugins/hello-world" ]]; then
  pack_one "$ROOT/examples/plugins/hello-world"
fi

if [[ -d "$ROOT/examples/plugins/echo-sidecar" && -f "$ROOT/examples/plugins/echo-sidecar/bin/echo-sidecar" ]]; then
  pack_one "$ROOT/examples/plugins/echo-sidecar"
else
  echo "（跳过 echo-sidecar：bin/echo-sidecar 未构建——见 examples/plugins/echo-sidecar/README.md）"
fi
