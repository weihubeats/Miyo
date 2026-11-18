# 第三方插件开发指南

从零发布一个可安装、可签名、可上架官方索引的 Miyo 插件。

## 1. 选择插件形态

| 形态 | 适合 | manifest.kind |
|---|---|---|
| UI Plugin | 纯页面（数据展示 / 工具面板） | `ui` |
| UI+Sidecar | 需要后端进程（长任务 / 系统交互） | `ui-sidecar` |

## 2. UI 插件最小工程

参考：`examples/plugins/hello-world/`（两文件：manifest.json + index.html）。

- 入口 HTML **自包含**（内联 CSS/JS——blob URL 无法相对引用资源）；
- 沙箱属性 `allow-scripts`（无 same-origin）：无 DOM / 存储 / 父窗口访问；
- RPC 客户端自备（~20 行，见示例）——方法白名单见 SDK.md；
- 主程序 i18n / 主题不进沙箱：文案样式插件自持。

## 3. UI+Sidecar 插件

参考：`examples/plugins/echo-sidecar/`。sidecar 可用任意语言实现 stdio 协议
（4 字节 LE 长度前缀 + JSON 帧；`hello`/`ping`/`req` 入、`hello_ack`/`pong`/
`res`/`evt`/`log` 出，帧 ≤ 8 MiB）——Rust 参考实现
`crates/miyo-sidecar/src/bin/echo-sidecar.rs`。

manifest 关键字段：

```json
{
  "kind": "ui-sidecar",
  "sidecar": { "executable": "bin/echo-sidecar", "protocol": 1,
               "max_memory_mb": 256, "restart_policy": "on-crash" },
  "platforms": ["windows", "macos", "linux"],
  "permissions": ["runtime.process.start", "runtime.process.stop"]
}
```

## 4. 打包 / 签名 / 验证

```bash
# 生成全文件清单 + 签名（未设置 seed 则跳过签名，产物走显式信任通道）
MIYO_PLUGIN_SIGNING_SEED=<seed> bash scripts/pack_plugin.sh <插件目录>
# 验证（完整性 + 信任级）
node scripts/sign_plugin.mjs verify <插件目录>
# 安装：Miyo → 插件管理 → 本地安装（检视确认 → 权限明示 → 安装）
```

约束速查：
- 包 = tar.gz（.miyo）；条目 ≤ 200、单文件 ≤ 10 MiB、总量 ≤ 64 MiB；
- 文件名白名单 `[A-Za-z0-9._-/]`；禁止符号链接 / 绝对路径 / `..`；
- 未签名包需用户勾选「信任此未签名插件」；
- `runtime_deps` 只做声明与 UI 提示（如 Java ≥ 11），连通性由插件自行探测。

## 5. 权限申请

按需声明（目录见 PERMISSIONS.md）；High 风险项（java.method.invoke /
runtime.process.start / database.*）在安装确认页单独着色明示。声明未知权限
→ manifest 校验直接失败。

## 6. 上架官方索引

1. Fork 仓库 → 在 `docs/market/index.json` 追加条目（schema 见 MARKETPLACE.md）；
2. 发布 .miyo 产物到 GitHub Release（附件名建议 `<id>-<version>.miyo`）；
3. PR 合入后随下个 Miyo 版本进入内置索引。
