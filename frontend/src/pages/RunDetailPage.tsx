import { useMemo } from "react";
import { Link, useParams } from "react-router";
import type { DatasetProgress, DatasetWithoutTasks, RunDetail, TaskFailure } from "../api/generated/model";
import { useGetRun, useListDatasets } from "../api/generated/sync/sync";
import { ChainProgress } from "../components/ChainProgress";
import { Badge, Card, Empty, PageHeader, ProgressBar, QueryView, Stat, Table } from "../components/ui";
import { fmtDateTime, fmtDuration, fmtInt } from "../lib/format";
import { RUN_STATUS, runSource, TASK_STATUS } from "../lib/labels";
import { summarizeChain } from "../lib/runs";

// @project-doc docs/architecture/frontend.md#pages
/** 一次同步的详情：几批合在一起看，数字和同步记录里那一行一致 */
export function RunDetailPage() {
  const { runId = "" } = useParams();
  const run = useGetRun(Number(runId), {
    query: {
      refetchInterval: (q) => (q.state.data && summarizeChain(q.state.data.waves)?.status === "running" ? 5000 : false),
      staleTime: 0,
    },
  });
  const datasets = useListDatasets();
  const titles = useMemo(() => new Map((datasets.data ?? []).map((d) => [d.dataset_id, d.title])), [datasets.data]);
  return <QueryView query={run}>{(r) => <RunBody run={r} titles={titles} />}</QueryView>;
}

function RunBody({ run, titles }: { run: RunDetail; titles: Map<string, string> }) {
  const c = summarizeChain(run.waves);
  if (!c) return <Empty>这次同步不存在</Empty>;
  const st = RUN_STATUS[c.status] ?? { label: c.status, tone: "muted" as const };
  const name = (id: string) => titles.get(id) ?? id;
  const only = (run.params as { only_thscodes?: string[] }).only_thscodes ?? [];
  const batches = run.waves.length + run.waiting.length;
  return (
    <>
      <PageHeader
        title={`${runSource(run.kind, run.params)}${run.trade_date ? ` · ${run.trade_date}` : ""}`}
        desc={`同步 #${c.root}`}
        extra={
          <Link to="/sync" className="text-sm text-zinc-500 hover:text-zinc-900 dark:hover:text-zinc-100">
            ← 返回数据同步
          </Link>
        }
      />
      <Card>
        <div className="grid grid-cols-2 gap-4 sm:grid-cols-3 lg:grid-cols-5">
          <Stat label="状态" value={<Badge tone={st.tone}>{st.label}</Badge>} />
          <Stat label="任务" value={<ChainProgress c={c} />} />
          <Stat label="开始" value={<span className="text-sm">{fmtDateTime(c.first.started_at)}</span>} />
          <Stat
            label="耗时"
            value={fmtDuration(c.first.started_at, c.finishedAt)}
            sub={c.finishedAt ? `结束于 ${fmtDateTime(c.finishedAt)}` : "进行中，每 5 秒刷新"}
          />
          <Stat
            label="分批"
            value={`${batches} 批`}
            sub={run.waiting.length > 0 ? `还有 ${run.waiting.length} 批等前一批结束` : undefined}
          />
        </div>
        {only.length > 0 && <p className="mt-3 text-xs text-zinc-500">只同步：{only.join("、")}</p>}
      </Card>
      {batches > 1 && (
        <Card
          title="分批执行"
          extra={<span className="text-xs text-zinc-400">后一批要用到前一批的结果，前一批全部结束后才开始</span>}
        >
          <ol className="flex flex-col divide-y divide-zinc-100 text-sm dark:divide-zinc-800">
            {run.waves.map((w, i) => {
              const ws = RUN_STATUS[w.status] ?? { label: w.status, tone: "muted" as const };
              const ids = (w.params as { datasets?: string[] }).datasets ?? [];
              return (
                <li key={w.run_id} className="flex flex-col gap-1 py-2 sm:flex-row sm:items-center sm:gap-4">
                  <span className="w-14 shrink-0 text-zinc-500">第 {i + 1} 批</span>
                  <span className="min-w-0 flex-1 text-xs text-zinc-600 dark:text-zinc-400">
                    {ids.map(name).join("、")}
                  </span>
                  <span className="flex shrink-0 items-center gap-2">
                    <span className="num text-xs text-zinc-500">
                      {fmtInt(w.tasks_done + w.tasks_failed)} / {fmtInt(w.tasks_total)}
                    </span>
                    <Badge tone={ws.tone}>{ws.label}</Badge>
                  </span>
                </li>
              );
            })}
            {run.waiting.map((ids, i) => (
              <li key={ids.join()} className="flex flex-col gap-1 py-2 sm:flex-row sm:items-center sm:gap-4">
                <span className="w-14 shrink-0 text-zinc-500">第 {run.waves.length + i + 1} 批</span>
                <span className="min-w-0 flex-1 text-xs text-zinc-600 dark:text-zinc-400">
                  {ids.map(name).join("、")}
                </span>
                <Badge>等前一批结束</Badge>
              </li>
            ))}
          </ol>
        </Card>
      )}
      <Card title="各数据集进度" extra={<span className="text-xs text-zinc-400">各批合计</span>}>
        <ProgressTable rows={run.progress} without={run.without_tasks} name={name} />
      </Card>
      <Card title="失败的任务" extra={<span className="text-xs text-zinc-400">包括等待重试的，最多显示 50 条</span>}>
        <FailureTable rows={run.failures} name={name} />
      </Card>
    </>
  );
}

function ProgressTable({
  rows,
  without,
  name,
}: {
  rows: DatasetProgress[];
  without: DatasetWithoutTasks[];
  name: (id: string) => string;
}) {
  if (rows.length === 0 && without.length === 0) return <Empty>这次同步还没有任务</Empty>;
  return (
    <div className="flex flex-col gap-3">
      {rows.length > 0 && (
        <Table>
          <thead>
            <tr>
              <th>数据</th>
              <th className="w-48">进度</th>
              <th className="text-right!" title="成功、没有数据或跳过">
                已结束
              </th>
              <th className="text-right!" title="包括失败后等待重试的">
                等待
              </th>
              <th className="text-right!">执行中</th>
              <th className="text-right!" title="不再重试的失败">
                失败
              </th>
              <th className="text-right!" title="接口返回的记录数">
                记录
              </th>
              <th className="text-right!" title="新增或值有变化的记录数">
                变化
              </th>
            </tr>
          </thead>
          <tbody>
            {rows.map((p) => (
              <tr key={p.dataset_id}>
                <td title={p.dataset_id}>{name(p.dataset_id)}</td>
                <td>
                  <ProgressBar done={p.done} failed={p.failed} total={p.tasks} />
                </td>
                <td className="num text-right">
                  {fmtInt(p.done)} / {fmtInt(p.tasks)}
                </td>
                <td className="num text-right">{p.pending > 0 ? fmtInt(p.pending) : "—"}</td>
                <td className="num text-right">{p.running > 0 ? fmtInt(p.running) : "—"}</td>
                <td className="num text-right">
                  {p.failed > 0 ? <span className="text-red-600 dark:text-red-400">{fmtInt(p.failed)}</span> : "—"}
                </td>
                <td className="num text-right">{fmtInt(p.records)}</td>
                <td className="num text-right">{fmtInt(p.changed)}</td>
              </tr>
            ))}
          </tbody>
        </Table>
      )}
      {without.length > 0 && (
        <div className="text-sm">
          <h3 className="mb-1 text-xs font-medium text-zinc-500">这次没有新建任务的数据（{without.length} 类）</h3>
          <ul className="flex flex-col gap-1">
            {without.map((w) => (
              <li key={w.dataset_id} className="flex flex-wrap items-baseline gap-x-2">
                <span title={w.dataset_id}>{name(w.dataset_id)}</span>
                <span className="text-xs text-zinc-500">
                  {w.queued_in ? (
                    <>
                      同样的内容已经在
                      <Link to={`/sync/runs/${w.queued_in}`} className="mx-1 text-accent hover:underline">
                        同步 #{w.queued_in}
                      </Link>
                      的队列里，不重复创建，在那里看进度
                    </>
                  ) : (
                    "没有需要同步的部分（已是最新）"
                  )}
                </span>
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}

function FailureTable({ rows, name }: { rows: TaskFailure[]; name: (id: string) => string }) {
  if (rows.length === 0) return <Empty>没有失败的任务</Empty>;
  return (
    <Table>
      <thead>
        <tr>
          <th>任务</th>
          <th>数据集 / 分区</th>
          <th>状态</th>
          <th className="text-right!">尝试</th>
          <th>错误码</th>
          <th>原因</th>
        </tr>
      </thead>
      <tbody>
        {rows.map((t) => {
          const st = TASK_STATUS[t.status] ?? { label: t.status, tone: "muted" as const };
          return (
            <tr key={t.task_id} className="align-top">
              <td className="num">{t.task_id}</td>
              <td>
                <div className="text-xs" title={t.dataset_id}>
                  {name(t.dataset_id)}
                </div>
                <div className="font-mono text-xs text-zinc-400">{t.part_key}</div>
              </td>
              <td>
                <Badge tone={st.tone}>{st.label}</Badge>
              </td>
              <td className="num text-right">{t.attempts}</td>
              <td className="num">{t.last_api_code ?? "—"}</td>
              <td className="max-w-md text-xs break-words whitespace-normal text-zinc-600 dark:text-zinc-400">
                {t.last_error ?? "—"}
              </td>
            </tr>
          );
        })}
      </tbody>
    </Table>
  );
}
