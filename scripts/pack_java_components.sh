#!/usr/bin/env bash
# 打包 Java Attach 组件（Release 附件：jar + sha256 sidecar）。
# 用法：scripts/pack_java_components.sh <输出目录>
# 被 release.yml 调用；组件由市场「组件自动获取」按需下载
# （java_component_download 校验 sha256 后落盘 data_dir/java/）。
set -euo pipefail

out_dir="${1:?用法: pack_java_components.sh <输出目录>}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
mkdir -p "$out_dir"

# 增量构建 jar（缺失 / 过期才重建）
bash "$ROOT/scripts/ensure-java-jars.sh"

for jar in miyo-java-agent.jar miyo-attach-launcher.jar; do
  src="$ROOT/java/dist/$jar"
  if [[ ! -f "$src" ]]; then
    echo "错误：$src 不存在" >&2
    exit 1
  fi
  cp "$src" "$out_dir/$jar"
  if command -v sha256sum >/dev/null 2>&1; then
    (cd "$out_dir" && sha256sum "$jar" > "$jar.sha256")
  else
    (cd "$out_dir" && shasum -a 256 "$jar" > "$jar.sha256")
  fi
  echo "已打包：${out_dir}/${jar}（含 .sha256）"
done
