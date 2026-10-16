import { useQueryClient } from "@tanstack/react-query";
import { clsx } from "clsx";
import { useMemo, useState } from "react";
import { Link, useNavigate } from "react-router";
import type { DatasetStatus, RunSummary } from "../api/generated/model";
import { getListRunsQueryKey, useCreateRun, useListDatasets, useListRuns } from "../api/generated/sync/sync";
import {
  Badge,
  buttonClass,
  Card,
  Empty,
  errorText,
  Field,
  inputClass,
  PageHeader,
  ProgressBar,
  QueryView,
  Segmented,
  Stat,
  Table,
} from "../components/ui";
import { fmtAgo, fmtDateTime, fmtDuration, fmtInt } from "../lib/format";
import { PRIORITIES, RUN_KINDS, RUN_STATUS } from "../lib/labels";

export function SyncPage() {
  const runs = useListRuns({ limit: 50 }, { query: { refetchInterval: 5000, staleTime: 0 } });
  const datasets = useListDatasets({ query: { refetchInterval: 15_000, staleTime: 0 } });
  const [creating, setCreating] = useState(false);

  return (
    <>
      <PageHeader
        title="数据同步"
        desc="从同花顺同步到本地数据库的运行和数据集状态。每个交易日 18:00 自动发起增量运行，历史回补在后台排队执行。"
        extra={
          <div className="flex items-center gap-3">
            <Link to="/indices" className="text-sm text-zinc-500 hover:text-zinc-900 dark:hover:text-zinc-100">
              指数与板块 →
            </Link>
            <button type="button" className={buttonClass()} onClick={() => setCreating((v) => !v)}>
              {creating ? "收起" : "发起运行"}
            </button>
          </div>
        }
      />
      {datasets.data && <Overview datasets={datasets.data} runs={runs.data ?? []} />}
      {creating && datasets.data && <CreateRunForm datasets={datasets.data} onDone={() => setCreating(false)} />}
      <Card title="最近的运行" extra={<span className="text-xs text-zinc-400">每 5 秒刷新</span>}>
        <QueryView query={runs}>{(rows) => <RunTable rows={rows} />}</QueryView>
      </Card>
      <Card title="数据集">
        <QueryView query={datasets}>{(rows) => <DatasetTable rows={rows} />}</QueryView>
      </Card>
    </>
  );
}

function Overview({ datasets, runs }: { datasets: DatasetStatus[]; runs: RunSummary[] }) {
  const open = datasets.reduce((n, d) => n + d.open_tasks, 0);
  const dead = datasets.reduce((n, d) => n + d.dead_tasks, 0);
  const enabled = datasets.filter((d) => d.enabled).length;
  const running = runs.filter((r) => r.status === "running").length;
  // 实测吞吐约每秒 1.7 个任务，见 docs/architecture/sync_pipeline.md#rate_limit
  const etaHours = open / 1.7 / 3600;
  return (
    <Card>
      <div className="grid grid-cols-2 gap-4 sm:grid-cols-4">
        <Stat
          label="待执行任务"
          value={fmtInt(open)}
          sub={
            open > 0
              ? `按每秒约 1.7 个估算，还需约 ${etaHours < 1 ? "不到 1" : etaHours.toFixed(0)} 小时`
              : "队列已清空"
          }
        />
        <Stat
          label="失败任务"
          value={<span className={dead > 0 ? "text-red-600 dark:text-red-400" : ""}>{fmtInt(dead)}</span>}
          sub="不再重试，需要人工处理"
        />
        <Stat label="进行中的运行" value={fmtInt(running)} />
        <Stat label="已开启的数据集" value={`${enabled} / ${datasets.length}`} />
      </div>
    </Card>
  );
}

function RunTable({ rows }: { rows: RunSummary[] }) {
  if (rows.length === 0) return <Empty>还没有运行</Empty>;
  return (
    <Table>
      <thead>
        <tr>
          <th>运行</th>
          <th>类型</th>
          <th>交易日</th>
          <th>状态</th>
          <th className="w-56">进度</th>
          <th>数据集</th>
          <th>开始</th>
          <th>耗时</th>
        </tr>
      </thead>
      <tbody>
        {rows.map((r) => {
          const st = RUN_STATUS[r.status] ?? { label: r.status, tone: "muted" as const };
          const ds = (r.params as { datasets?: string[] }).datasets ?? [];
          const root = (r.params as { root_run?: number | null }).root_run;
          return (
            <tr key={r.run_id} className="hover:bg-zinc-50 dark:hover:bg-zinc-800/50">
              <td className="num">
                <Link to={`/sync/runs/${r.run_id}`} className="text-accent hover:underline">
                  #{r.run_id}
                </Link>
                {root && <span className="ml-1 text-xs text-zinc-400">（接续 #{root}）</span>}
              </td>
              <td>{RUN_KINDS[r.kind] ?? r.kind}</td>
              <td className="num">{r.trade_date ?? "—"}</td>
              <td>
                <Badge tone={st.tone}>{st.label}</Badge>
              </td>
              <td>
                <div className="flex flex-col gap-1">
                  <ProgressBar done={r.tasks_done} failed={r.tasks_failed} total={r.tasks_total} />
                  <span className="num text-xs text-zinc-500">
                    {fmtInt(r.tasks_done + r.tasks_failed)} / {fmtInt(r.tasks_total)}
                    {r.tasks_failed > 0 && <span className="text-red-500">（失败 {fmtInt(r.tasks_failed)}）</span>}
                  </span>
                </div>
              </td>
              <td className="max-w-64 truncate text-xs text-zinc-500" title={ds.join("\n")}>
                {ds.length <= 2 ? ds.join("、") : `${ds.slice(0, 2).join("、")} 等 ${ds.length} 个`}
              </td>
              <td className="num text-xs text-zinc-500">{fmtDateTime(r.started_at)}</td>
              <td className="num text-xs text-zinc-500">{fmtDuration(r.started_at, r.finished_at)}</td>
            </tr>
          );
        })}
      </tbody>
    </Table>
  );
}

function DatasetTable({ rows }: { rows: DatasetStatus[] }) {
  const [q, setQ] = useState("");
  const [filter, setFilter] = useState<"all" | "busy" | "dead" | "enabled">("enabled");
  const shown = useMemo(
    () =>
      rows.filter(
        (d) =>
          (!q || d.dataset_id.includes(q) || d.endpoint.includes(q)) &&
          (filter === "all" ||
            (filter === "enabled" && d.enabled) ||
            (filter === "busy" && d.open_tasks > 0) ||
            (filter === "dead" && d.dead_tasks > 0)),
      ),
    [rows, q, filter],
  );
  return (
    <>
      <div className="mb-3 flex flex-wrap items-center gap-3">
        <input
          className={`${inputClass} w-56`}
          placeholder="数据集 ID 或接口路径"
          value={q}
          onChange={(e) => setQ(e.target.value)}
        />
        <Segmented
          value={filter}
          onChange={setFilter}
          options={[
            { value: "enabled", label: "已开启" },
            { value: "busy", label: "有待执行任务" },
            { value: "dead", label: "有失败任务" },
            { value: "all", label: "全部" },
          ]}
        />
      </div>
      {shown.length === 0 ? (
        <Empty>没有符合条件的数据集</Empty>
      ) : (
        <Table>
          <thead>
            <tr>
              <th>数据集</th>
              <th>优先级</th>
              <th>状态</th>
              <th>最近完成</th>
              <th className="text-right!">待执行</th>
              <th className="text-right!">失败</th>
            </tr>
          </thead>
          <tbody>
            {shown.map((d) => (
              <tr key={d.dataset_id} className={clsx(!d.enabled && "opacity-50")}>
                <td>
                  <div className="font-mono text-xs">{d.dataset_id}</div>
                  <div className="text-xs text-zinc-400">{d.endpoint}</div>
                </td>
                <td>
                  <span title={PRIORITIES[d.priority]}>{d.priority}</span>
                </td>
                <td>{d.enabled ? <Badge tone="ok">开启</Badge> : <Badge>关闭</Badge>}</td>
                <td className="text-xs text-zinc-500" title={fmtDateTime(d.last_done_at)}>
                  {fmtAgo(d.last_done_at)}
                </td>
                <td className="num text-right">{d.open_tasks > 0 ? fmtInt(d.open_tasks) : "—"}</td>
                <td className={clsx("num text-right", d.dead_tasks > 0 && "text-red-600 dark:text-red-400")}>
                  {d.dead_tasks > 0 ? fmtInt(d.dead_tasks) : "—"}
                </td>
              </tr>
            ))}
          </tbody>
        </Table>
      )}
    </>
  );
}

function CreateRunForm({ datasets, onDone }: { datasets: DatasetStatus[]; onDone: () => void }) {
  const [kind, setKind] = useState<"incremental" | "backfill">("incremental");
  const [mode, setMode] = useState<"priority" | "datasets">("priority");
  const [priorities, setPriorities] = useState<string[]>(["P0"]);
  const [picked, setPicked] = useState<string[]>([]);
  const [codes, setCodes] = useState("");
  const [q, setQ] = useState("");
  const navigate = useNavigate();
  const qc = useQueryClient();
  const create = useCreateRun({
    mutation: {
      onSuccess: (r) => {
        qc.invalidateQueries({ queryKey: getListRunsQueryKey() });
        onDone();
        navigate(`/sync/runs/${r.run_id}`);
      },
    },
  });
  const thscodes = codes
    .split(/[\s,，、]+/)
    .map((c) => c.trim().toUpperCase())
    .filter(Boolean);
  const enabled = datasets.filter((d) => d.enabled);
  const selectable = enabled.filter((d) => !q || d.dataset_id.includes(q));
  const valid = mode === "priority" ? priorities.length > 0 : picked.length > 0;
  const toggle = (list: string[], v: string) => (list.includes(v) ? list.filter((x) => x !== v) : [...list, v]);

  return (
    <Card title="发起运行">
      <form
        className="flex flex-col gap-4"
        onSubmit={(e) => {
          e.preventDefault();
          create.mutate({
            data: {
              kind,
              datasets: mode === "datasets" ? picked : [],
              priorities: mode === "priority" ? priorities : [],
              thscodes,
            },
          });
        }}
      >
        <div className="flex flex-wrap gap-6">
          <Field label="类型">
            <Segmented
              value={kind}
              onChange={setKind}
              options={[
                { value: "incremental", label: "增量", hint: "从上次同步到的位置往后补" },
                { value: "backfill", label: "历史回补", hint: "从数据集配置的起点全部重取，任务量很大" },
              ]}
            />
          </Field>
          <Field label="选择数据集">
            <Segmented
              value={mode}
              onChange={setMode}
              options={[
                { value: "priority", label: "按优先级" },
                { value: "datasets", label: "指定数据集" },
              ]}
            />
          </Field>
        </div>
        {mode === "priority" ? (
          <div className="flex flex-wrap gap-3">
            {Object.entries(PRIORITIES).map(([p, label]) => (
              <label key={p} className="flex items-center gap-1.5 text-sm">
                <input
                  type="checkbox"
                  checked={priorities.includes(p)}
                  onChange={() => setPriorities(toggle(priorities, p))}
                />
                {label}
                <span className="text-xs text-zinc-400">（{enabled.filter((d) => d.priority === p).length} 个）</span>
              </label>
            ))}
          </div>
        ) : (
          <div className="flex flex-col gap-2">
            <input
              className={`${inputClass} w-64`}
              placeholder="筛选数据集"
              value={q}
              onChange={(e) => setQ(e.target.value)}
            />
            <div className="grid max-h-56 grid-cols-1 gap-1 overflow-y-auto rounded-lg border border-zinc-200 p-2 sm:grid-cols-2 lg:grid-cols-3 dark:border-zinc-700">
              {selectable.map((d) => (
                <label key={d.dataset_id} className="flex items-center gap-1.5 font-mono text-xs">
                  <input
                    type="checkbox"
                    checked={picked.includes(d.dataset_id)}
                    onChange={() => setPicked(toggle(picked, d.dataset_id))}
                  />
                  {d.dataset_id}
                  <span className="text-zinc-400">{d.priority}</span>
                </label>
              ))}
            </div>
            <span className="text-xs text-zinc-500">已选 {picked.length} 个</span>
          </div>
        )}
        <Field
          label="只同步这些股票（可选）"
          hint="作用于按股票展开的数据集，例如重新拉取个别股票；多个代码用空格或逗号分隔"
        >
          <input
            className={`${inputClass} max-w-md`}
            placeholder="600519.SH 000001.SZ"
            value={codes}
            onChange={(e) => setCodes(e.target.value)}
          />
        </Field>
        {kind === "backfill" && thscodes.length === 0 && (
          <p className="rounded-lg bg-amber-50 p-3 text-sm text-amber-800 dark:bg-amber-500/10 dark:text-amber-300">
            全市场历史回补可能产生数万到数十万个任务，要执行一两天。已在队列中的同一分区不会重复创建。
          </p>
        )}
        {create.error && <p className="text-sm text-red-600 dark:text-red-400">{errorText(create.error)}</p>}
        <div className="flex gap-2">
          <button type="submit" className={buttonClass()} disabled={!valid || create.isPending}>
            {create.isPending ? "规划中…" : "规划并开始"}
          </button>
          <button type="button" className={buttonClass("ghost")} onClick={onDone}>
            取消
          </button>
        </div>
      </form>
    </Card>
  );
}
