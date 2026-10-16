import { Link, useParams } from "react-router";
import type { DatasetProgress, RunDetail, TaskFailure } from "../api/generated/model";
import { useGetRun } from "../api/generated/sync/sync";
import { Badge, Card, Empty, PageHeader, ProgressBar, QueryView, Stat, Table } from "../components/ui";
import { fmtDateTime, fmtDuration, fmtInt } from "../lib/format";
import { RUN_KINDS, RUN_STATUS, TASK_STATUS } from "../lib/labels";

export function RunDetailPage() {
  const { runId = "" } = useParams();
  const run = useGetRun(Number(runId), {
    query: { refetchInterval: (q) => (q.state.data?.status === "running" ? 5000 : false), staleTime: 0 },
  });
  return (
    <>
      <PageHeader
        title={`运行 #${runId}`}
        extra={
          <Link to="/sync" className="text-sm text-zinc-500 hover:text-zinc-900 dark:hover:text-zinc-100">
            ← 返回数据同步
          </Link>
        }
      />
      <QueryView query={run}>{(r) => <RunBody run={r} />}</QueryView>
    </>
  );
}

function RunBody({ run }: { run: RunDetail }) {
  const st = RUN_STATUS[run.status] ?? { label: run.status, tone: "muted" as const };
  const params = run.params as { pending_waves?: string[][]; root_run?: number | null; only_thscodes?: string[] };
  const pending = params.pending_waves ?? [];
  return (
    <>
      <Card>
        <div className="grid grid-cols-2 gap-4 sm:grid-cols-3 lg:grid-cols-6">
          <Stat label="状态" value={<Badge tone={st.tone}>{st.label}</Badge>} />
          <Stat
            label="类型"
            value={RUN_KINDS[run.kind] ?? run.kind}
            sub={run.trade_date ? `交易日 ${run.trade_date}` : undefined}
          />
          <Stat
            label="任务"
            value={`${fmtInt(run.tasks_done + run.tasks_failed)} / ${fmtInt(run.tasks_total)}`}
            sub={run.tasks_failed ? `失败 ${fmtInt(run.tasks_failed)}` : undefined}
          />
          <Stat label="开始" value={<span className="text-sm">{fmtDateTime(run.started_at)}</span>} />
          <Stat
            label="耗时"
            value={fmtDuration(run.started_at, run.finished_at)}
            sub={run.finished_at ? `结束于 ${fmtDateTime(run.finished_at)}` : "进行中，每 5 秒刷新"}
          />
          <Stat
            label="波次"
            value={params.root_run ? `接续 #${params.root_run}` : "第一个波次"}
            sub={pending.length > 0 ? `之后还有 ${pending.length} 个波次` : "之后没有波次"}
          />
        </div>
        {params.only_thscodes && params.only_thscodes.length > 0 && (
          <p className="mt-3 text-xs text-zinc-500">只同步：{params.only_thscodes.join("、")}</p>
        )}
        <p className="mt-3 text-xs text-zinc-400">
          有依赖关系的数据集分波次执行：一个波次的任务全部结束后，才规划下一个波次（新的运行编号）。
        </p>
      </Card>
      <Card title="各数据集进度">
        <ProgressTable rows={run.progress} />
      </Card>
      <Card title="失败的任务" extra={<span className="text-xs text-zinc-400">最多显示 50 条</span>}>
        <FailureTable rows={run.failures} />
      </Card>
    </>
  );
}

function ProgressTable({ rows }: { rows: DatasetProgress[] }) {
  if (rows.length === 0) return <Empty>这个运行没有任务</Empty>;
  return (
    <Table>
      <thead>
        <tr>
          <th>数据集</th>
          <th className="w-48">进度</th>
          <th className="text-right!">完成</th>
          <th className="text-right!">等待</th>
          <th className="text-right!">执行中</th>
          <th className="text-right!">失败</th>
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
            <td className="font-mono text-xs">{p.dataset_id}</td>
            <td>
              <ProgressBar done={p.done} failed={p.failed} total={p.tasks} />
            </td>
            <td className="num text-right">{fmtInt(p.done)}</td>
            <td className="num text-right">{fmtInt(p.pending)}</td>
            <td className="num text-right">{fmtInt(p.running)}</td>
            <td className="num text-right">
              {p.failed > 0 ? <span className="text-red-600 dark:text-red-400">{fmtInt(p.failed)}</span> : "—"}
            </td>
            <td className="num text-right">{fmtInt(p.records)}</td>
            <td className="num text-right">{fmtInt(p.changed)}</td>
          </tr>
        ))}
      </tbody>
    </Table>
  );
}

function FailureTable({ rows }: { rows: TaskFailure[] }) {
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
                <div className="font-mono text-xs">{t.dataset_id}</div>
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
