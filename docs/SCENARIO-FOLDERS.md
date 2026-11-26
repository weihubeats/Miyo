# 集成测试场景目录（多级文件夹）

场景库从「扁平列表 + 分类 chips」升级为「左侧业务目录树 + 右侧场景列表」。
目录按业务域组织（如 电商业务 / 订单中心 / 支付），支持多级、拖拽、批量操作，
且不改变场景执行语义。

## 数据模型

| 位置 | 内容 |
| --- | --- |
| `scenario_folders`（迁移 `0035`） | `id / project_id / parent_id / name / sort_order / created_at / updated_at`，`parent_id` 自引用 |
| `scenarios.folder_id` | 所属目录，`NULL` = 未分类；指向已删目录时按未分类展示 |
| `scenarios.tags_json` | 标签数组（`Vec<String>`），批量追加去重 |
| `scenarios.favorite` | 收藏标记，列表默认置顶 |
| `scenarios.run_count / ok_run_count / fail_run_count` | 全量执行计数 |

约束：

- 同级目录名唯一（DB 唯一索引 `COALESCE(parent_id,'')` + 应用层预检给出可读报错）；
- 场景目录与接口 `folders` 是**两张独立的表**：删场景目录不影响接口树，反之亦然；
- `scenarios.folder_id` 有 `ON DELETE SET NULL` 外键，但删除逻辑不依赖
  `PRAGMA foreign_keys`（递归 CTE 显式处理子树）。

### 执行计数为什么单独存列

运行历史会按场景修剪到最近 N 条（`SCENARIO_RUN_KEEP`），从 `scenario_runs`
聚合会随修剪丢失总数。因此每次落历史时增量更新
`run_count / ok_run_count / fail_run_count`（`bump_scenario_run_stats`）：

- `ok` 计数：主步骤无失败且未取消；
- `fail` 计数：主步骤有失败，**不含取消**；
- 取消的运行只计入 `run_count`。

存量行的旧历史不回填（从 0 起算）。

## 删除策略（三选一，绝不静默丢数据）

删除目录时按子树内场景数决定可选策略：

| mode | 行为 |
| --- | --- |
| `empty` | 仅删除空目录；子树内有场景时后端拒绝（有场景时 UI 禁用该项） |
| `keep` | 删除目录，子树内场景移到未分类（`folder_id = NULL`） |
| `cascade` | 删除目录 + 子树内全部场景 + 其运行历史（UI 需勾选二次确认） |

实现：递归 CTE 收集子树 → 按 mode 处理场景 → 删除目录，全部单事务。

## 命令（Tauri）

| 命令 | 说明 |
| --- | --- |
| `list_scenario_folders` | 项目全部目录（`sort_order` → 名称序） |
| `save_scenario_folder` | 新建 / 重命名（校验同级重名、父级同项目、成环） |
| `move_scenario_folder` | 移动目录到新父级（防环：不能移到自身或后代） |
| `delete_scenario_folder` | `mode = empty \| keep \| cascade` |
| `reorder_scenario_folders` | 同级重排（拖拽后写回 `sort_order`） |
| `move_scenarios` | 批量移动场景（`folderId = null` 即未分类） |
| `add_scenario_tags` | 批量追加标签（去重合并，不覆盖已有） |
| `set_scenarios_favorite` | 批量收藏 / 取消收藏 |
| `delete_scenarios` | 批量删除（连同运行历史，单事务） |
| `reorder_scenarios` | 同级场景重排 |
| `list_scenario_summaries` | 列表项含 `last_run` + 三个全量计数 |

## 前端结构

- `domains/integration/utils/folderTree.ts`：纯函数——建树、子树收集、路径、
  防环校验、状态汇总、扁平化。孤儿目录（父级缺失）提升为根，环用访问集兜底。
- `components/integration/ScenarioFolderTree.vue`：左侧树（扁平渲染，避免递归组件），
  行内新建子目录 / 重命名 / 删除，拖拽落点高亮。
- `components/integration/ScenarioLibrary.vue`：右侧列表 / 卡片视图、搜索（含标签与
  目录路径）、状态 / 分类 / 收藏 / 目录筛选、多选 + 批量工具栏、拖拽移动。
- `stores/integration.ts`：目录缓存、`folderFilter`、`viewMode`、`expandedFolders`、
  批量选择——跨路由存活（进编辑器返回不丢状态、不重放请求）。

筛选语义：选中某目录时**包含其后代目录**的场景；「未分类」= `folder_id` 为空或
指向已删目录；搜索跨目录（名称 / 描述 / 标签 / 目录路径）。

## 备份与同步

- 备份 `schema_version = 6`，新增 `scenario_folders`；
- 恢复：目录 id 与 `parent_id` 重映射到新项目，场景 `folder_id` 随目录映射表重映射
  （旧备份 / 残缺备份 → 未分类）；写入顺序为目录先、场景后（外键引用）；
- 同步覆盖：保留目录 id（同类保留接口 id 的口径），场景引用无需重映射。

## 回归要点

- 目录树 3 态（空 / 有场景 / 有失败）与场景数汇总；
- 删除三策略：空目录拒绝、keep 移动不丢、cascade 连历史一起删；
- 防环：不能把目录移到自身或后代；
- 同级重名拒绝，不同父级可同名；
- 批量移动 / 打标 / 收藏 / 删除的项目归属校验；
- 备份往返：目录与 `folder_id` 重映射后不残留旧 id。