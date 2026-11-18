# 插件市场方案

> 实现：`docs/market/index.json`（官方静态索引，编译期嵌入）+
> `plugin_market_index` 命令 + PluginsView 市场页签。

## 设计原则

按任务书要求：「市场暂时先使用官方静态索引，不引入复杂服务端架构」。

- **本期**：编译期嵌入的静态 JSON（`include_str!`），零网络依赖、零服务端；
- **路线图**：设置项 `plugin.market.url` 指向远端索引（同 schema），签名验证
  后合并展示；分发即 GitHub Releases 的 .miyo 附件——不做服务端。

## 功能矩阵

| 功能 | 状态 | 说明 |
|---|---|---|
| 官方插件 | ✅ | 内置目录（编译期嵌入 manifest）+ 索引条目合并展示 |
| 第三方插件 | ✅ | 索引条目（无内置目录命中时：复制下载地址 + 获取引导） |
| 搜索 | ✅ | 名称 / ID / 描述（大小写不敏感） |
| 分类 | ✅ | debuggers / examples / tools…（索引 category 字段驱动） |
| 版本 | ✅ | 已安装版本与目录版本比对（更新按钮） |
| 作者 / 描述 | ✅ | 卡片展示 |
| 权限清单 | ✅ | 安装前风险着色 chips（最小知情） |
| 更新日志 | ✅ | 索引 changelog 字段（详情抽屉概览） |
| 下载和安装 | ✅ | 复制下载 URL → 本地 .miyo 检视安装（签名信任流）；内置命中直接安装 |
| 启用 / 禁用 / 卸载 | ✅ | 已安装页签（builtin/local 同规则） |
| 更新 | ✅ | builtin：plugin_update_builtin；local：从 .miyo 包更新（检视流） |
| 回滚 | ✅ | 本地包：版本历史 + 备份目录换回（确认弹窗 / 详情页） |
| 配置 | ✅ | config_schema 驱动表单（会话凭证保护） |
| 日志 | ✅ | 级别过滤 + audit 置顶 |
| 权限管理 | ✅ | 批准快照复选框 + 收权即时生效 + 审计 |
| 运行状态 | ✅ | 激活耗时/次数 + Sidecar（PID/版本/重启数） |
| 签名状态 | ✅ | official/foreign/unsigned 三级徽章 |
| 文件完整性 | ✅ | files_verified + 最近校验时间 + 手动校验按钮 |
| 加载耗时 | ✅ | activate ms 计量 |
| 最近错误 | ✅ | error 态现场 + 恢复按钮 |

## 索引 schema（docs/market/index.json）

```json
{
  "version": 1,
  "updated_at": "2026-11-19",
  "plugins": [
    {
      "id": "com.example.hello-world",
      "name": "Hello World 示例",
      "version": "0.1.0",
      "authors": ["Miyo Examples"],
      "description": "…",
      "category": "examples",
      "permissions": ["std.storage", "…"],
      "min_app_version": "0.0.60",
      "sdk_version": 1,
      "changelog": "…",
      "download_url": "https://github.com/…"
    }
  ]
}
```

新增官方插件：改 `docs/market/index.json` +（如随应用内置）在
`crates/miyo-plugin/builtins/` 加目录 + 前端 `builtinModules.ts` 注册 loader。
