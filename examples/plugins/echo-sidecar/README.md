# Echo Sidecar 示例插件（ui-sidecar 形态）

展示「iframe UI + 常驻 sidecar 进程」的完整插件形态。

## 结构

```
echo-sidecar/
├── manifest.json      # kind: ui-sidecar + sidecar 配置 + files 清单
├── index.html         # iframe UI（经 RPC 调 runtime.start / runtime.stop）
└── bin/echo-sidecar   # sidecar 可执行文件（按平台构建后放入）
```

## 构建与打包

```bash
# 1. 构建 sidecar 二进制（源码在 crates/miyo-sidecar/src/bin/echo-sidecar.rs，
#    第三方插件可用任意语言实现同一 stdio 协议）
cargo build --release -p miyo-sidecar --bin echo-sidecar
cp target/release/echo-sidecar examples/plugins/echo-sidecar/bin/echo-sidecar

# 2. 生成 files 清单并签名（需要 MIYO_PLUGIN_SIGNING_SEED；未设置则产出未签名包）
node scripts/sign_plugin.mjs sign examples/plugins/echo-sidecar

# 3. 打包
bash scripts/pack_plugin.sh examples/plugins/echo-sidecar
```

## 生命周期

- **安装**：解包 + files 清单逐文件 sha256 校验 + 签名复核；
- **启用**：UI 经 iframe 加载（plugin_read_asset 收口读取）；
- **runtime.start**：宿主 spawn `bin/echo-sidecar`（setsid / Job Object 加固、
  最小环境变量），stdin 握手下发一次性 token（hello_ack 回显校验）；
- **心跳**：10s ping / 30s 无 pong 判死 → on-crash 策略指数退避重启
  （1 小时内超 5 次放弃，防启动循环）；
- **runtime.stop / 卸载**：进程组 kill（unix kill(-pgid) / Windows Job
  KILL_ON_JOB_CLOSE），无僵尸残留。

## sidecar 协议（v1）

stdio 上 4 字节小端长度前缀 + JSON 帧；宿主 → sidecar：`hello`（含一次性
token）/ `ping` / `req`；sidecar → 宿主：`hello_ack`（token 回显）/ `pong` /
`res` / `evt` / `log`。单帧上限 8 MiB。参考实现：
`crates/miyo-sidecar/src/bin/echo-sidecar.rs`。
