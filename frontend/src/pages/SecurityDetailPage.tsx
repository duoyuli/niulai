import { keepPreviousData } from "@tanstack/react-query";
import { useMemo } from "react";
import { Link, useParams } from "react-router";
import { useGetSecurity, useSecurityBars, useSecurityMa } from "../api/generated/market/market";
import type { Bar, MaPoint } from "../api/generated/model";
import { MaControls } from "../components/MaControls";
import { PriceChart } from "../components/PriceChart";
import {
  Badge,
  buttonClass,
  Card,
  Empty,
  ErrorState,
  PageHeader,
  Segmented,
  SignalBadge,
  Spinner,
  Stat,
  Table,
} from "../components/ui";
import { WatchButton } from "../components/WatchButton";
import { useSearchState } from "../hooks/useSearchState";
import { fmtBig, fmtInt, fmtPercent, fmtPrice, RANGES, rangeStart, toNumber, upDownClass } from "../lib/format";
import { ADJUSTS, type Adjust, BOARDS } from "../lib/labels";

export function SecurityDetailPage() {
  const { thscode = "" } = useParams();
  const [s, set] = useSearchState({ adjust: "qfq", range: "3", fast: "15", slow: "30" });
  const adjust = s.adjust as Adjust;
  const fast = Number(s.fast) || 15;
  const slow = Number(s.slow) || 30;
  const start = rangeStart(s.range);

  const info = useGetSecurity(thscode);
  const bars = useSecurityBars(
    thscode,
    { adjust, start, limit: 10000 },
    { query: { placeholderData: keepPreviousData } },
  );
  // 均线和交叉信号固定按等比前复权计算，与 K 线口径无关
  const ma = useSecurityMa(thscode, { fast, slow, start }, { query: { placeholderData: keepPreviousData } });

  if (info.error) return <ErrorState error={info.error} onRetry={() => info.refetch()} />;
  const sec = info.data;
  const maOverlay = adjust === "qfq" && ma.data ? { points: ma.data.points, fast, slow } : undefined;

  return (
    <>
      <PageHeader
        title={
          <span className="flex flex-wrap items-baseline gap-2">
            {sec?.name ?? thscode}
            <span className="num font-mono text-sm font-normal text-zinc-500">{thscode}</span>
            {sec && <Badge>{BOARDS[sec.board] ?? sec.board}</Badge>}
            {sec?.delisted_on && <Badge tone="bad">已退市 {sec.delisted_on}</Badge>}
          </span>
        }
        desc={
          sec && (
            <>
              上市 {sec.list_date ?? "—"} · 本地日线 {sec.first_bar ?? "—"} 至 {sec.last_bar ?? "—"}，共{" "}
              {fmtInt(sec.bars)} 根
            </>
          )
        }
        extra={
          <div className="flex items-center gap-3">
            <WatchButton thscode={thscode} withLabel />
            <Link
              to={`/backtest?code=${thscode}&fast=${fast}&slow=${slow}`}
              className={buttonClass("ghost")}
              title="按当前均线周期回测：金叉买入、死叉卖出"
            >
              回测
            </Link>
            <Link to="/" className="text-sm text-zinc-500 hover:text-zinc-900 dark:hover:text-zinc-100">
              ← 返回列表
            </Link>
          </div>
        }
      />

      {bars.data && <Summary bars={bars.data.bars} ma={ma.data?.points} />}

      <Card
        title="日 K 线"
        extra={
          <div className="flex flex-wrap items-end gap-3">
            <Segmented value={adjust} options={ADJUSTS} onChange={(adjust) => set({ adjust })} />
            <Segmented value={s.range} options={RANGES} onChange={(range) => set({ range })} />
            <MaControls fast={fast} slow={slow} onChange={(f, sl) => set({ fast: String(f), slow: String(sl) })} />
          </div>
        }
      >
        <p className="mb-3 text-xs text-zinc-500">
          {ADJUSTS.find((a) => a.value === adjust)?.hint}。
          {adjust === "qfq" ? (
            <>
              橙线是 MA{fast}，紫线是 MA{slow}；▲ 金叉表示快线上穿慢线，▼ 死叉表示快线下穿慢线。
            </>
          ) : (
            <>均线按前复权价计算，切换到「等比前复权」后显示在图上。</>
          )}
        </p>
        {bars.error ? (
          <ErrorState error={bars.error} onRetry={() => bars.refetch()} />
        ) : !bars.data ? (
          <Spinner />
        ) : bars.data.bars.length === 0 ? (
          <Empty>本地还没有这只股票的日线，可能仍在回补中</Empty>
        ) : (
          <PriceChart bars={bars.data.bars} ma={maOverlay} />
        )}
      </Card>

      <div className="grid gap-4 lg:grid-cols-3">
        <Card title={`交叉信号（MA${fast} / MA${slow}）`} className="lg:col-span-2">
          {ma.isPending ? (
            <Spinner />
          ) : ma.error ? (
            <ErrorState error={ma.error} />
          ) : (
            <SignalTable points={ma.data.points} />
          )}
        </Card>
        <Card title="简称变化">
          {sec?.name_history.length ? (
            <ul className="flex flex-col gap-2 text-sm">
              {sec.name_history.map((n) => (
                <li key={n.valid_from} className="flex items-center justify-between gap-2">
                  <span>
                    {n.name} {n.is_st && <Badge tone="warn">ST</Badge>}
                  </span>
                  <span className="num text-xs text-zinc-500">
                    {n.valid_from} 起{n.valid_to ? ` 至 ${n.valid_to}` : ""}
                  </span>
                </li>
              ))}
            </ul>
          ) : (
            <Empty>暂无记录</Empty>
          )}
          <p className="mt-3 text-xs text-zinc-400">简称历史从开始采集当天起积累，供应商没有更早的记录。</p>
        </Card>
      </div>
    </>
  );
}

function Summary({ bars, ma }: { bars: Bar[]; ma?: MaPoint[] }) {
  const stats = useMemo(() => {
    const last = bars.at(-1);
    const prev = bars.at(-2);
    const first = bars[0];
    if (!last || !first) return null;
    const close = toNumber(last.close) ?? 0;
    const prevClose = toNumber(prev?.close);
    let high = Number.NEGATIVE_INFINITY;
    let low = Number.POSITIVE_INFINITY;
    for (const b of bars) {
      high = Math.max(high, toNumber(b.high) ?? high);
      low = Math.min(low, toNumber(b.low) ?? low);
    }
    const lastMa = ma?.at(-1);
    return {
      last,
      close,
      change: prevClose ? close / prevClose - 1 : null,
      rangeReturn: close / (toNumber(first.close) ?? close) - 1,
      high,
      low,
      trend: lastMa
        ? lastMa.ma_fast > lastMa.ma_slow
          ? "up"
          : lastMa.ma_fast < lastMa.ma_slow
            ? "down"
            : "flat"
        : null,
    };
  }, [bars, ma]);
  if (!stats) return null;
  return (
    <Card>
      <div className="grid grid-cols-2 gap-4 sm:grid-cols-3 lg:grid-cols-6">
        <Stat
          label={`最新收盘（${stats.last.trade_date}）`}
          value={<span className={upDownClass(stats.change)}>{fmtPrice(stats.close)}</span>}
          sub={<span className={upDownClass(stats.change)}>{fmtPercent(stats.change)}</span>}
        />
        <Stat
          label="区间涨跌"
          value={<span className={upDownClass(stats.rangeReturn)}>{fmtPercent(stats.rangeReturn)}</span>}
          sub="按所选复权口径"
        />
        <Stat label="区间最高" value={fmtPrice(stats.high)} />
        <Stat label="区间最低" value={fmtPrice(stats.low)} />
        <Stat label="最新成交额" value={fmtBig(stats.last.amount)} sub={`成交量 ${fmtBig(stats.last.volume)} 股`} />
        <Stat
          label="均线状态"
          value={
            stats.trend === "up" ? (
              <span className="text-up">快线在上</span>
            ) : stats.trend === "down" ? (
              <span className="text-down">快线在下</span>
            ) : (
              "—"
            )
          }
          sub={stats.trend === "up" ? "短期走势强于长期" : stats.trend === "down" ? "短期走势弱于长期" : undefined}
        />
      </div>
    </Card>
  );
}

function SignalTable({ points }: { points: MaPoint[] }) {
  const rows = useMemo(() => points.filter((p) => p.signal).reverse(), [points]);
  if (rows.length === 0) return <Empty>所选区间内没有发生交叉</Empty>;
  return (
    <div className="max-h-96 overflow-y-auto">
      <Table>
        <thead>
          <tr>
            <th>日期</th>
            <th>信号</th>
            <th className="text-right!">收盘（前复权）</th>
            <th className="text-right!">快线</th>
            <th className="text-right!">慢线</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((p) => (
            <tr key={p.trade_date}>
              <td className="num">{p.trade_date}</td>
              <td>{p.signal && <SignalBadge signal={p.signal} />}</td>
              <td className="num text-right">{fmtPrice(p.close)}</td>
              <td className="num text-right">{fmtPrice(p.ma_fast)}</td>
              <td className="num text-right">{fmtPrice(p.ma_slow)}</td>
            </tr>
          ))}
        </tbody>
      </Table>
    </div>
  );
}
