# 同步永远不结束：波次重复规划故障报告与线上热修

> 故障报告，不属于 Project Doc 文档库。2026-10-18 写，记录服务器 `/opt/niulai` 上「数据同步一直完不成」的根因、已执行的数据库热修，以及待发布的代码修复。

## 一、现象

- 服务器部署目录 `/opt/niulai`，镜像 `ghcr.io/duoyuli/niulai:2026.10.18.6`（运行 `niulai serve`，内置 PostgreSQL）。
- 网页同步记录里，`2026-10-16` 创建的 run 1 一直停在 `running`（3,083/3,084），另有多个回补运行同时处于 `running`。
- 待执行任务长期维持 29 万左右，吞吐约每秒 5 个任务，看起来「永远跑不完」；同步记录里还不断出现新的整波回补运行。

## 二、根因：波次接续不是幂等的

### 2.1 部署版本缺少修复提交

当前镜像 label 的 revision 是 `37b134f`（即标签 `v2026.10.18.6` 指向的提交）。修复这个问题的提交 `6c8dc09`（`Make sync chaining and scheduling survive transient failures`）晚于它，**从未进入任何已发布镜像**；本地 HEAD `189f57c` 已包含该修复。

### 2.2 有问题的调用链（部署版本 `37b134f`）

1. `backend/src/sync/planner.rs:130-147`（@37b134f）`continue_chain`：读取已结束运行 `params.pending_waves` 的第一个波次，调 `plan_wave` 创建新运行。
2. `backend/src/sync/planner.rs:151-199`（@37b134f）`plan_wave`：只 INSERT 新 `ops.run` 和任务，事务里没有清除、也没有条件抢占已结束运行的 `pending_waves`。
3. 部署版本里没有任何代码会再次写 `ops.run.params.pending_waves`，即「波次被消费」这个事实没有落库。
4. `backend/src/sync/executor.rs:247-265`（@37b134f）`after_task`：运行每次进入终态都调用 `continue_chain`；定期维护的 `run::stalled()` 也会对「状态还是 `running`、但已无未完成任务」的运行调用它。所以运行只要再进入一次终态，就会把同一个波次再规划一遍。
5. `backend/src/sync/audit.rs:406-428`（@37b134f）兜底补偿：把 `dead` 且错误以「可重试错误」开头的最新分区任务重置为 `pending`、`attempts = 0`，随后 `UPDATE ops.run SET status = 'running'` 把这条历史运行重新打开。

两者叠加：一条早已结束、带 `pending_waves` 的根运行被兜底补偿重开 → 再次结束 → 再规划一次同样的波次。由于任务去重只看「当前未完成的分区」，上一波已经完成的分区会被重新建一遍，回补任务越滚越多。

### 2.3 本次的触发条件

- run 1 第 0 波里只剩一条失败任务：`special.hot_stock_daily`，`date=2026-07-17`，供应商返回 5003 `Data source unavailable`。
- `2026-07-17` 在 `ref.trading_day` 里是交易日（`source = api`）；该数据集是 `calendar: natural` 且没有配置 `empty_codes`，5003 按可重试错误处理（`backend/src/vendor/client.rs:71-73`），重试 4 次后变 `dead`。
- 兜底补偿在最近一个交易日的每日同步之后会重排这类任务，于是 run 1 每天被重新打开一次；手动点「立即补齐」也会触发。

### 2.4 服务器实测证据

容器日志（上海时间）：

```
2026-10-18 10:36:01 任务失败 task_id=2984 dataset=special.hot_stock_daily part=date=2026-07-17
                    可重试错误（code=Some(5003)，http=Some(200)）：Data source unavailable
2026-10-18 10:36:01 运行结束，开始后处理 run_id=1 status=partial
2026-10-18 10:37:42 已规划 run_id=28 kind=backfill tasks=271624 datasets=11
2026-10-18 10:37:42 已规划下一个波次 run_id=1 next=28
2026-10-18 11:48:03 任务失败 task_id=2984 ...（11:47 的一次「立即补齐」又把它重排，run 1 被重新打开）
```

数据库统计（热修前）：

| 项目 | 数值 |
| --- | --- |
| run 1 这条链的后续波次运行 | run 9 / 10 / 20 / 22 / 28 共 5 个，合计 537,508 个任务 |
| 当前待执行任务 | 292,098 |
| 其中同一 `dataset_id + part_key` 已有成功写入的 | 275,215（94%） |
| 真正还没覆盖的 | 16,740（`income_q` / `balance_q` / `cashflow_q` 推到 10-18 的窗口） |
| 另一条完整 P0/P1 回补链 run 8 | 314,572/314,572，`2026-10-17 16:37 UTC` 已成功 |

结论：**数据层面其实早就同步完了**（run 8 完整成功），之后都是同一条根运行被反复重开后产生的重复回补。页面上看到的「一直同步不完」是重复任务和永远 `running` 的 run 1 造成的。

## 三、已在服务器执行的数据库热修

按「先止血、再发版」处理：不重启容器、不改镜像，只修正数据状态并加一个临时触发器挡住复发路径。

### 3.1 清空残留的 `pending_waves`

旧代码从不清理它；这 11 个运行的下一波此前都已经接上过，留着只会在再次结束时重复规划。

```sql
UPDATE ops.run
SET params = jsonb_set(params, '{pending_waves}', '[]'::jsonb)
WHERE jsonb_array_length(coalesce(params->'pending_waves','[]'::jsonb)) > 0;
```

命中 11 个运行（run 1、2、4、5、11、12、13、15、23、24、26）。

### 3.2 供应商缺数的单日分区标为跳过

```sql
UPDATE ops.task
SET status = 'skipped', finished_at = now(),
    last_error = '2026-07-17 供应商返回 5003 Data source unavailable，按无数据跳过（热修）'
WHERE dataset_id = 'special.hot_stock_daily'
  AND part_key = 'date=2026-07-17'
  AND status IN ('pending','failed','dead');
```

之后 run 1 没有未完成任务，正常结束为 `succeeded`（不再计入失败）。

### 3.3 跳过重复回补任务

判定标准：`status = 'pending'` 且同一 `dataset_id + part_key` 已经存在 `done` 或 `empty` 的任务。这些都是旧代码反复重开根运行产生的回补任务，数据已由 run 8/10 等运行写入；跳过不丢数据。

```sql
WITH dup AS (
  SELECT t.task_id
  FROM ops.task t
  WHERE t.status = 'pending'
    AND EXISTS (
      SELECT 1 FROM ops.task o
      WHERE o.dataset_id = t.dataset_id
        AND o.part_key = t.part_key
        AND o.task_id <> t.task_id
        AND o.status IN ('done','empty')
    )
)
UPDATE ops.task
SET status = 'skipped', finished_at = now(),
    last_error = '重复回补：同一分区已由之前的运行成功写入，热修跳过'
WHERE task_id IN (SELECT task_id FROM dup);
```

命中 275,215 行，耗时约 12 秒。

### 3.4 临时触发器：运行被重开时清空已消费的波次

只改数据挡不住以后：每天的增量同步同样会产生带 `pending_waves` 的根运行，旧代码依旧不清。发版前用一个临时触发器把这个行为补上，语义与 `6c8dc09` 的 `take_pending_waves` 一致：

```sql
CREATE OR REPLACE FUNCTION ops.clear_consumed_waves_on_reopen() RETURNS trigger AS $$
BEGIN
  IF NEW.status = 'running' AND OLD.status <> 'running'
     AND jsonb_array_length(coalesce(NEW.params->'pending_waves','[]'::jsonb)) > 0
     AND EXISTS (SELECT 1 FROM ops.run c WHERE (c.params->>'root_run')::bigint = NEW.run_id)
  THEN
    NEW.params := jsonb_set(NEW.params, '{pending_waves}', '[]'::jsonb);
  END IF;
  RETURN NEW;
END
$$ LANGUAGE plpgsql;

CREATE TRIGGER run_reopen_clears_consumed_waves
BEFORE UPDATE OF status ON ops.run
FOR EACH ROW EXECUTE FUNCTION ops.clear_consumed_waves_on_reopen();
```

条件里要求「已经存在以本运行 `root_run` 的子运行」，即这一波确实接出过；如果某一波因为后处理失败根本没接上，重开时不清空，保留旧代码偶然补救的路径。

已用事务 + `ROLLBACK` 验证两条分支：run 2（有子运行 run 3）重开时被清空；run 27（无子运行）重开时保留。

### 3.5 热修结果

| 项目 | 热修前 | 热修后 |
| --- | --- | --- |
| 待执行任务 | 292,098 | 16,326 |
| run 1 | running，3,083/3,084 | succeeded，3,084/3,084 |
| run 20 | running，39,485/69,184 | succeeded，69,184/69,184 |
| run 22 | running，6,113/9,273 | succeeded，9,273/9,273 |
| run 28 | running，12,381/271,624 | running，继续执行剩余约 1.6 万个真实任务 |
| 新出现的重复运行 | 每轮兜底补偿产生一个 | 无（`max(run_id)` 仍为 28） |

剩余的约 1.6 万个任务是 `income_q` / `balance_q` / `cashflow_q` 窗口推到 `2026-10-18` 的那一版，属于本次规划应有的工作，按每秒约 5 个任务预计 1 小时左右跑完，无需人工干预。热修后 10 分钟的接口调用里没有 4001（限流）和其他错误；5003 只来自 `fin_indicator`（预期，按无数据处理）和热股 `2026-07-17` 在热修前 03:53 UTC 的最后一次重试，热修后没有再请求这个分区。

### 3.6 回滚方式

- 删除临时触发器（代码发版后必须做）：

```sql
DROP TRIGGER IF EXISTS run_reopen_clears_consumed_waves ON ops.run;
DROP FUNCTION IF EXISTS ops.clear_consumed_waves_on_reopen();
```

- `pending_waves` 清空和任务标 `skipped` 都是状态修正：被清空的波次此前都已接上（由 run 9/10/20/22/28 实际执行），被跳过的任务同一分区都已有成功写入记录，不需要还原。

## 四、代码修复（已存在于本地仓库，待发版）

提交 `6c8dc09`：

- `backend/src/sync/run.rs:113-123`：新增 `take_pending_waves`，用条件 UPDATE 清空 `pending_waves`；`:127-136` 新增 `unchained` 供维护补接续。
- `backend/src/sync/planner.rs:171-175`：`continue_chain` 传出 `prev_run`，在创建新运行的同一个事务里抢占并清空上一运行的 `pending_waves`，抢不到返回 `None`（语义变为幂等）。
- `backend/src/sync/executor.rs:128-132`：维护里补接续；`:259-264`：`after_task` 改为先接续、再做后处理，避免后处理失败把后续波次一起丢掉。
- `backend/migrations/20261018000007_sync_claim_and_chain.sql`：清空已结束运行残留的 `pending_waves`；认领索引改为 `(priority, task_id)`。
- 注意：该迁移的清理条件是 `status <> 'running'`，被补偿重开成 `running` 的 run 1 不会被它清掉；本次热修已手工清空。

修复后行为：每个波次只会被规划一次；运行即使被兜底补偿重新打开，也不会再重复规划已经接上的波次。

## 五、遗留问题

1. **兜底补偿对「历史日期 + 永久 5003」没有跨轮次上限**。发版后不会再复制整波，但 `special.hot_stock_daily` 的 `2026-07-17` 仍会每天被重排一次，run 1 会反复 `running → partial`。可选处理：给该数据集配置 `empty_codes: [5003]`（代价是瞬时故障会被当成无数据），或让补偿只重排最近 N 个交易日的可重试失败。本次热修已把该分区标为 `skipped`，暂时不会再被重排（分区后续没有新的失败任务）。
2. **3 条 `dead` 写入错误**（数据不缺，所在分区后来都成功过，会被保留期清理，可暂不处理）：
   - `mkt.stock_bar_1d` 价格列 `NUMERIC(12,4)` 溢出：`000066.SZ`、`000863.SZ` 的 `2005-01-01..2014-12-29` 窗口。
   - `mkt.index_bar_1d.volume` 为 NULL（列 `NOT NULL`）：`886082.TI` 的 `2022-01-01..2026-10-16` 窗口。
3. **文档与线上行为不一致**：`docs/architecture/sync_pipeline.md` 的「波次规划」一节描述的是修复后的语义（接续时在同一事务里清空 `pending_waves`），而线上跑的是旧代码。发版后即一致，无需改文档。
4. 服务器上还保留了本次热修安装的临时触发器，**发版后按 3.6 删除**。

## 六、后续动作

1. 发布包含 `6c8dc09`（及之后提交）的新镜像并一键升级，让触发器可以移除。
2. 视情况处理 `special.hot_stock_daily` 的 5003 口径（见遗留问题 1）。
3. 发版后删除临时触发器，并复核 `ops.run` 是否还有非空 `pending_waves` 的 `running` 运行。
