import { keepPreviousData } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { Link } from "react-router";
import { useMaCrossBacktest } from "../api/generated/backtest/backtest";
import type { BacktestResult, Trade } from "../api/generated/model";
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

/** 交易成本的默认值，与后端一致：佣金万 2.5、最低 5 元，卖出印花税千分之 0.5 */
const DEFAULTS = { capital: "100000", commission: "2.5", minFee: "5", stamp: "0.5" };

export function BacktestPage() {
  const [s, set] = useSearchState({ code: "", fast: "15", slow: "30", range: "10", ...DEFAULTS, log: "" });
  const fast = Number(s.fast) || 15;
  const slow = Number(s.slow) || 30;
  const query = useMaCrossBacktest(
    {
      thscode: s.code,
      fast,
      slow,
      start: rangeStart(s.range),
      capital: Number(s.capital) || undefined,
      commission_rate: Number(s.commission) / 10000,
      min_commission: Number(s.minFee),
      stamp_tax_rate: Number(s.stamp) / 1000,
    },
    { query: { enabled: s.code !== "", placeholderData: keepPreviousData } },
  );

  return (
    <>
      <PageHeader
        title="策略回测"
        desc="用历史数据检验均线策略：快线上穿慢线（金叉）后买入，下穿（死叉）后卖出，看看这样做过去能赚多少、最多亏过多少，并和一直持有不动比较。"
      />
      <Card>
        <div className="flex flex-wrap items-end gap-4">
          <SecuritySearch label="股票" onPick={(code) => set({ code })} />
          <MaControls fast={fast} slow={slow} onChange={(f, sl) => set({ fast: String(f), slow: String(sl) })} />
          {/* 不用 Field（label 元素）：点击标签会触发里面的第一个按钮 */}
          <div className="flex flex-col gap-1 text-xs text-zinc-500">
            <span>回测区间</span>
            <Segmented value={s.range} options={RANGES} onChange={(range) => set({ range })} />
          </div>
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
        {!s.code && <Quickpick onPick={(code) => set({ code })} />}
      </Card>

      {s.code &&
        (query.error ? (
          <Card>
            <ErrorState error={query.error} onRetry={() => query.refetch()} />
          </Card>
        ) : !query.data ? (
          <Card>
            <Spinner label="回测中" />
          </Card>
        ) : (
          <Result data={query.data} log={s.log === "1"} onLog={(log) => set({ log: log ? "1" : "" })} />
        ))}

      <Card title="回测规则">
        <ul className="list-disc space-y-1 pl-5 text-sm text-zinc-600 dark:text-zinc-400">
          <li>收盘后才知道当天是否金叉或死叉，所以在下一个交易日开盘时按开盘价买入或卖出。</li>
          <li>只做多，买入时用全部可用资金、按每手 100 股买；卖出时全部卖出。剩余不够一手的钱留作现金。</li>
          <li>开盘就一字涨停买不进、一字跌停卖不出时，顺延到下一个交易日。</li>
          <li>
            金叉死叉按等比前复权价判断；买卖和市值按交易所实际价格计算。持有期间的现金分红（税前）计入现金、不自动再买入，送转股按比例增加股数，不参与配股。手续费按上面的交易成本扣除。
          </li>
          <li>对比基准「买入持有」：同样的钱在区间第一天开盘全部买入，一直拿到最后，分红送转同样处理，不计手续费。</li>
          <li>历史表现不代表未来，回测结果仅供研究。</li>
        </ul>
      </Card>
    </>
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

function Result({ data, log, onLog }: { data: BacktestResult; log: boolean; onLog: (v: boolean) => void }) {
  const m = data.summary;
  const beat = m.total_return - m.benchmark_return;
  return (
    <>
      <Card
        title={
          <span className="flex flex-wrap items-baseline gap-2">
            <Link to={`/securities/${data.thscode}`} className="hover:underline">
              {data.name}
            </Link>
            <span className="num font-mono text-xs font-normal text-zinc-500">{data.thscode}</span>
            <span className="text-xs font-normal text-zinc-500">
              MA{data.fast} / MA{data.slow} · {m.start} 至 {m.end}（{fmtInt(m.days)} 个交易日）
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
          <Stat
            label="交易次数"
            value={`${m.closed_trades} 次`}
            sub={
              m.win_rate === null || m.win_rate === undefined
                ? "还没有卖出过"
                : `赚钱的占 ${fmtPercent(m.win_rate, 0).replace("+", "")}`
            }
          />
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
        <EquityChart points={data.equity} capital={m.capital} log={log} />
        <p className="mt-2 text-xs text-zinc-500">蓝线是策略，灰线是同期买入持有。</p>
      </Card>

      <Card title={`交易记录（${data.trades.length} 笔）`}>
        <TradeTable trades={data.trades} />
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
