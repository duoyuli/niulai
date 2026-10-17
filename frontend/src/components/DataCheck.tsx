import { useQueryClient } from "@tanstack/react-query";
import { Link } from "react-router";
import type { CheckResult, DataCheckStatus } from "../api/generated/model";
import { getDataCheckStatusQueryKey, useDataCheckStatus, useStartDataCheck } from "../api/generated/sync/sync";
import { fmtDateTime, fmtInt } from "../lib/format";
import { buttonClass, Card, errorText, QueryView, Table } from "./ui";

const TRIGGERS: Record<string, string> = {
  weekly: "每周同步后自动",
  followup: "重取结束后复查",
  manual: "手动",
  cli: "命令行",
};

// @project-doc docs/architecture/sync_pipeline.md#data_check
/** 数据核检：按股票 × 交易日对照日线，缺口自动重取，接口仍然没有的确认为停牌 */
export function DataCheck() {
  const q = useDataCheckStatus({
    query: { refetchInterval: (x) => (x.state.data?.running ? 3000 : 30_000), staleTime: 0 },
  });
  const qc = useQueryClient();
  const start = useStartDataCheck({
    mutation: { onSettled: () => qc.invalidateQueries({ queryKey: getDataCheckStatusQueryKey() }) },
  });
  const running = q.data?.running || start.isPending;
  return (
    <Card
      title="数据核检"
      extra={
        <button type="button" className={buttonClass("ghost")} disabled={running} onClick={() => start.mutate()}>
          {running ? "正在核检…" : "立即核检"}
        </button>
      }
    >
      <div className="flex flex-col gap-3 text-sm">
        <p className="text-xs text-zinc-500">
          逐只股票、逐个交易日对照日线：从上市日到最近一个交易日，交易日里有、日线里没有的就是缺口。发现缺口后自动重新取一次：取回来就补上了；接口仍然没有，说明那天停牌，记下来以后不再报。每周同步结束后自动核检一次，重取结束后再复查一次。指数日线也一样查。
        </p>
        {!!start.error && <p className="text-red-600">{errorText(start.error)}</p>}
        <QueryView query={q}>{(s) => <Body s={s} />}</QueryView>
      </div>
    </Card>
  );
}

function Body({ s }: { s: DataCheckStatus }) {
  const last = s.last;
  if (!last) return <p className="text-zinc-500">还没有核检过。每周同步结束后会自动核检，也可以点「立即核检」。</p>;
  const results = last.result ?? [];
  const open = results.reduce((n, r) => n + r.open, 0);
  const fresh = results.reduce((n, r) => n + r.new_gaps, 0);
  return (
    <>
      <p className="text-xs text-zinc-500">
        上次核检：{fmtDateTime(last.started_at)}（{TRIGGERS[last.trigger] ?? last.trigger}）
        {last.end_date && `，查到 ${last.end_date}`}
        {!last.finished_at && (s.running ? "，进行中" : "，没有做完（程序中途退出了），下次核检会重新查")}
      </p>
      {last.error && <p className="text-red-600">核检出错：{last.error}</p>}
      {last.finished_at && !last.error && (
        <p
          className={
            open === 0
              ? "font-medium text-emerald-700 dark:text-emerald-400"
              : "font-medium text-amber-700 dark:text-amber-400"
          }
        >
          {open === 0
            ? "日线齐全：没有缺数据的股票和指数（停牌日除外）。"
            : `还有 ${fmtInt(open)} 个「股票 × 交易日」缺日线${fresh > 0 ? `（这次新发现 ${fmtInt(fresh)} 个）` : ""}，已安排重新取，取完会自动复查。`}
        </p>
      )}
      {results.length > 0 && <ResultTable rows={results} />}
      {s.open.length > 0 && (
        <div>
          <h3 className="mb-1 text-xs font-medium text-zinc-500">还缺数据的（缺的天数多的在前，最多 50 只）</h3>
          <ul className="flex flex-col gap-1">
            {s.open.map((g) => (
              <li key={`${g.dataset_id}-${g.thscode}`} className="flex flex-wrap items-baseline gap-x-2">
                <Link
                  to={`/${g.dataset_id === "index.bar_1d" ? "indices" : "securities"}/${g.thscode}`}
                  className="font-medium hover:underline"
                >
                  {g.name ?? g.thscode}
                </Link>
                <span className="num text-xs text-zinc-400">{g.thscode}</span>
                <span className="text-xs text-zinc-500">
                  缺 {fmtInt(g.days)} 天：{g.dates.join("、")}
                  {g.days > g.dates.length && " 等"}
                </span>
                {g.refetch_run && (
                  <Link to={`/sync/runs/${g.refetch_run}`} className="text-xs text-accent hover:underline">
                    重取中 · 同步 #{g.refetch_run}
                  </Link>
                )}
              </li>
            ))}
          </ul>
        </div>
      )}
    </>
  );
}

function ResultTable({ rows }: { rows: CheckResult[] }) {
  return (
    <Table>
      <thead>
        <tr>
          <th>数据</th>
          <th title="查了多少只、多少个「证券 × 交易日」">查了</th>
          <th className="text-right!" title="这次新发现的缺口">
            新发现
          </th>
          <th className="text-right!" title="以前的缺口，这次已经有数据了">
            已补回
          </th>
          <th className="text-right!" title="重新取过、接口仍然没有，确认是停牌（累计）">
            停牌
          </th>
          <th className="text-right!">还缺</th>
          <th>重取</th>
        </tr>
      </thead>
      <tbody>
        {rows.map((r) => (
          <tr key={r.dataset_id}>
            <td>{r.title}</td>
            <td className="text-xs text-zinc-500">
              {fmtInt(r.securities)} 只、{fmtInt(r.security_days)} 天
              {r.busy > 0 && (
                <span className="block text-zinc-400" title="这些证券正在同步，等同步完再查">
                  {fmtInt(r.busy)} 只正在同步，先不查
                </span>
              )}
            </td>
            <td className="num text-right">{r.new_gaps > 0 ? fmtInt(r.new_gaps) : "—"}</td>
            <td className="num text-right">{r.filled > 0 ? fmtInt(r.filled) : "—"}</td>
            <td className="num text-right">
              {fmtInt(r.suspended)}
              {r.confirmed_now > 0 && (
                <span className="text-xs text-zinc-400">（本次 +{fmtInt(r.confirmed_now)}）</span>
              )}
            </td>
            <td className="num text-right">
              {r.open > 0 ? <span className="text-amber-700 dark:text-amber-400">{fmtInt(r.open)}</span> : "—"}
            </td>
            <td className="text-xs">
              {r.refetch_run ? (
                <Link to={`/sync/runs/${r.refetch_run}`} className="text-accent hover:underline">
                  同步 #{r.refetch_run} · {fmtInt(r.refetch_tasks)} 个任务
                </Link>
              ) : (
                <span className="text-zinc-400">—</span>
              )}
            </td>
          </tr>
        ))}
      </tbody>
    </Table>
  );
}
