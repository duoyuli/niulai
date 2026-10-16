import { keepPreviousData } from "@tanstack/react-query";
import { type ReactNode, useEffect, useState } from "react";
import { Link } from "react-router";
import { useMaCrossBacktest, usePortfolioBacktest, useStrategyBacktest } from "../api/generated/backtest/backtest";
import type { EquityPoint, PortfolioResult, Summary, Trade } from "../api/generated/model";
import { useListStrategies } from "../api/generated/strategy/strategy";
import { useListWatchlist } from "../api/generated/watchlist/watchlist";
import { EquityChart } from "../components/EquityChart";
import { MaControls } from "../components/MaControls";
import { SecuritySearch } from "../components/SecuritySearch";
import {
  buttonClass,
  Card,
  Empty,
  ErrorState,
  Field,
  inputClass,
  PageHeader,
  Segmented,
  Spinner,
  Stat,
  Table,
} from "../components/ui";
import { useSearchState } from "../hooks/useSearchState";
import { fmtBig, fmtInt, fmtPercent, fmtPrice, RANGES, rangeStart, upDownClass } from "../lib/format";
import { EXIT_REASONS } from "../lib/strategy";

/** 交易成本的默认值，与后端一致：佣金万 2.5、最低 5 元，卖出印花税千分之 0.5 */
const DEFAULTS = { capital: "100000", commission: "2.5", minFee: "5", stamp: "0.5" };

const MODES = [
  { value: "ma", label: "均线交叉", hint: "一只股票：快线上穿慢线买入，下穿卖出" },
  { value: "strategy", label: "按策略", hint: "一只股票：符合已保存策略的条件时买入，按策略的卖出规则卖出" },
  { value: "portfolio", label: "全市场选股", hint: "每天用策略在全市场选股买入，最多同时持有几只" },
] as const;

/** 全市场回测最长 5 年：几年的全市场日线逐只计算，区间再长耗时和内存都会明显增加 */
const PORTFOLIO_RANGES = RANGES.filter((r) => r.value === "1" || r.value === "3" || r.value === "5");

const DESC: Record<string, string> = {
  ma: "用历史数据检验均线策略：快线上穿慢线（金叉）后买入，下穿（死叉）后卖出，看看这样做过去能赚多少、最多亏过多少，并和一直持有不动比较。",
  strategy:
    "在一只股票上检验已保存的策略：符合选股条件的第二天买入，触发卖出规则（卖出信号、止损、止盈、持有到期）的第二天卖出。",
  portfolio:
    "假设过去每天收盘后都用这个策略在全市场选股，第二天开盘买入，最多同时持有几只、空余仓位平分现金，按卖出规则卖出。看看整体能赚多少，并和指数比较。",
};

export function BacktestPage() {
  const [s, set] = useSearchState({
    mode: "ma",
    code: "",
    strategy: "",
    fast: "15",
    slow: "30",
    range: "10",
    positions: "5",
    ...DEFAULTS,
    log: "",
  });
  const fast = Number(s.fast) || 15;
  const slow = Number(s.slow) || 30;
  const strategyId = Number(s.strategy) || 0;
  const costs = {
    capital: Number(s.capital) || undefined,
    commission_rate: Number(s.commission) / 10000,
    min_commission: Number(s.minFee),
    stamp_tax_rate: Number(s.stamp) / 1000,
  };
  const mode = s.mode as (typeof MODES)[number]["value"];
  // 全市场回测的区间不能超过 5 年
  const range = mode === "portfolio" && !PORTFOLIO_RANGES.some((r) => r.value === s.range) ? "3" : s.range;
  const opts = { placeholderData: keepPreviousData, retry: false } as const;
  const ma = useMaCrossBacktest(
    { thscode: s.code, fast, slow, start: rangeStart(range), ...costs },
    { query: { enabled: mode === "ma" && s.code !== "", ...opts } },
  );
  const single = useStrategyBacktest(
    { strategy_id: strategyId, thscode: s.code, start: rangeStart(range), ...costs },
    { query: { enabled: mode === "strategy" && s.code !== "" && strategyId > 0, ...opts } },
  );
  const portfolio = usePortfolioBacktest(
    { strategy_id: strategyId, start: rangeStart(range), max_positions: Number(s.positions) || 5, ...costs },
    { query: { enabled: mode === "portfolio" && strategyId > 0, staleTime: 5 * 60_000, ...opts } },
  );
  const log = s.log === "1";
  const onLog = (v: boolean) => set({ log: v ? "1" : "" });

  return (
    <>
      <PageHeader title="策略回测" desc={DESC[mode]} />
      <Card>
        <div className="mb-4">
          <Segmented value={mode} options={MODES} onChange={(mode) => set({ mode })} />
        </div>
        <div className="flex flex-wrap items-end gap-4">
          {mode !== "portfolio" && <SecuritySearch label="股票" onPick={(code) => set({ code })} />}
          {mode === "ma" ? (
            <MaControls fast={fast} slow={slow} onChange={(f, sl) => set({ fast: String(f), slow: String(sl) })} />
          ) : (
            <StrategySelect value={s.strategy} onChange={(strategy) => set({ strategy })} />
          )}
          {/* 不用 Field（label 元素）：点击标签会触发里面的第一个按钮 */}
          <div className="flex flex-col gap-1 text-xs text-zinc-500">
            <span>回测区间</span>
            <Segmented
              value={range}
              options={mode === "portfolio" ? PORTFOLIO_RANGES : RANGES}
              onChange={(range) => set({ range })}
            />
          </div>
          {mode === "portfolio" && (
            <NumberField
              label="最多同时持有（只）"
              hint="1 到 20。每次买入时，用现金除以空余的仓位数，平均分给新买的股票"
              value={s.positions}
              onCommit={(positions) => set({ positions })}
              width="w-16"
            />
          )}
          <NumberField label="初始资金（元）" value={s.capital} onCommit={(capital) => set({ capital })} width="w-28" />
        </div>
        <details className="mt-4 text-sm">
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
        {mode !== "portfolio" && !s.code && <Quickpick onPick={(code) => set({ code })} />}
      </Card>

      {mode === "ma" && s.code && (
        <Loaded query={ma}>
          {(d) => (
            <Result
              thscode={d.thscode}
              name={d.name}
              label={`MA${d.fast} / MA${d.slow}`}
              summary={d.summary}
              equity={d.equity}
              trades={d.trades}
              log={log}
              onLog={onLog}
            />
          )}
        </Loaded>
      )}
      {mode === "strategy" && s.code && strategyId > 0 && (
        <Loaded query={single}>
          {(d) => (
            <Result
              thscode={d.thscode}
              name={d.name}
              label={
                <Link to={`/screener/${d.strategy_id}`} className="hover:underline">
                  策略「{d.strategy_name}」
                </Link>
              }
              summary={d.summary}
              equity={d.equity}
              trades={d.trades}
              log={log}
              onLog={onLog}
            />
          )}
        </Loaded>
      )}
      {mode === "portfolio" && strategyId > 0 && (
        <Loaded query={portfolio} pending="正在逐只计算全市场的日线，通常需要几秒到十几秒">
          {(d) => <PortfolioView data={d} log={log} onLog={onLog} />}
        </Loaded>
      )}

      <Card title="回测规则">
        <ul className="list-disc space-y-1 pl-5 text-sm text-zinc-600 dark:text-zinc-400">
          {mode === "portfolio" ? (
            <>
              <li>每个交易日收盘后用策略选股，下一个交易日开盘买入；同一天候选太多时按策略的排序先买靠前的。</li>
              <li>
                最多同时持有设定的只数，已经持有的股票不重复买入；仓位已满时放弃新的信号。卖出规则满足任意一条后，下一个交易日开盘卖出。
              </li>
              <li>
                为了让全市场几千只股票能在几秒内算完，收益按等比前复权价计算（相当于分红再投资），允许买零股，不按一手
                100 股取整；ST 按现在的简称判断。股票池里的「只看强势板块」在回测中不生效。
              </li>
              <li>
                开盘一字涨停买不进时放弃这次买入；一字跌停卖不出时顺延到下一个交易日。手续费按上面的交易成本扣除。
              </li>
              <li>
                对比基准是同期的沪深 300
                指数（没有它的日线时用上证指数），不计分红；本地没有区间开头的指数日线时不显示基准。
              </li>
              <li>历史表现不代表未来，回测结果仅供研究。</li>
            </>
          ) : (
            <>
              <li>
                {mode === "ma"
                  ? "收盘后才知道当天是否金叉或死叉，所以在下一个交易日开盘时按开盘价买入或卖出。"
                  : "收盘后才知道当天是否符合条件，所以在下一个交易日开盘时按开盘价买入；卖出规则（卖出信号、止损、止盈、持有到期）满足任意一条后，下一个交易日开盘卖出。"}
              </li>
              <li>只做多，买入时用全部可用资金、按每手 100 股买；卖出时全部卖出。剩余不够一手的钱留作现金。</li>
              <li>开盘就一字涨停买不进、一字跌停卖不出时，顺延到下一个交易日。</li>
              <li>
                信号按等比前复权价判断；买卖和市值按交易所实际价格计算。持有期间的现金分红（税前）计入现金、不自动再买入，送转股按比例增加股数，不参与配股。手续费按上面的交易成本扣除。
              </li>
              <li>
                对比基准「买入持有」：同样的钱在区间第一天开盘全部买入，一直拿到最后，分红送转同样处理，不计手续费。
              </li>
              <li>历史表现不代表未来，回测结果仅供研究。</li>
            </>
          )}
        </ul>
      </Card>
    </>
  );
}

/** 查询结果的三种状态 */
function Loaded<T>({
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

function StrategySelect({ value, onChange }: { value: string; onChange: (v: string) => void }) {
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
function NumberField({
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

/** 还没选股票时，从自选中快速选一只 */
function Quickpick({ onPick }: { onPick: (code: string) => void }) {
  const list = useListWatchlist();
  const items = list.data?.items ?? [];
  return (
    <div className="mt-4 text-sm text-zinc-500">
      {items.length === 0 ? (
        <p>搜索一只股票开始回测，也可以先在「自选」里收藏常看的股票。</p>
      ) : (
        <div className="flex flex-wrap items-center gap-2">
          <span>从自选中选择：</span>
          {items.map((x) => (
            <button key={x.thscode} type="button" className={buttonClass("ghost")} onClick={() => onPick(x.thscode)}>
              {x.name}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

function Result({
  thscode,
  name,
  label,
  summary: m,
  equity,
  trades,
  log,
  onLog,
}: {
  thscode: string;
  name: string;
  label: ReactNode;
  summary: Summary;
  equity: EquityPoint[];
  trades: Trade[];
  log: boolean;
  onLog: (v: boolean) => void;
}) {
  const beat = m.total_return - m.benchmark_return;
  return (
    <>
      <Card
        title={
          <span className="flex flex-wrap items-baseline gap-2">
            <Link to={`/securities/${thscode}`} className="hover:underline">
              {name}
            </Link>
            <span className="num font-mono text-xs font-normal text-zinc-500">{thscode}</span>
            <span className="text-xs font-normal text-zinc-500">
              {label} · {m.start} 至 {m.end}（{fmtInt(m.days)} 个交易日）
            </span>
          </span>
        }
      >
        <p className="mb-4 text-sm">
          {fmtBig(m.capital)} 元按这个策略交易，最后变成{" "}
          <b className={upDownClass(m.total_return)}>{fmtBig(m.final_equity)} 元</b>（{fmtPercent(m.total_return)}
          ）；同期一直持有是 <b className={upDownClass(m.benchmark_return)}>{fmtPercent(m.benchmark_return)}</b>。
          {beat >= 0 ? (
            <span className="text-up">策略跑赢了买入持有 {fmtPercent(beat)}。</span>
          ) : (
            <span className="text-down">策略跑输了买入持有 {fmtPercent(-beat)}。</span>
          )}
        </p>
        <div className="grid grid-cols-2 gap-4 sm:grid-cols-4">
          <Stat
            label="策略收益"
            value={<span className={upDownClass(m.total_return)}>{fmtPercent(m.total_return)}</span>}
            sub={
              m.annual_return === null || m.annual_return === undefined
                ? "区间太短，不算年化"
                : `年化 ${fmtPercent(m.annual_return)}`
            }
          />
          <Stat
            label="买入持有收益"
            value={<span className={upDownClass(m.benchmark_return)}>{fmtPercent(m.benchmark_return)}</span>}
            sub={
              m.benchmark_annual_return === null || m.benchmark_annual_return === undefined
                ? undefined
                : `年化 ${fmtPercent(m.benchmark_annual_return)}`
            }
          />
          <Stat
            label="最大回撤"
            value={<span className="text-down">{fmtPercent(-m.max_drawdown)}</span>}
            sub={`买入持有 ${fmtPercent(-m.benchmark_max_drawdown)}，从最高点最多跌了多少`}
          />
          <WinStat closed={m.closed_trades} winRate={m.win_rate} payoff={m.payoff_ratio} />
          <Stat
            label="平均持有"
            value={
              m.avg_bars_held === null || m.avg_bars_held === undefined ? "—" : `${m.avg_bars_held.toFixed(0)} 个交易日`
            }
          />
          <Stat label="持仓时间占比" value={fmtPercent(m.exposure, 0).replace("+", "")} sub="其余时间拿着现金" />
          <Stat label="手续费合计" value={`${fmtBig(m.fees)} 元`} sub={`收到分红 ${fmtBig(m.dividends)} 元（税前）`} />
          <Stat
            label="未成交"
            value={`${m.deferred_orders + m.skipped_buys} 次`}
            sub={`一字板顺延 ${m.deferred_orders} 次，资金不足一手 ${m.skipped_buys} 次`}
          />
        </div>
      </Card>

      <EquityCard log={log} onLog={onLog} note="蓝线是策略，灰线是同期买入持有。">
        <EquityChart points={equity} capital={m.capital} log={log} />
      </EquityCard>

      <Card title={`交易记录（${trades.length} 笔）`}>
        <TradeTable trades={trades} />
      </Card>
    </>
  );
}

/** 交易次数、胜率和盈亏比 */
function WinStat({ closed, winRate, payoff }: { closed: number; winRate?: number | null; payoff?: number | null }) {
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

function EquityCard({
  log,
  onLog,
  note,
  children,
}: {
  log: boolean;
  onLog: (v: boolean) => void;
  note: string;
  children: ReactNode;
}) {
  return (
    <Card
      title="账户总值"
      extra={
        <Segmented
          value={log ? "log" : "normal"}
          options={[
            { value: "normal", label: "普通坐标" },
            { value: "log", label: "对数坐标", hint: "同样的涨跌幅在图上高度相同，适合看很长的区间" },
          ]}
          onChange={(v) => onLog(v === "log")}
        />
      }
    >
      {children}
      <p className="mt-2 text-xs text-zinc-500">{note}</p>
    </Card>
  );
}

function PortfolioView({ data, log, onLog }: { data: PortfolioResult; log: boolean; onLog: (v: boolean) => void }) {
  const m = data.summary;
  const bench = m.benchmark_name;
  const beat =
    m.benchmark_return === null || m.benchmark_return === undefined ? null : m.total_return - m.benchmark_return;
  return (
    <>
      <Card
        title={
          <span className="flex flex-wrap items-baseline gap-2">
            <Link to={`/screener/${data.strategy_id}`} className="hover:underline">
              策略「{data.strategy_name}」
            </Link>
            <span className="text-xs font-normal text-zinc-500">
              全市场 · 最多持有 {data.max_positions} 只 · {m.start} 至 {m.end}（{fmtInt(m.days)} 个交易日）
            </span>
          </span>
        }
      >
        <p className="mb-4 text-sm">
          {fmtBig(m.capital)} 元按这个策略在全市场选股交易，最后变成{" "}
          <b className={upDownClass(m.total_return)}>{fmtBig(m.final_equity)} 元</b>（{fmtPercent(m.total_return)}）
          {bench && beat !== null ? (
            <>
              ；同期{bench}是 <b className={upDownClass(m.benchmark_return)}>{fmtPercent(m.benchmark_return)}</b>。
              {beat >= 0 ? (
                <span className="text-up">策略跑赢了指数 {fmtPercent(beat)}。</span>
              ) : (
                <span className="text-down">策略跑输了指数 {fmtPercent(-beat)}。</span>
              )}
            </>
          ) : (
            "。"
          )}
        </p>
        <div className="grid grid-cols-2 gap-4 sm:grid-cols-4">
          <Stat
            label="策略收益"
            value={<span className={upDownClass(m.total_return)}>{fmtPercent(m.total_return)}</span>}
            sub={
              m.annual_return === null || m.annual_return === undefined
                ? "区间太短，不算年化"
                : `年化 ${fmtPercent(m.annual_return)}`
            }
          />
          <Stat
            label="最大回撤"
            value={<span className="text-down">{fmtPercent(-m.max_drawdown)}</span>}
            sub="从最高点最多跌了多少"
          />
          <WinStat closed={m.closed_trades} winRate={m.win_rate} payoff={m.payoff_ratio} />
          <Stat
            label="平均持有"
            value={
              m.avg_bars_held === null || m.avg_bars_held === undefined ? "—" : `${m.avg_bars_held.toFixed(0)} 个交易日`
            }
            sub={`平均每天持有 ${m.avg_positions.toFixed(1)} 只`}
          />
          <Stat label="买入信号" value={`${fmtInt(m.signals)} 次`} sub={`仓位已满没买 ${fmtInt(m.skipped_full)} 次`} />
          <Stat label="手续费合计" value={`${fmtBig(m.fees)} 元`} />
        </div>
      </Card>

      <EquityCard
        log={log}
        onLog={onLog}
        note={
          bench ? `蓝线是策略，灰线是同样本金买入${bench}（不计分红）。` : "本地没有区间开头的指数日线，不显示基准。"
        }
      >
        <EquityChart points={data.equity} capital={m.capital} log={log} benchmarkLabel={bench ?? "基准"} />
      </EquityCard>

      <Card title={`交易记录（${data.trades.length} 笔${data.trades.length >= 500 ? "，只显示最近的" : ""}）`}>
        {data.trades.length === 0 ? (
          <Empty>区间内没有发生过买入</Empty>
        ) : (
          <div className="max-h-[32rem] overflow-y-auto">
            <Table>
              <thead>
                <tr>
                  <th>股票</th>
                  <th title="符合条件的那天；第二天开盘买入">信号日</th>
                  <th>买入日期</th>
                  <th className="text-right!">买入价</th>
                  <th>卖出日期</th>
                  <th className="text-right!">卖出价</th>
                  <th>卖出原因</th>
                  <th className="text-right!">持有（交易日）</th>
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
                    <td className="num text-zinc-500">{t.signal_date}</td>
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
      </Card>
    </>
  );
}

function TradeTable({ trades }: { trades: Trade[] }) {
  if (trades.length === 0) return <Empty>区间内没有发生过买入</Empty>;
  return (
    <div className="max-h-[32rem] overflow-y-auto">
      <Table>
        <thead>
          <tr>
            <th>买入日期</th>
            <th className="text-right!">买入价</th>
            <th>卖出日期</th>
            <th className="text-right!">卖出价</th>
            <th>卖出原因</th>
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
  );
}
