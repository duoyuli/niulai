import { clsx } from "clsx";
import { useState } from "react";
import { Link } from "react-router";
import type { RulesTrade } from "../api/generated/model";
import { yuan } from "../lib/accounts";
import { fmtPctValue, fmtPrice, upDownClass } from "../lib/format";
import { Segmented, Table } from "./ui";

const FILTERS = [
  { value: "all", label: "全部" },
  { value: "win", label: "赚钱的" },
  { value: "loss", label: "亏钱的" },
  { value: "open", label: "还持有" },
] as const;
type Filter = (typeof FILTERS)[number]["value"];

/** 默认先显示多少笔 */
const FIRST = 50;

/** 规则回测的交易记录：什么时候因为哪些策略买、什么时候为什么卖、每笔赚亏多少，看得出总收益是怎么来的 */
export function BacktestTrades({ trades }: { trades: RulesTrade[] }) {
  const [filter, setFilter] = useState<Filter>("all");
  const [all, setAll] = useState(false);
  if (trades.length === 0)
    return <p className="mt-2 text-sm text-zinc-500">这段时间没有买入：没有够门槛的策略选出符合规则的股票。</p>;
  const closed = trades.filter((t) => t.exit_date);
  const wins = closed.filter((t) => t.profit > 0).length;
  const open = trades.length - closed.length;
  const profit = trades.reduce((s, t) => s + t.profit, 0);
  const shown = trades.filter((t) =>
    filter === "win"
      ? t.exit_date && t.profit > 0
      : filter === "loss"
        ? t.exit_date && t.profit <= 0
        : filter === "open"
          ? !t.exit_date
          : true,
  );
  const rows = all ? shown : shown.slice(0, FIRST);
  return (
    <div className="mt-3 flex flex-col gap-2">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <p className="text-sm">
          共 {trades.length} 笔：已卖出 {closed.length} 笔（赚钱 {wins} 笔、亏钱 {closed.length - wins} 笔），还持有{" "}
          {open} 笔；合计盈亏 <span className={clsx("num font-medium", upDownClass(profit))}>{yuan(profit)}</span>
          <span className="text-xs text-zinc-500">（本金按 100 万元算）</span>
        </p>
        <Segmented
          value={filter}
          options={FILTERS}
          onChange={(f) => {
            setFilter(f);
            setAll(false);
          }}
        />
      </div>
      <Table>
        <thead>
          <tr>
            <th>买入</th>
            <th>股票（选中它的策略）</th>
            <th>买入价</th>
            <th>卖出</th>
            <th>卖出价</th>
            <th>为什么卖</th>
            <th>持有</th>
            <th>投入</th>
            <th>收益</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((t) => (
            <tr key={`${t.thscode}-${t.entry_date}`}>
              <td className="num text-xs" title={`${t.signal_date} 收盘出现买点，第二天开盘买入`}>
                {t.entry_date}
              </td>
              <td className="max-w-64">
                <Link to={`/securities/${t.thscode}`} className="hover:underline">
                  {t.name || t.thscode}
                </Link>
                <span className="block truncate text-xs text-zinc-500" title={t.strategies.join("、")}>
                  {t.strategies.join("、")}
                </span>
              </td>
              <td className="num">{fmtPrice(t.entry_price)}</td>
              <td className="num text-xs">{t.exit_date ?? <span className="text-zinc-400">还持有</span>}</td>
              <td className="num">{t.exit_price ? fmtPrice(t.exit_price) : ""}</td>
              <td className="text-xs text-zinc-600 dark:text-zinc-400">{t.exit_reason ?? ""}</td>
              <td className="num text-xs">{t.days_held} 天</td>
              <td className="num text-xs">{yuan(t.invest)}</td>
              <td className={clsx("num", upDownClass(t.profit))}>
                {fmtPctValue(t.return_pct, 1)}
                <span className="block text-xs">{yuan(t.profit)}</span>
              </td>
            </tr>
          ))}
        </tbody>
      </Table>
      {shown.length > FIRST && (
        <button type="button" className="self-start text-sm text-accent hover:underline" onClick={() => setAll(!all)}>
          {all ? "只看最近 50 笔" : `显示全部 ${shown.length} 笔`}
        </button>
      )}
      <p className="text-xs text-zinc-400">
        买入价、卖出价是未复权的开盘价；收益按前复权价算（含分红送转），扣了佣金和印花税；还持有的按最后一天收盘价算，不扣卖出费用。持有天数是交易日。
      </p>
    </div>
  );
}
