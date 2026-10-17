import { keepPreviousData } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import { Link, useParams } from "react-router";
import { useIndexBars, useIndexMembers, useListIndices } from "../api/generated/market/market";
import { PriceChart } from "../components/PriceChart";
import { RangePicker, rangeQuery } from "../components/RangePicker";
import { Badge, Card, Empty, ErrorState, inputClass, PageHeader, QueryView, Spinner, Table } from "../components/ui";
import { useSearchState } from "../hooks/useSearchState";
import { fmtInt } from "../lib/format";
import { INDEX_CATEGORIES } from "../lib/labels";

/** 供应商的指数和板块日线从这一天开始 */
const INDEX_EARLIEST = "2022-01-04";

export function IndexDetailPage() {
  const { thscode = "" } = useParams();
  const [s, set] = useSearchState({ range: "3", from: "", to: "" });
  const info = useListIndices({ q: thscode }, { query: { select: (rows) => rows.find((r) => r.thscode === thscode) } });
  const bars = useIndexBars(
    thscode,
    { ...rangeQuery(s), limit: 10000 },
    { query: { placeholderData: keepPreviousData } },
  );
  const members = useIndexMembers(thscode);
  const idx = info.data;

  return (
    <>
      <PageHeader
        title={
          <span className="flex flex-wrap items-baseline gap-2">
            {idx?.name ?? thscode}
            <span className="num font-mono text-sm font-normal text-zinc-500">{thscode}</span>
            {idx && <Badge>{INDEX_CATEGORIES[idx.category] ?? idx.category}</Badge>}
          </span>
        }
        extra={
          <Link to="/indices" className="text-sm text-zinc-500 hover:text-zinc-900 dark:hover:text-zinc-100">
            ← 返回列表
          </Link>
        }
      />
      <Card title="日 K 线">
        <div className="mb-3 flex flex-col gap-2">
          <RangePicker value={s} earliest={INDEX_EARLIEST} loading={bars.isPlaceholderData} onChange={set} />
          <p className="text-xs text-zinc-500">指数没有除权，不需要复权。供应商的指数日线从 2022-01-04 开始。</p>
        </div>
        {bars.error ? (
          <ErrorState error={bars.error} onRetry={() => bars.refetch()} />
        ) : !bars.data ? (
          <Spinner />
        ) : bars.data.bars.length === 0 ? (
          <Empty>本地还没有这个指数的日线</Empty>
        ) : (
          <PriceChart bars={bars.data.bars} height={400} />
        )}
      </Card>
      <Card title="当前成分股">
        <QueryView query={members}>{(rows) => <MemberTable rows={rows} />}</QueryView>
      </Card>
    </>
  );
}

function MemberTable({ rows }: { rows: { thscode: string; name?: string | null; valid_from: string }[] }) {
  const [q, setQ] = useState("");
  const shown = useMemo(() => rows.filter((r) => !q || r.thscode.includes(q) || r.name?.includes(q)), [rows, q]);
  if (rows.length === 0) return <Empty>还没有采集到成分股</Empty>;
  return (
    <>
      <div className="mb-3 flex items-center justify-between gap-3">
        <input
          className={`${inputClass} w-56`}
          placeholder="在成分股中查找"
          value={q}
          onChange={(e) => setQ(e.target.value)}
        />
        <span className="text-sm text-zinc-500">共 {fmtInt(rows.length)} 只</span>
      </div>
      <div className="max-h-[32rem] overflow-y-auto">
        <Table>
          <thead>
            <tr>
              <th>代码</th>
              <th>名称</th>
              <th title="历史成分从开始采集当天起才有，更早纳入的也显示为采集起始日">记录起始</th>
            </tr>
          </thead>
          <tbody>
            {shown.map((m) => (
              <tr key={m.thscode} className="hover:bg-zinc-50 dark:hover:bg-zinc-800/50">
                <td className="num font-mono">
                  <Link to={`/securities/${m.thscode}`} className="text-accent hover:underline">
                    {m.thscode}
                  </Link>
                </td>
                <td>{m.name ?? "—"}</td>
                <td className="num">{m.valid_from}</td>
              </tr>
            ))}
          </tbody>
        </Table>
      </div>
    </>
  );
}
