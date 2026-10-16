import { keepPreviousData } from "@tanstack/react-query";
import { useMemo } from "react";
import { Link } from "react-router";
import { useMaCrossScan } from "../api/generated/market/market";
import { MaControls } from "../components/MaControls";
import { Card, Empty, Field, inputClass, PageHeader, QueryView, Segmented, SignalBadge, Table } from "../components/ui";
import { WatchButton } from "../components/WatchButton";
import { useSearchState } from "../hooks/useSearchState";
import { fmtInt, fmtPercent, fmtPrice, todayCn } from "../lib/format";

const SIGNALS = [
  { value: "", label: "全部" },
  { value: "golden", label: "金叉" },
  { value: "death", label: "死叉" },
] as const;

export function SignalsPage() {
  const [s, set] = useSearchState({ date: "", fast: "15", slow: "30", signal: "" });
  const fast = Number(s.fast) || 15;
  const slow = Number(s.slow) || 30;
  const query = useMaCrossScan(
    { date: s.date || undefined, fast, slow, signal: s.signal || undefined },
    { query: { placeholderData: keepPreviousData } },
  );

  return (
    <>
      <PageHeader
        title="均线扫描"
        desc="找出某个交易日发生均线交叉的股票。金叉：短期均线从下往上穿过长期均线，常被看作走强的信号；死叉相反。"
      />
      <Card>
        <div className="mb-4 flex flex-wrap items-end gap-4">
          <Field label="交易日" hint="留空表示本地数据中的最新交易日">
            <input
              type="date"
              className={inputClass}
              max={todayCn()}
              value={s.date}
              onChange={(e) => set({ date: e.target.value })}
            />
          </Field>
          <MaControls fast={fast} slow={slow} onChange={(f, sl) => set({ fast: String(f), slow: String(sl) })} />
          <Segmented
            value={s.signal as (typeof SIGNALS)[number]["value"]}
            options={SIGNALS}
            onChange={(signal) => set({ signal })}
          />
        </div>
        <QueryView query={query}>
          {(data) => (
            <>
              <p className="mb-3 text-sm text-zinc-500">
                {data.trade_date} · 扫描 {fmtInt(data.scanned)} 只股票，{data.hits.length} 只发生交叉（MA{data.fast} /
                MA{data.slow}，等比前复权）
              </p>
              <HitTable hits={data.hits} />
            </>
          )}
        </QueryView>
      </Card>
    </>
  );
}

function HitTable({
  hits,
}: {
  hits: {
    thscode: string;
    name?: string | null;
    signal: "golden" | "death";
    close: number;
    ma_fast: number;
    ma_slow: number;
  }[];
}) {
  const rows = useMemo(
    () => [...hits].sort((a, b) => a.signal.localeCompare(b.signal) || a.thscode.localeCompare(b.thscode)),
    [hits],
  );
  if (rows.length === 0) return <Empty>这一天没有股票发生交叉</Empty>;
  return (
    <Table>
      <thead>
        <tr>
          <th className="w-0" aria-label="收藏" />
          <th>代码</th>
          <th>名称</th>
          <th>信号</th>
          <th className="text-right!">收盘（前复权）</th>
          <th className="text-right!">快线</th>
          <th className="text-right!">慢线</th>
          <th className="text-right!" title="快线相对慢线的偏离">
            快慢线差
          </th>
        </tr>
      </thead>
      <tbody>
        {rows.map((h) => (
          <tr key={h.thscode} className="hover:bg-zinc-50 dark:hover:bg-zinc-800/50">
            <td className="pr-0!">
              <WatchButton thscode={h.thscode} />
            </td>
            <td className="num font-mono">
              <Link to={`/securities/${h.thscode}`} className="text-accent hover:underline">
                {h.thscode}
              </Link>
            </td>
            <td>{h.name ?? "—"}</td>
            <td>
              <SignalBadge signal={h.signal} />
            </td>
            <td className="num text-right">{fmtPrice(h.close)}</td>
            <td className="num text-right">{fmtPrice(h.ma_fast)}</td>
            <td className="num text-right">{fmtPrice(h.ma_slow)}</td>
            <td className="num text-right">{fmtPercent(h.ma_fast / h.ma_slow - 1)}</td>
          </tr>
        ))}
      </tbody>
    </Table>
  );
}
