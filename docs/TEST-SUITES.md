# 测试套件与批量执行

集成测试场景可以单独运行，也可以组织成**测试套件（Test Suite）**做批量回归。
一次批量执行会被持久化为**批次（Test Run Batch）**，逐场景记录状态、耗时、
失败原因与对应的单场景执行记录，可暂停、恢复、取消、重试失败项或整批重跑。

> 设计原则：批量调度全部在 Rust 侧完成（`miyo-test::batch` 状态机 +
> `miyo-tauri` 命令层），前端只负责运行配置与结果展示。单场景执行仍走原有
> `run_scenario_lifecycle`，不另起一套执行引擎。

## 1. 概念与数据模型

| 对象 | 表 | 说明 |
| --- | --- | --- |
| 测试套件 | `test_suites` | 项目级场景集合，只保存引用 + 顺序 + 启用，不复制场景步骤 |
| 套件成员 | `test_suite_items` | `(suite_id, scenario_id)` 唯一；`sort_order` / `enabled` |
| 执行批次 | `test_run_batches` | 一次运行（套件 / 勾选 / 目录）的配置、状态、计数、起止时间 |
| 批次明细 | `test_run_batch_items` | 单个场景的结果：状态 / 次数 / 耗时 / 失败原因 / `execution_record_id` |

关键约定：

- **同一场景可被多个套件引用**；套件与场景数据分表，互不复制。
- 批次明细保存 `scenario_name` 快照，且 `suite_id` / `scenario_id` **不设外键**：
  场景或套件被删除后，历史报告仍可读（不会级联清空历史）。
- 明细的 `execution_record_id` 指向 `scenario_runs.id`，可直接回看该次单场景执行
  的步骤结果与变量池。

迁移文件：`crates/miyo-storage/migrations/0036_test_suites.sql`。

## 2. 三个执行入口

三个入口最终都落到**同一张批次表**，行为一致：

1. **套件全量执行**：集成测试 → 测试套件 → 选中套件 → 运行套件；
2. **手动勾选场景**：场景库勾选多个场景 → 批量执行；
3. **目录批量执行**：场景目录树行内「运行」按钮（含子目录）。

运行前会打开**批量执行配置**：执行模式、最大并发、失败处理、重试次数、
是否跳过禁用场景；执行环境跟随当前激活环境（切换环境请新建批次）。

## 3. 执行模式与失败策略

| 配置 | 取值 | 行为 |
| --- | --- | --- |
| 模式 | `serial`（默认） | 严格按套件顺序逐个执行 |
| 模式 | `parallel` | 按「波次」并发，每波最多 `concurrency` 个（1–50） |
| 失败处理 | `continue`（默认） | 失败后继续执行后续场景 |
| 失败处理 | `stop` | 出现失败即停止调度，剩余场景标记 `skipped`，批次以 `failed` 收尾 |
| 失败处理 | `retry` | 失败项按 `retry_count` 重试，仍失败则继续后续场景 |
| 失败处理 | `skip_dependents` | 失败后跳过尚未开始的场景，批次按 `completed` 收尾（与 `stop` 区分） |

`skip_disabled`：套件里被禁用的成员不执行，但仍会出现在报告中（标记跳过）。

## 4. 状态机

批次状态：

```
pending → running → completed
              ├──→ failed
              ├──→ cancelled
              ├──→ paused ──(恢复)──→ running
              └──→ interrupted ──(恢复 / 重跑)
```

明细状态：`pending / running / passed / failed / skipped / cancelled / interrupted`。

- `failed`：真实执行失败（请求错误 / 断言不过）；
- `skipped`：按失败策略未执行（不是失败）；
- `cancelled`：用户取消，或在途请求被协作式取消；
- `interrupted`：应用异常退出 / 进程被杀后，由启动恢复标记（不是失败）。

## 5. 暂停 / 恢复 / 取消

- **暂停**：不伪造「任意时刻冻结」。串行时当前场景跑完即停；并行时当前波次
  内已在途的场景跑完即停，尚未开始的保持 `pending`。批次状态置 `paused`。
- **恢复**：把 `pending` / `running` / `interrupted` 明细重置为 `pending` 后续跑；
  已完成（通过 / 失败 / 跳过）的明细保持原状，报告不丢已完成部分。
- **取消**：取消令牌传给单场景引擎，在步间 / 在途请求处协作式中止；未开始项
  标记 `cancelled`，批次状态置 `cancelled`。

批次之间互相隔离：每个批次持有独立的取消令牌与暂停标志，进度事件按
`batch_id` 过滤，不会出现跨批次脏状态。

## 6. 崩溃恢复

应用启动时（`lib.rs` setup）调用 `recover_interrupted_batches`：

- 把上次遗留的 `pending / running / paused` 批次标记为 `interrupted`；
- 其中 `pending / running` 明细同样标记 `interrupted`，已完成明细保留。

UI 会提示「上次执行被中断」，用户可以选择**继续执行**（同批次续跑）或
**整批重跑**（新建批次）。

## 7. 重试与重跑

- 失败项重试（单个或全部失败项）：**新建批次**，`parent_batch_id` 指向原批次，
  只跑失败场景；
- 整批重跑：**新建批次**，套件批次优先复用套件（成员可能已更新），手动 / 目录
  批次复用原场景清单。

任何重试 / 重跑都不会覆盖或修改历史批次记录。

## 8. 进度事件与前端状态

后端在每个场景开始 / 结束及每个波次结束时推送 `miyo:test-batch-progress`：

```ts
{
  batch_id, status,
  total, done, passed, failed, skipped, cancelled, running,
  current?: { item_id, scenario_id, scenario_name },
  item?: TestRunBatchItem
}
```

前端 `stores/testSuites.ts` 维护在途批次、事件增量与最终报告；终态事件会回读
`get_test_batch`（以 DB 为准）并刷新历史列表。

## 9. 命令清单

| 命令 | 说明 |
| --- | --- |
| `list_test_suites` / `save_test_suite` / `delete_test_suite` / `duplicate_test_suite` | 套件 CRUD 与复制 |
| `reorder_test_suite_items` / `set_test_suite_item_enabled` | 成员排序 / 启停 |
| `run_test_batch` | 启动批量执行（返回最终报告） |
| `pause_test_batch` / `resume_test_batch` / `cancel_test_batch` | 暂停 / 恢复 / 取消 |
| `list_test_batches` / `get_test_batch` / `delete_test_batch` | 历史批次 |
| `recover_test_batches` | 手动触发启动恢复（幂等） |

## 10. 自动化测试

| 位置 | 覆盖 |
| --- | --- |
| `crates/miyo-test/src/batch.rs` | 8 场景串行、8 场景并行（并发上限）、失败继续、失败停止、失败重试、取消、暂停、跳过后续、进度计数 |
| `crates/miyo-storage/src/repository/test_suites.rs` | 套件 CRUD / 复制 / 重排 / 启停、批次落库、环境切换重跑保留历史、套件删除后报告可读、启动恢复幂等 |
| `crates/miyo-tauri/tests/permissions_consistency.rs` | 新命令均注册了 `allow-*` 权限 |
| `frontend/src/domains/integration/stores/testSuites.spec.ts` | 启动入参、事件按批次隔离、失败重试 / 整批重跑新建批次、套件缓存同步 |

运行：

```bash
cargo test -p miyo-test -p miyo-storage
(cd crates/miyo-tauri && cargo test)
cd frontend && npx vitest run
```