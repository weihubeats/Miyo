# Plugin SDK 2.0

> 实现：`frontend/src/plugins/sdk.ts`（createPluginSession）+
> `host/bridge.ts`（RPC 桥）+ `host/protocol.ts`（帧与限额）。
> RPC 帧结构保持 v1（向后兼容），安全能力为宿主侧增量。

## API 面

```ts
interface PluginContext {
  manifest: PluginManifest
  storage: { get / set / delete / keys }        // std.storage（会话凭证自动携带）
  logger: { debug / info / warn / error / audit } // std.log
  notify: { info / success / warn / error }       // std.notify
  events: { emit / on }                           // std.events（命名空间隔离）
  env: { getVariableMap() }                       // std.env.read
  commands: { invoke(command, args) }             // 白名单 + granted 双闸
  config: { get / set }                           // __config 保留键
  clipboard: { writeText }                        // std.clipboard.write
  ui: { nav / route register / unregister }       // 宿主桥委托（内置插件）
}
```

**2.0 变化**：
- 所有 storage / log 调用自动携带会话凭证（`plugin_session_create` 惰性签发；
  FORBIDDEN 时重建会话重试一次——收权 / 停用即时生效且调用方无感，二次失败
  如实上抛）；
- `commands.invoke` 白名单按权限细分映射（java.* 四子权限 / runtime.* /
  std.env.read）；granted 快照含 java.debug 聚合键时等价展开（与后端同口径）；
- 第三方 UI 插件可经 RPC `commands.invoke` 调 `plugin_sidecar_start/stop/
  request`（runtime.process.* 权限）。

## RPC v1（沙箱 iframe ↔ 宿主）

帧：`{ v:1, type:'req', id, method, params? }`；响应 res / 事件 evt。
方法白名单 12 个 + 参数形状校验（`host/protocol.ts::METHODS`）。

### 宿主侧安全限额（RPC_LIMITS）

| 限额 | 值 | 行为 |
|---|---|---|
| 请求 ID 去重 | 表 4096 | 同 id 重放直接丢弃（重放防护） |
| 并发在途 | 20 | 超出立即 RATE_LIMITED |
| 整帧体积 | 1,000,000 字符 | 超限静默丢弃（不回执，防探测） |
| JSON 深度 | 16 | BAD_REQUEST |
| 订阅数 | 32 / 插件 | PLUGIN_SUBSCRIPTION_LIMIT |
| 熔断 | 30s 窗口 50 错 → 开 60s | 打开期 RATE_LIMITED 快速失败 |
| nonce | 8B hex | 帧可选携带 `n`，不匹配静默丢帧；**v1 插件不携带保持兼容，v2 起强制** |

未知帧 / 未知方法 / 非本宿主窗口的帧一律丢弃；权限拒绝统一
`PERMISSION_DENIED`。

## 版本协商

- manifest.sdk_version ↔ SUPPORTED_SDK_VERSION 严格相等（启用前置 + 后端强制）；
- RPC_VERSION = 1（帧结构破坏性变更才递增）；
- sidecar.protocol 独立版本号（hello 握手一致性校验）。

## 已知边界（诚实声明）

- `registerCommand` / `registerSettings`：未实现（路线图）——插件自定义命令
  面与设置页注册；
- 第三方插件动态路由：仍锁死 manifest.nav → PluginIframeHost；
- iframe 内资源加载：blob URL 无法相对引用——插件包需单文件自包含（入口
  HTML 内联 JS/CSS），二进制资源暂不经 plugin_read_asset 之外的通道。
