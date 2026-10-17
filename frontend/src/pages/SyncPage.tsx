import { useQueryClient } from "@tanstack/react-query";
import { clsx } from "clsx";
import { useMemo, useState } from "react";
import { Link, useNavigate } from "react-router";
import type { DatasetStatus, Gap, RunSummary, SyncHealth } from "../api/generated/model";
import {
  getListRunsQueryKey,
  useCreateRun,
  useListDatasets,
  useListRuns,
  useSyncHealth,
  useSyncRepair,
} from "../api/generated/sync/sync";
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
  Spinner,
  Table,
} from "../components/ui";
import { fmtAgo, fmtDateTime, fmtDuration, fmtInt } from "../lib/format";
import { DATASET_CATEGORIES, PRIORITIES, RUN_KINDS, RUN_STATUS, SCHEDULES } from "../lib/labels";

// @project-doc docs/architecture/frontend.md#pages
/** 数据同步：最上面是状态、同步记录和手动同步（最常用），然后是要处理的问题，最后按分类看每类数据 */
export function SyncPage() {
  const health = useSyncHealth({ query: { refetchInterval: 15_000, staleTime: 0 } });
  const datasets = useListDatasets({ query: { refetchInterval: 30_000, staleTime: 0 } });
  const runs = useListRuns({ limit: 60 }, { query: { refetchInterval: 10_000, staleTime: 0 } });
  const [creating, setCreating] = useState(false);

  return (
    <>
      <PageHeader
        title="数据同步"
        desc="每个交易日 18:00（上海时间）自动同步当天的数据；之后自动核对最近 20 个交易日的数据是否齐全，能补的自动补。"
        extra={
          <div className="flex flex-wrap items-center gap-2">
            <QuickSync />
            <button type="button" className={buttonClass("ghost")} onClick={() => setCreating((v) => !v)}>
              {creating ? "收起" : "自定义同步…"}
            </button>
            <Link to="/indices" className="text-sm text-zinc-500 hover:text-zinc-900 dark:hover:text-zinc-100">
              指数与板块 →
            </Link>
          </div>
        }
      />
      {creating && datasets.data && <CreateRunForm datasets={datasets.data} onDone={() => setCreating(false)} />}
      {health.isPending ? <Spinner /> : health.data && <Status h={health.data} />}
      <Card title="同步记录" extra={<span className="text-xs text-zinc-400">每 10 秒刷新，点一行看详情</span>}>
        <QueryView query={runs}>{(rows) => <History rows={rows} />}</QueryView>
      </Card>
      {health.data && (health.data.gaps.length > 0 || health.data.failures.length > 0) && <Problems h={health.data} />}
      <Card
        title="数据"
        extra={<span className="text-xs text-zinc-400">按分类，每类数据是什么、多久更新一次、现在怎么样</span>}
      >
        <QueryView query={datasets}>{(rows) => <Datasets rows={rows} />}</QueryView>
      </Card>
    </>
  );
}

/** 一键增量同步每天都要的数据（P0、P1）：不用等到 18:00，例如刚部署或刚补好 API Key 时 */
function QuickSync() {
  const qc = useQueryClient();
  const navigate = useNavigate();
  const create = useCreateRun({
    mutation: {
      onSuccess: (r) => {
        qc.invalidateQueries({ queryKey: getListRunsQueryKey() });
        navigate(`/sync/runs/${r.run_id}`);
      },
    },
  });
  return (
    <>
      <button
        type="button"
        className={buttonClass()}
        disabled={create.isPending}
        title="增量同步核心和常用数据（P0、P1），从上次同步到的位置往后补；已经在队列里的不会重复"
        onClick={() =>
          create.mutate({ data: { kind: "incremental", datasets: [], priorities: ["P0", "P1"], thscodes: [] } })
        }
      >
        {create.isPending ? "规划中…" : "立即同步"}
      </button>
      {!!create.error && <span className="text-sm text-red-600">{errorText(create.error)}</span>}
    </>
  );
}

/** 顶部一句话：最近一个交易日的数据是否就绪，以及历史回补的进度 */
function Status({ h }: { h: SyncHealth }) {
  const tone = {
    ready: "bg-emerald-500",
    running: "bg-accent animate-pulse",
    incomplete: "bg-amber-500",
    not_started: "bg-zinc-400",
  }[h.daily_state];
  const b = h.backfill;
  return (
    <Card>
      <div className="flex flex-col gap-2 text-sm">
        <p className="flex items-center gap-2 text-base font-medium">
          <span className={clsx("size-2.5 rounded-full", tone)} />
          {h.message}
        </p>
        {b && b.total > 0 && (
          <div className="flex flex-col gap-1">
            <p className="text-zinc-600 dark:text-zinc-400">
              正在补历史数据：{fmtInt(b.done)} / {fmtInt(b.total)}（{((b.done / b.total) * 100).toFixed(0)}%）
              {b.eta_hours !== null &&
                b.eta_hours !== undefined &&
                `，按现在的速度还要约 ${Math.ceil(b.eta_hours)} 小时`}
              。补历史排在每日数据后面，不影响每天的同步。
            </p>
            <div className="max-w-md">
              <ProgressBar done={b.done} failed={0} total={b.total} />
            </div>
          </div>
        )}
        {h.last_repair_at && (
          <p className="text-xs text-zinc-400">上次自动核对和补齐：{fmtDateTime(h.last_repair_at)}</p>
        )}
      </div>
    </Card>
  );
}

/** 缺口和失败：能补的一键补，补不回来的如实说明 */
function Problems({ h }: { h: SyncHealth }) {
  const qc = useQueryClient();
  const repair = useSyncRepair({ mutation: { onSuccess: () => qc.invalidateQueries() } });
  const recoverable = h.gaps.filter((g) => g.recoverable);
  const lost = h.gaps.filter((g) => !g.recoverable);
  return (
    <Card
      title="需要注意"
      extra={
        <button
          type="button"
          className={buttonClass()}
          disabled={repair.isPending}
          title="能补的缺口安排补偿同步，供应商暂时不可用的失败再试一次；日期本来就没有数据的失败（节假日、超出一年）改为跳过"
          onClick={() => repair.mutate()}
        >
          {repair.isPending ? "处理中…" : "立即补齐"}
        </button>
      }
    >
      <div className="flex flex-col gap-4 text-sm">
        {repair.data && (
          <p className="text-emerald-700 dark:text-emerald-400">
            已安排 {repair.data.runs.length} 个补偿同步，重新排队 {repair.data.retried}{" "}
            个失败的任务，会排在每日数据之后执行
            {repair.data.cleared > 0 && `；${repair.data.cleared} 个日期本来就没有数据的失败任务已改为跳过`}。
          </p>
        )}
        {!!repair.error && <p className="text-red-600">{errorText(repair.error)}</p>}
        {recoverable.length > 0 && (
          <GapList title="缺少的数据（可以补，每天同步后会自动补）" gaps={recoverable} tone="warn" />
        )}
        {lost.length > 0 && <GapList title="补不回来的数据（只有当天的值，错过了就没有了）" gaps={lost} tone="bad" />}
        {h.failures.length > 0 && (
          <div>
            <h3 className="mb-1 text-xs font-medium text-zinc-500">失败的任务</h3>
            <ul className="flex flex-col gap-1">
              {h.failures.map((f) => (
                <li key={`${f.dataset_id}-${f.sample_error}`} className="flex flex-wrap items-baseline gap-x-2">
                  <span className="font-medium">{f.title}</span>
                  <span className="text-zinc-500">
                    {fmtInt(f.tasks)} 个：{f.reason}
                  </span>
                  <span className="truncate text-xs text-zinc-400" title={f.sample_error}>
                    {f.sample_error}
                  </span>
                </li>
              ))}
            </ul>
          </div>
        )}
      </div>
    </Card>
  );
}

function GapList({ title, gaps, tone }: { title: string; gaps: Gap[]; tone: "warn" | "bad" }) {
  return (
    <div>
      <h3
        className={clsx(
          "mb-1 text-xs font-medium",
          tone === "bad" ? "text-red-600 dark:text-red-400" : "text-amber-700 dark:text-amber-400",
        )}
      >
        {title}
      </h3>
      <ul className="flex flex-col gap-1">
        {gaps.slice(0, 20).map((g) => (
          <li key={`${g.trade_date}-${g.dataset_id}-${g.kind}`} className="flex flex-wrap items-baseline gap-x-2">
            <span className="num text-zinc-500">{g.trade_date}</span>
            <span className="font-medium">{g.title}</span>
            <span className="text-xs text-zinc-500">{g.message}</span>
          </li>
        ))}
        {gaps.length > 20 && <li className="text-xs text-zinc-400">还有 {gaps.length - 20} 项</li>}
      </ul>
    </div>
  );
}

/** 一个数据集的状态，用一个词说清 */
function datasetState(d: DatasetStatus): {
  label: string;
  tone: "ok" | "warn" | "bad" | "info" | "muted";
  hint?: string;
} {
  if (!d.enabled) return { label: "未开启", tone: "muted" };
  if (d.dead_tasks > 0) return { label: `${fmtInt(d.dead_tasks)} 个失败`, tone: "bad", hint: "见上方「需要注意」" };
  if (d.backfill_total > 0 && d.backfill_done < d.backfill_total)
    return {
      label: `补历史 ${Math.floor((d.backfill_done / d.backfill_total) * 100)}%`,
      tone: "info",
      hint: `${fmtInt(d.backfill_done)} / ${fmtInt(d.backfill_total)}`,
    };
  if (d.open_tasks > 0) return { label: "同步中", tone: "info" };
  if (!d.last_done_at) return { label: "还没同步过", tone: "warn" };
  return { label: "正常", tone: "ok" };
}

function categoryOrder(c: string) {
  const i = DATASET_CATEGORIES.indexOf(c);
  return i < 0 ? DATASET_CATEGORIES.length : i;
}

function groupByCategory(list: DatasetStatus[]): [string, DatasetStatus[]][] {
  const m = new Map<string, DatasetStatus[]>();
  for (const d of [...list].sort((a, b) => categoryOrder(a.category) - categoryOrder(b.category))) {
    m.set(d.category, [...(m.get(d.category) ?? []), d]);
  }
  return [...m];
}

/** 按分类分组；没有开启的数据集收在最后 */
function Datasets({ rows }: { rows: DatasetStatus[] }) {
  const [showOff, setShowOff] = useState(false);
  const on = useMemo(() => groupByCategory(rows.filter((d) => d.enabled)), [rows]);
  const off = useMemo(() => groupByCategory(rows.filter((d) => !d.enabled)), [rows]);
  const offCount = rows.filter((d) => !d.enabled).length;
  return (
    <div className="flex flex-col gap-5">
      {on.map(([cat, list]) => (
        <Category key={cat} name={cat} rows={list} />
      ))}
      {offCount > 0 && (
        <div>
          <button
            type="button"
            className="text-sm text-zinc-500 hover:text-zinc-900 dark:hover:text-zinc-100"
            onClick={() => setShowOff((v) => !v)}
          >
            {showOff ? "▾" : "▸"} 没有开启的数据（{offCount} 个：基金、期货、期权等，开启方法见数据目录文档）
          </button>
          {showOff && (
            <div className="mt-3 flex flex-col gap-5 opacity-70">
              {off.map(([cat, list]) => (
                <Category key={cat} name={cat} rows={list} />
              ))}
            </div>
          )}
        </div>
      )}
    </div>
  );
}

function Category({ name, rows }: { name: string; rows: DatasetStatus[] }) {
  return (
    <section>
      <h3 className="mb-1 text-xs font-medium text-zinc-500">
        {name} <span className="text-zinc-400">{rows.length}</span>
      </h3>
      <ul className="divide-y divide-zinc-100 rounded-lg border border-zinc-100 dark:divide-zinc-800 dark:border-zinc-800">
        {rows.map((d) => {
          const st = datasetState(d);
          return (
            <li key={d.dataset_id} className="flex flex-col gap-1 px-3 py-2 sm:flex-row sm:items-center sm:gap-4">
              <div className="min-w-0 flex-1">
                <div className="flex flex-wrap items-center gap-2">
                  <span className="font-medium" title={d.dataset_id}>
                    {d.title}
                  </span>
                  {d.only_current && (
                    <span
                      className="text-xs text-amber-700 dark:text-amber-400"
                      title="接口只返回当天的值，没有历史，错过当天就补不回来"
                    >
                      只有当天值
                    </span>
                  )}
                </div>
                <p className="text-xs text-zinc-500">{d.summary}</p>
              </div>
              <div className="flex shrink-0 items-center gap-3 text-xs text-zinc-500">
                <span className="w-24">{SCHEDULES[d.schedule] ?? d.schedule}</span>
                <span className="w-20" title={fmtDateTime(d.last_done_at)}>
                  {d.last_done_at ? fmtAgo(d.last_done_at) : "—"}
                </span>
                <span className="w-24 text-right" title={st.hint}>
                  <Badge tone={st.tone}>{st.label}</Badge>
                </span>
              </div>
            </li>
          );
        })}
      </ul>
    </section>
  );
}

/** 同步记录：一次同步可能分几个波次（有依赖的数据集要等前一批完成），这里合并成一行 */
function History({ rows }: { rows: RunSummary[] }) {
  const groups = useMemo(() => {
    const m = new Map<number, RunSummary[]>();
    for (const r of rows) {
      const root = (r.params as { root_run?: number | null }).root_run ?? r.run_id;
      m.set(root, [...(m.get(root) ?? []), r]);
    }
    return [...m].sort((a, b) => b[0] - a[0]).slice(0, 15);
  }, [rows]);
  if (groups.length === 0) return <Empty>还没有同步过</Empty>;
  return (
    <Table>
      <thead>
        <tr>
          <th>类型</th>
          <th>交易日</th>
          <th>状态</th>
          <th className="w-56">进度</th>
          <th>开始</th>
          <th>耗时</th>
        </tr>
      </thead>
      <tbody>
        {groups.map(([root, list]) => {
          const total = list.reduce((n, r) => n + r.tasks_total, 0);
          const done = list.reduce((n, r) => n + r.tasks_done, 0);
          const failed = list.reduce((n, r) => n + r.tasks_failed, 0);
          const running = list.some((r) => r.status === "running");
          const status = running
            ? "running"
            : list.some((r) => r.status === "partial")
              ? "partial"
              : (list[0]?.status ?? "succeeded");
          const st = RUN_STATUS[status] ?? { label: status, tone: "muted" as const };
          const first = list[list.length - 1] ?? list[0];
          const last = list[0];
          if (!first || !last) return null;
          return (
            <tr key={root} className="hover:bg-zinc-50 dark:hover:bg-zinc-800/50">
              <td>
                <Link to={`/sync/runs/${root}`} className="hover:underline">
                  {RUN_KINDS[first.kind] ?? first.kind}
                </Link>
                {list.length > 1 && <span className="ml-1 text-xs text-zinc-400">{list.length} 批</span>}
              </td>
              <td className="num">{first.trade_date ?? "—"}</td>
              <td>
                <Badge tone={st.tone}>{st.label}</Badge>
              </td>
              <td>
                <div className="flex flex-col gap-1">
                  <ProgressBar done={done} failed={failed} total={total} />
                  <span className="num text-xs text-zinc-500">
                    {fmtInt(done + failed)} / {fmtInt(total)}
                    {failed > 0 && <span className="text-red-500">（失败 {fmtInt(failed)}）</span>}
                  </span>
                </div>
              </td>
              <td className="num text-xs text-zinc-500">{fmtDateTime(first.started_at)}</td>
              <td className="num text-xs text-zinc-500">
                {running ? "进行中" : fmtDuration(first.started_at, last.finished_at)}
              </td>
            </tr>
          );
        })}
      </tbody>
    </Table>
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
  const selectable = enabled.filter((d) => !q || d.title.includes(q) || d.dataset_id.includes(q));
  const valid = mode === "priority" ? priorities.length > 0 : picked.length > 0;
  const toggle = (list: string[], v: string) => (list.includes(v) ? list.filter((x) => x !== v) : [...list, v]);

  return (
    <Card title="自定义同步：选择类型和数据">
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
                <label key={d.dataset_id} className="flex items-center gap-1.5 text-xs" title={d.dataset_id}>
                  <input
                    type="checkbox"
                    checked={picked.includes(d.dataset_id)}
                    onChange={() => setPicked(toggle(picked, d.dataset_id))}
                  />
                  {d.title}
                  <span className="text-zinc-400">{d.category}</span>
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
