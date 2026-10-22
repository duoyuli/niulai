/** 回测的共用部分：参数输入、交易成本、结果展示。个股页的「回测」和策略页的「历史回测」都用它。规则见 docs/domains/backtest.md */
import { type ReactNode, useEffect, useState } from "react";
import { Link } from "react-router";
import type { EquityPoint, PortfolioResult, Summary, Trade } from "../api/generated/model";
import { useListStrategies } from "../api/generated/strategy/strategy";
import { downloadCsv } from "../lib/csv";
import { fmtBig, fmtInt, fmtPercent, fmtPrice, upDownClass } from "../lib/format";
import { EXIT_REASONS } from "../lib/strategy";
import { EquityChart } from "./EquityChart";
import { Card, Empty, ErrorState, Field, inputClass, Segmented, Spinner, Stat, Table } from "./ui";

/** 交易成本的默认值，与后端一致：佣金万 2.5、最低 5 元，卖出印花税千分之 0.5 */
export const COST_DEFAULTS = { capital: "100000", commission: "2.5", minFee: "5", stamp: "0.5" };
type CostState = typeof COST_DEFAULTS;

/** 页面上的成本输入换成接口参数 */
export function costParams(s: CostState) {
  return {
    capital: Number(s.capital) || undefined,
    commission_rate: Number(s.commission) / 10000,
    min_commission: Number(s.minFee),
    stamp_tax_rate: Number(s.stamp) / 1000,
  };
}

/** 初始资金和折叠起来的交易成本 */
export function CostFields({ s, set }: { s: CostState; set: (patch: Partial<CostState>) => void }) {
  return (
    <>
      <NumberField label="初始资金（元）" value={s.capital} onCommit={(capital) => set({ capital })} width="w-28" />
      <details className="w-full text-sm">
        <summary className="cursor-pointer text-zinc-500 select-none">交易成本</summary>
        <div className="mt-3 flex flex-wrap items-end gap-4">
          <NumberField
            label="佣金（万分之）"
            hint="券商佣金，买卖都收。万 2.5 表示每成交 1 万元收 2.5 元"
            value={s.commission}
            onCommit={(commission) => set({ commission })}
          />
          <NumberField
            label="最低佣金（元）"
            hint="单笔佣金不足这个数时按这个数收"
            value={s.minFee}
            onCommit={(minFee) => set({ minFee })}
          />
          <NumberField
            label="印花税（千分之）"
            hint="国家收的税，只在卖出时收；2023 年 8 月 28 日起为千分之 0.5"
            value={s.stamp}
            onCommit={(stamp) => set({ stamp })}
          />
        </div>
      </details>
    </>
  );
}

/** 回测规则说明，折叠起来 */
export function RulesNote({ items }: { items: string[] }) {
  return (
    <details className="text-sm text-zinc-500">
      <summary className="cursor-pointer select-none">回测怎么算的</summary>
      <ul className="mt-2 list-disc space-y-1 pl-5 text-zinc-600 dark:text-zinc-400">
        {items.map((x) => (
          <li key={x}>{x}</li>
        ))}
        <li>历史表现不代表未来，回测结果仅供研究。</li>
      </ul>
    </details>
  );
}

export function Loaded<T>({
  query,
  pending = "回测中",
  children,
}: {
  query: { data: T | undefined; error: unknown; refetch: () => unknown };
  pending?: string;
  children: (d: T) => ReactNode;
}) {
  if (query.error)
    return (
      <Card>
        <ErrorState error={query.error} onRetry={() => query.refetch()} />
      </Card>
    );
  if (query.data === undefined)
    return (
      <Card>
        <Spinner label={pending} />
      </Card>
    );
  return <>{children(query.data)}</>;
}

export function StrategySelect({ value, onChange }: { value: string; onChange: (v: string) => void }) {
  const list = useListStrategies();
  const items = list.data ?? [];
  return (
    <Field label="策略">
      <select className={`${inputClass} w-48`} value={value} onChange={(e) => onChange(e.target.value)}>
        <option value="">{list.isPending ? "加载中…" : items.length ? "选择一个策略" : "还没有策略"}</option>
        {items.map((x) => (
          <option key={x.strategy_id} value={x.strategy_id}>
            {x.name}
          </option>
        ))}
      </select>
    </Field>
  );
}

/** 数字输入：失焦或回车时提交 */
export function NumberField({
  label,
  hint,
  value,
  onCommit,
  width = "w-20",
}: {
  label: string;
  hint?: string;
  value: string;
  onCommit: (v: string) => void;
  width?: string;
}) {
  const [v, setV] = useState(value);
  useEffect(() => setV(value), [value]);
  const commit = () => {
    if (v !== value && Number.isFinite(Number(v)) && Number(v) >= 0) onCommit(v);
    else setV(value);
  };
  return (
    <Field label={label} hint={hint}>
      <input
        className={`${inputClass} num ${width}`}
        inputMode="decimal"
        value={v}
        onChange={(e) => setV(e.target.value)}
        onBlur={commit}
        onKeyDown={(e) => e.key === "Enter" && commit()}
      />
    </Field>
  );
}

/** 单只股票的回测结果：一句结论、四个关键数字、账户曲线，其余数字和交易记录折叠起来 */
export function Result({
  label,
  summary: m,
  equity,
  trades,
  log,
  onLog,
}: {
  label: ReactNode;
  summary: Summary;
  equity: EquityPoint[];
  trades: Trade[];
  log: boolean;
  onLog: (v: boolean) => void;
}) {
  const beat = m.total_return - m.benchmark_return;
  return (
    <Card
      title={
        <span className="text-xs font-normal text-zinc-500">
          {label} · {m.start} 至 {m.end}（{fmtInt(m.days)} 个交易日）
        </span>
      }
    >
      <p className="mb-4 text-base">
        {fmtBig(m.capital)} 元按这个方法交易，变成{" "}
        <b className={upDownClass(m.total_return)}>{fmtBig(m.final_equity)} 元</b>（{fmtPercent(m.total_return)}）；
        一直拿着不动是 <b className={upDownClass(m.benchmark_return)}>{fmtPercent(m.benchmark_return)}</b>。
        {beat >= 0 ? (
          <span className="text-up">比拿着不动多赚 {fmtPercent(beat)}。</span>
        ) : (
          <span className="text-down">比拿着不动少赚 {fmtPercent(-beat)}。</span>
        )}
      </p>
      <div className="grid grid-cols-2 gap-4 sm:grid-cols-4">
        <Stat
          label="收益"
          value={<span className={upDownClass(m.total_return)}>{fmtPercent(m.total_return)}</span>}
          sub={
            m.annual_return === null || m.annual_return === undefined
              ? "区间太短，不算年化"
              : `年化 ${fmtPercent(m.annual_return)}`
          }
        />
        <Stat
          label="最多亏过"
          value={<span className="text-down">{fmtPercent(-m.max_drawdown)}</span>}
          sub={`从最高点往下跌的最大幅度；拿着不动是 ${fmtPercent(-m.benchmark_max_drawdown)}`}
        />
        <WinStat closed={m.closed_trades} winRate={m.win_rate} payoff={m.payoff_ratio} />
        <Stat
          label="平均拿多久"
          value={
            m.avg_bars_held === null || m.avg_bars_held === undefined ? "—" : `${m.avg_bars_held.toFixed(0)} 个交易日`
          }
          sub={`${fmtPercent(m.exposure, 0).replace("+", "")} 的时间持有股票`}
        />
      </div>
      <RiskLine risk={m.risk} />
      <div className="mt-5">
        <EquityChart points={equity} capital={m.capital} log={log} />
        <LogToggle log={log} onLog={onLog} note="蓝线是这个方法，灰线是一直拿着不动。" />
      </div>
      <details className="mt-4 text-sm">
        <summary className="cursor-pointer text-zinc-500 select-none">交易记录（{trades.length} 笔）和费用</summary>
        <p className="my-3 text-xs text-zinc-500">
          手续费 {fmtBig(m.fees)} 元，收到分红 {fmtBig(m.dividends)} 元（税前）；一字板顺延 {m.deferred_orders}{" "}
          次，钱不够买一手放弃 {m.skipped_buys} 次。
        </p>
        <TradeTable trades={trades} />
      </details>
    </Card>
  );
}

/** 交易次数、胜率和盈亏比 */
export function WinStat({
  closed,
  winRate,
  payoff,
}: {
  closed: number;
  winRate?: number | null;
  payoff?: number | null;
}) {
  return (
    <Stat
      label="交易次数"
      value={`${fmtInt(closed)} 次`}
      sub={
        winRate === null || winRate === undefined ? (
          "还没有卖出过"
        ) : (
          <span title="盈亏比：赚钱的交易平均赚多少 ÷ 亏钱的交易平均亏多少；胜率不高但盈亏比大，也可能整体赚钱">
            赚钱的占 {fmtPercent(winRate, 0).replace("+", "")}
            {payoff !== null && payoff !== undefined && `，盈亏比 ${payoff.toFixed(2)}`}
          </span>
        )
      }
    />
  );
}

/** 曲线下方：坐标切换和说明 */
function LogToggle({ log, onLog, note }: { log: boolean; onLog: (v: boolean) => void; note: string }) {
  return (
    <div className="mt-2 flex flex-wrap items-center justify-between gap-2 text-xs text-zinc-500">
      <span>{note}</span>
      <Segmented
        value={log ? "log" : "normal"}
        options={[
          { value: "normal", label: "普通坐标" },
          { value: "log", label: "对数坐标", hint: "同样的涨跌幅在图上高度相同，适合看很长的区间" },
        ]}
        onChange={(v) => onLog(v === "log")}
      />
    </div>
  );
}

/** 全市场组合回测的结果 */
export function PortfolioView({
  data,
  log,
  onLog,
}: {
  data: PortfolioResult;
  log: boolean;
  onLog: (v: boolean) => void;
}) {
  const m = data.summary;
  const bench = m.benchmark_name;
  const beat =
    m.benchmark_return === null || m.benchmark_return === undefined ? null : m.total_return - m.benchmark_return;
  return (
    <Card
      title={
        <span className="text-xs font-normal text-zinc-500">
          全市场 · 最多同时持有 {data.max_positions} 只 · {m.start} 至 {m.end}（{fmtInt(m.days)} 个交易日）
        </span>
      }
    >
      <p className="mb-4 text-base">
        如果过去每天按这个策略选股买入，{fmtBig(m.capital)} 元会变成{" "}
        <b className={upDownClass(m.total_return)}>{fmtBig(m.final_equity)} 元</b>（{fmtPercent(m.total_return)}）
        {bench && beat !== null ? (
          <>
            ；同期{bench}是 <b className={upDownClass(m.benchmark_return)}>{fmtPercent(m.benchmark_return)}</b>。
            {beat >= 0 ? (
              <span className="text-up">跑赢 {fmtPercent(beat)}。</span>
            ) : (
              <span className="text-down">跑输 {fmtPercent(-beat)}。</span>
            )}
          </>
        ) : (
          "。"
        )}
      </p>
      <div className="grid grid-cols-2 gap-4 sm:grid-cols-4">
        <Stat
          label="收益"
          value={<span className={upDownClass(m.total_return)}>{fmtPercent(m.total_return)}</span>}
          sub={
            m.annual_return === null || m.annual_return === undefined
              ? "区间太短，不算年化"
              : `年化 ${fmtPercent(m.annual_return)}`
          }
        />
        <Stat
          label="最多亏过"
          value={<span className="text-down">{fmtPercent(-m.max_drawdown)}</span>}
          sub="从最高点往下跌的最大幅度"
        />
        <WinStat closed={m.closed_trades} winRate={m.win_rate} payoff={m.payoff_ratio} />
        <Stat
          label="平均拿多久"
          value={
            m.avg_bars_held === null || m.avg_bars_held === undefined ? "—" : `${m.avg_bars_held.toFixed(0)} 个交易日`
          }
          sub={`平均每天持有 ${m.avg_positions.toFixed(1)} 只`}
        />
      </div>
      <RiskLine risk={m.risk} />
      <div className="mt-5">
        <EquityChart points={data.equity} capital={m.capital} log={log} benchmarkLabel={bench ?? "基准"} />
        <LogToggle
          log={log}
          onLog={onLog}
          note={
            bench
              ? `蓝线是策略，灰线是同样的钱每天平均分散买入全部正常交易的股票（含分红，相当于天天随便买）。`
              : "这段时间没有足够的日线，不显示基准。"
          }
        />
      </div>
      <details className="mt-4 text-sm">
        <summary className="cursor-pointer text-zinc-500 select-none">
          交易记录（{data.trades.length} 笔{data.trades.length >= 500 ? "，只显示最近的" : ""}）和费用
        </summary>
        <div className="my-3 flex flex-wrap items-center justify-between gap-2">
          <p className="text-xs text-zinc-500">
            手续费 {fmtBig(m.fees)} 元；区间内出现买入信号 {fmtInt(m.signals)} 次，其中仓位已满没买{" "}
            {fmtInt(m.skipped_full)} 次。
          </p>
          <button
            type="button"
            className="text-sm text-accent hover:underline"
            onClick={() =>
              downloadCsv(
                `组合回测交易记录-${new Date().toISOString().slice(0, 10)}`,
                ["股票", "代码", "信号日", "买入日", "买入价", "卖出日", "卖出价", "为什么卖", "持有交易日", "收益率%"],
                data.trades.map((t) => [
                  t.name,
                  t.thscode,
                  t.signal_date,
                  t.entry_date,
                  t.entry_price,
                  t.exit_date,
                  t.exit_price,
                  t.exit_reason ? EXIT_REASONS[t.exit_reason] : "",
                  t.bars_held,
                  t.return_pct,
                ]),
              )
            }
          >
            下载 CSV
          </button>
        </div>
        {data.trades.length === 0 ? (
          <Empty>区间内没有发生过买入</Empty>
        ) : (
          <div className="max-h-[32rem] overflow-y-auto">
            <Table>
              <thead>
                <tr>
                  <th>股票</th>
                  <th>买入日期</th>
                  <th className="text-right!">买入价</th>
                  <th>卖出日期</th>
                  <th className="text-right!">卖出价</th>
                  <th>为什么卖</th>
                  <th className="text-right!">拿了（交易日）</th>
                  <th className="text-right!" title="扣除手续费，含分红送转">
                    收益率
                  </th>
                </tr>
              </thead>
              <tbody>
                {data.trades.map((t) => (
                  <tr key={`${t.thscode}-${t.entry_date}`}>
                    <td>
                      <Link to={`/securities/${t.thscode}`} className="hover:underline">
                        {t.name} <span className="num font-mono text-xs text-zinc-500">{t.thscode}</span>
                      </Link>
                    </td>
                    <td className="num">{t.entry_date}</td>
                    <td className="num text-right">{fmtPrice(t.entry_price)}</td>
                    <td className="num">{t.exit_date ?? <span className="text-amber-600">仍持有</span>}</td>
                    <td className="num text-right">
                      {t.exit_price === null || t.exit_price === undefined ? "—" : fmtPrice(t.exit_price)}
                    </td>
                    <td>{t.exit_reason ? EXIT_REASONS[t.exit_reason] : "—"}</td>
                    <td className="num text-right">{t.bars_held}</td>
                    <td className={`num text-right ${upDownClass(t.return_pct)}`}>{fmtPercent(t.return_pct)}</td>
                  </tr>
                ))}
              </tbody>
            </Table>
          </div>
        )}
      </details>
    </Card>
  );
}

function RiskLine({ risk }: { risk: Summary["risk"] }) {
  const value = (v: number | null | undefined, digits = 2) =>
    v === null || v === undefined || !Number.isFinite(v) ? "—" : v.toFixed(digits);
  return (
    <p className="mt-3 text-xs text-zinc-500">
      风险调整：Sharpe {value(risk.sharpe)}；Sortino {value(risk.sortino)}；Calmar {value(risk.calmar)}；年化波动{" "}
      {risk.annual_volatility === null || risk.annual_volatility === undefined
        ? "—"
        : fmtPercent(risk.annual_volatility)}
    </p>
  );
}

export function TradeTable({ trades }: { trades: Trade[] }) {
  if (trades.length === 0) return <Empty>区间内没有发生过买入</Empty>;
  return (
    <div>
      <div className="mb-2 flex justify-end">
        <button
          type="button"
          className="text-sm text-accent hover:underline"
          onClick={() =>
            downloadCsv(
              `单只股票回测交易记录-${new Date().toISOString().slice(0, 10)}`,
              [
                "买入日",
                "买入价",
                "卖出日",
                "卖出价",
                "为什么卖",
                "买入股数",
                "卖出股数",
                "持有交易日",
                "分红（元）",
                "盈亏（元）",
                "收益率%",
              ],
              trades.map((t) => [
                t.entry_date,
                t.entry_price,
                t.exit_date,
                t.exit_price,
                t.exit_reason ? EXIT_REASONS[t.exit_reason] : "",
                t.shares,
                t.exit_shares,
                t.bars_held,
                t.dividends,
                t.pnl,
                t.return_pct,
              ]),
            )
          }
        >
          下载 CSV
        </button>
      </div>
      <div className="max-h-[32rem] overflow-y-auto">
        <Table>
          <thead>
            <tr>
              <th>买入日期</th>
              <th className="text-right!">买入价</th>
              <th>卖出日期</th>
              <th className="text-right!">卖出价</th>
              <th>为什么卖</th>
              <th className="text-right!">股数</th>
              <th className="text-right!">持有（交易日）</th>
              <th className="text-right!" title="持有期间收到的现金分红，税前">
                分红（元）
              </th>
              <th className="text-right!" title="卖出所得加分红减成本，已扣手续费">
                盈亏（元）
              </th>
              <th className="text-right!">收益率</th>
            </tr>
          </thead>
          <tbody>
            {[...trades].reverse().map((t) => (
              <tr key={t.entry_date}>
                <td className="num">{t.entry_date}</td>
                <td className="num text-right">{fmtPrice(t.entry_price)}</td>
                <td className="num">{t.exit_date ?? <span className="text-amber-600">仍持有</span>}</td>
                <td className="num text-right">
                  {t.exit_price === null || t.exit_price === undefined ? "—" : fmtPrice(t.exit_price)}
                </td>
                <td>{t.exit_reason ? EXIT_REASONS[t.exit_reason] : "—"}</td>
                <td className="num text-right" title={t.exit_shares !== t.shares ? "持有期间有送转股" : undefined}>
                  {fmtInt(t.shares)}
                  {t.exit_shares !== t.shares && <span className="text-zinc-500"> → {fmtInt(t.exit_shares)}</span>}
                </td>
                <td className="num text-right">{t.bars_held}</td>
                <td className="num text-right">{t.dividends > 0 ? fmtBig(t.dividends) : "—"}</td>
                <td className={`num text-right ${upDownClass(t.pnl)}`}>{fmtBig(t.pnl)}</td>
                <td className={`num text-right ${upDownClass(t.return_pct)}`}>{fmtPercent(t.return_pct)}</td>
              </tr>
            ))}
          </tbody>
        </Table>
      </div>
    </div>
  );
}
