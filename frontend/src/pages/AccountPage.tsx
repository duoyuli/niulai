import { useQueryClient } from "@tanstack/react-query";
import { clsx } from "clsx";
import { useState } from "react";
import { Link, useNavigate, useParams } from "react-router";
import {
  getGetAccountQueryKey,
  getListAccountsQueryKey,
  useAccountBacktest,
  useAccountReview,
  useAddTrade,
  useAdviceHistory,
  useDeleteAccount,
  useGetAccount,
  useListTrades,
  useMarkChecked,
  useUpdateAccount,
} from "../api/generated/accounts/accounts";
import { AccountBacktestRange, type AccountDetail, type TradeKind } from "../api/generated/model";
import { AdviceView } from "../components/AdviceView";
import { PositionsEditor } from "../components/PositionsEditor";
import { BacktestLine, type FullRules, RulesEditor } from "../components/RulesEditor";
import { SecuritySearch } from "../components/SecuritySearch";
import {
  buttonClass,
  Card,
  Empty,
  errorText,
  Field,
  inputClass,
  QueryView,
  Segmented,
  Stat,
  Table,
} from "../components/ui";
import { useSearchState } from "../hooks/useSearchState";
import { ACTIONS, TRADE_KINDS, TRADE_SOURCES, yuan } from "../lib/accounts";
import { fmtDateTime, fmtPctValue, fmtPrice, upDownClass } from "../lib/format";

const TABS = [
  { value: "advice", label: "今日建议" },
  { value: "positions", label: "持仓" },
  { value: "records", label: "记录" },
  { value: "settings", label: "设置" },
] as const;

export function AccountPage() {
  const id = Number(useParams().id);
  const [s, set] = useSearchState({ tab: "advice" });
  const detail = useGetAccount(id);
  return (
    <QueryView query={detail}>
      {(d) => (
        <>
          <div className="flex flex-wrap items-end justify-between gap-3">
            <div>
              <Link to="/accounts" className="text-xs text-zinc-500 hover:text-zinc-900 dark:hover:text-zinc-100">
                ← 我的账户
              </Link>
              <h1 className="mt-1 text-xl font-semibold">{d.summary.account.name}</h1>
            </div>
            <div className="flex flex-wrap gap-6">
              <Stat label="总资产" value={yuan(d.summary.total_assets)} />
              <Stat
                label="今日盈亏"
                value={<span className={upDownClass(d.summary.day_pnl)}>{yuan(d.summary.day_pnl)}</span>}
              />
              <Stat label="现金" value={yuan(d.summary.account.cash)} />
              <Stat label="持仓" value={`${d.summary.positions} 只`} />
            </div>
          </div>
          {d.summary.check_hint && <CheckHint d={d} />}
          <div>
            <Segmented
              value={s.tab as (typeof TABS)[number]["value"]}
              options={TABS}
              onChange={(tab) => set({ tab })}
            />
          </div>
          {s.tab === "advice" && <AdviceView accountId={id} />}
          {s.tab === "positions" && <Positions d={d} />}
          {s.tab === "records" && <Records id={id} />}
          {s.tab === "settings" && <Settings d={d} />}
        </>
      )}
    </QueryView>
  );
}

function CheckHint({ d }: { d: AccountDetail }) {
  const qc = useQueryClient();
  const id = d.summary.account.account_id;
  const mark = useMarkChecked({
    mutation: { onSuccess: () => qc.invalidateQueries({ queryKey: getGetAccountQueryKey(id) }) },
  });
  return (
    <p className="rounded-lg bg-amber-50 px-3 py-2 text-sm text-amber-800 dark:bg-amber-500/10 dark:text-amber-300">
      {d.summary.check_hint}。
      <button
        type="button"
        className="ml-2 underline"
        disabled={mark.isPending}
        onClick={() => mark.mutate({ accountId: id })}
      >
        和券商一致
      </button>
    </p>
  );
}

function Positions({ d }: { d: AccountDetail }) {
  const [mode, setMode] = useState<"view" | "calibrate" | "trade">("view");
  const p = d.summary.account.rules as FullRules;
  return (
    <Card
      title="持仓"
      extra={
        <div className="flex gap-2">
          <button
            type="button"
            className={buttonClass(mode === "trade" ? "primary" : "ghost")}
            onClick={() => setMode(mode === "trade" ? "view" : "trade")}
          >
            记一笔
          </button>
          <button
            type="button"
            className={buttonClass(mode === "calibrate" ? "primary" : "ghost")}
            onClick={() => setMode(mode === "calibrate" ? "view" : "calibrate")}
          >
            校准持仓和现金
          </button>
        </div>
      }
    >
      {mode === "calibrate" && (
        <div className="mb-4">
          <PositionsEditor detail={d} onDone={() => setMode("view")} />
        </div>
      )}
      {mode === "trade" && <TradeForm d={d} onDone={() => setMode("view")} />}
      {d.positions.length === 0 ? (
        <Empty>还没有持仓。点「校准持仓和现金」把券商里的持仓填进来，或者照着建议买入后点「已执行」。</Empty>
      ) : (
        <Table>
          <thead>
            <tr>
              <th>股票</th>
              <th>股数</th>
              <th>现价</th>
              <th>市值 / 占比</th>
              <th>盈亏</th>
              <th title={`跌到止损价（比成本亏 ${p.stop_loss_pct}%）或收盘跌破 ${p.trend_ma} 日均线时，建议会提示卖出`}>
                止损价 / {p.trend_ma} 日均线
              </th>
              <th>买入依据</th>
            </tr>
          </thead>
          <tbody>
            {d.positions.map((v) => (
              <tr key={v.position.thscode}>
                <td>
                  <Link to={`/securities/${v.position.thscode}`} className="hover:underline">
                    {v.name}
                  </Link>
                  <span className="ml-1 text-xs text-zinc-400">{v.position.thscode}</span>
                  {v.delisted && <span className="ml-1 text-xs text-red-600">已退市</span>}
                </td>
                <td className="num">{v.position.shares}</td>
                <td className="num">
                  {fmtPrice(v.close)}{" "}
                  <span className={clsx("text-xs", upDownClass(v.change_pct))}>{fmtPctValue(v.change_pct)}</span>
                </td>
                <td className="num">
                  {yuan(v.market_value)} <span className="text-xs text-zinc-500">{v.weight_pct.toFixed(1)}%</span>
                </td>
                <td className={clsx("num", upDownClass(v.pnl))}>
                  {v.pnl === null || v.pnl === undefined ? (
                    <span className="text-xs text-zinc-400">没填成本</span>
                  ) : (
                    `${yuan(v.pnl)}（${fmtPctValue(v.pnl_pct, 1)}）`
                  )}
                </td>
                <td className="num text-xs">
                  {v.stop_price ? fmtPrice(v.stop_price) : "—"} / {fmtPrice(v.trend_ma)}
                  {v.trend_gap_pct !== null && v.trend_gap_pct !== undefined && (
                    <span className={clsx("ml-1", upDownClass(v.trend_gap_pct))}>
                      （{fmtPctValue(v.trend_gap_pct, 1)}）
                    </span>
                  )}
                </td>
                <td className="text-xs text-zinc-500">{v.strategy_name ?? "手动录入"}</td>
              </tr>
            ))}
          </tbody>
        </Table>
      )}
    </Card>
  );
}

function TradeForm({ d, onDone }: { d: AccountDetail; onDone: () => void }) {
  const id = d.summary.account.account_id;
  const qc = useQueryClient();
  const [kind, setKind] = useState<TradeKind>("buy");
  const [code, setCode] = useState("");
  const [shares, setShares] = useState("");
  const [price, setPrice] = useState("");
  const [amount, setAmount] = useState("");
  const [date, setDate] = useState("");
  const add = useAddTrade({
    mutation: {
      onSuccess: (r) => {
        qc.setQueryData(getGetAccountQueryKey(id), r);
        qc.invalidateQueries({ queryKey: getListAccountsQueryKey() });
        qc.invalidateQueries();
        onDone();
      },
    },
  });
  const stock = kind === "buy" || kind === "sell";
  return (
    <div className="mb-4 flex flex-wrap items-end gap-3 rounded-lg bg-zinc-50 p-3 dark:bg-zinc-800/50">
      <Field label="类型">
        <select className={inputClass} value={kind} onChange={(e) => setKind(e.target.value as TradeKind)}>
          {(["buy", "sell", "cash_in", "cash_out"] as const).map((k) => (
            <option key={k} value={k}>
              {TRADE_KINDS[k]}
            </option>
          ))}
        </select>
      </Field>
      {stock ? (
        <>
          {kind === "sell" ? (
            <Field label="股票">
              <select className={inputClass} value={code} onChange={(e) => setCode(e.target.value)}>
                <option value="">选一只</option>
                {d.positions.map((v) => (
                  <option key={v.position.thscode} value={v.position.thscode}>
                    {v.name}（{v.position.shares} 股）
                  </option>
                ))}
              </select>
            </Field>
          ) : (
            <div className="flex flex-col gap-1">
              <SecuritySearch label="股票" onPick={setCode} width="w-48" />
              {code && <span className="text-xs text-zinc-500">已选 {code}</span>}
            </div>
          )}
          <Field label="股数">
            <input
              className={`${inputClass} w-24`}
              inputMode="numeric"
              value={shares}
              onChange={(e) => setShares(e.target.value)}
            />
          </Field>
          <Field label="成交价">
            <input
              className={`${inputClass} w-24`}
              inputMode="decimal"
              value={price}
              onChange={(e) => setPrice(e.target.value)}
            />
          </Field>
        </>
      ) : (
        <Field label="金额（元）">
          <input
            className={`${inputClass} w-32`}
            inputMode="decimal"
            value={amount}
            onChange={(e) => setAmount(e.target.value)}
          />
        </Field>
      )}
      <Field label="日期（默认今天）">
        <input className={`${inputClass} w-36`} type="date" value={date} onChange={(e) => setDate(e.target.value)} />
      </Field>
      <button
        type="button"
        className={buttonClass()}
        disabled={
          add.isPending || (stock ? !code || !(Number(shares) > 0) || !(Number(price) > 0) : !(Number(amount) > 0))
        }
        onClick={() =>
          add.mutate({
            accountId: id,
            data: stock
              ? {
                  kind,
                  thscode: code,
                  shares: Math.round(Number(shares)),
                  price: Number(price),
                  trade_date: date || null,
                }
              : { kind, amount: Number(amount), trade_date: date || null },
          })
        }
      >
        {add.isPending ? "记录中…" : "记下来"}
      </button>
      <p className="basis-full text-xs text-zinc-500">
        买卖的费用按账户费率估算；记完后今天的建议会按新的持仓重新生成。
      </p>
      {!!add.error && <p className="basis-full text-sm text-red-600">{errorText(add.error)}</p>}
    </div>
  );
}

function Records({ id }: { id: number }) {
  const history = useAdviceHistory(id);
  const review = useAccountReview(id);
  const trades = useListTrades(id);
  const [day, setDay] = useState<string | null>(null);
  return (
    <div className="flex flex-col gap-4">
      {day && (
        <div>
          <button type="button" className="mb-2 text-sm text-accent hover:underline" onClick={() => setDay(null)}>
            ← 收起 {day} 的建议
          </button>
          <AdviceView accountId={id} date={day} />
        </div>
      )}
      <Card title="过去的建议准不准">
        <QueryView query={review}>
          {(r) => (
            <>
              <p className="text-sm">
                买入建议之后 20 个交易日平均{" "}
                <span className={clsx("num", upDownClass(r.buys.avg_20d_pct))}>
                  {fmtPctValue(r.buys.avg_20d_pct, 1)}
                </span>
                （{r.buys.samples} 条）；卖出、减仓建议之后平均{" "}
                <span className={clsx("num", upDownClass(r.sells.avg_20d_pct))}>
                  {fmtPctValue(r.sells.avg_20d_pct, 1)}
                </span>
                （{r.sells.samples} 条，跌了说明卖对了）。
              </p>
              {r.items.length > 0 && (
                <Table>
                  <thead>
                    <tr>
                      <th>日期</th>
                      <th>建议</th>
                      <th>处理</th>
                      <th>5 日后</th>
                      <th>10 日后</th>
                      <th>20 日后</th>
                    </tr>
                  </thead>
                  <tbody>
                    {r.items.slice(0, 60).map((i) => (
                      <tr key={`${i.trade_date}-${i.action}-${i.thscode}`}>
                        <td className="num text-xs">{i.trade_date}</td>
                        <td>
                          <span className={clsx("mr-1 rounded px-1 text-xs", ACTIONS[i.action].chip)}>
                            {ACTIONS[i.action].label}
                          </span>
                          {i.name}
                        </td>
                        <td className="text-xs text-zinc-500">
                          {i.state === "executed" ? "已执行" : i.state === "skipped" ? "不做" : "没处理"}
                        </td>
                        {(
                          [
                            ["5", i.after_5d_pct],
                            ["10", i.after_10d_pct],
                            ["20", i.after_20d_pct],
                          ] as const
                        ).map(([h, v]) => (
                          <td key={h} className={clsx("num", upDownClass(v))}>
                            {fmtPctValue(v, 1)}
                          </td>
                        ))}
                      </tr>
                    ))}
                  </tbody>
                </Table>
              )}
            </>
          )}
        </QueryView>
      </Card>
      <Card title="每天的建议">
        <QueryView query={history}>
          {(list) =>
            list.length === 0 ? (
              <Empty>还没有</Empty>
            ) : (
              <ul className="flex flex-col divide-y divide-zinc-100 text-sm dark:divide-zinc-800">
                {list.map((h) => (
                  <li key={h.advice_id}>
                    <button
                      type="button"
                      className="flex w-full flex-wrap gap-x-4 py-2 text-left hover:opacity-80"
                      onClick={() => setDay(h.trade_date)}
                    >
                      <span className="num w-24">{h.trade_date}</span>
                      <span className="flex-1">{h.headline}</span>
                      <span className="text-xs text-zinc-500">
                        已执行 {h.executed} · 不做 {h.skipped}
                      </span>
                    </button>
                  </li>
                ))}
              </ul>
            )
          }
        </QueryView>
      </Card>
      <Card title="流水（持仓和现金的每一次变动）">
        <QueryView query={trades}>
          {(list) => (
            <Table>
              <thead>
                <tr>
                  <th>日期</th>
                  <th>类型</th>
                  <th>股票</th>
                  <th>股数</th>
                  <th>价格</th>
                  <th>现金变化</th>
                  <th>说明</th>
                </tr>
              </thead>
              <tbody>
                {list.map((t) => (
                  <tr key={t.trade_id}>
                    <td className="num text-xs">{t.trade_date}</td>
                    <td>
                      {TRADE_KINDS[t.kind]}
                      <span className="ml-1 text-xs text-zinc-400">{TRADE_SOURCES[t.source]}</span>
                    </td>
                    <td>{t.name ?? t.thscode ?? ""}</td>
                    <td className="num">{t.shares ?? ""}</td>
                    <td className="num">{t.price ? fmtPrice(t.price) : ""}</td>
                    <td className={clsx("num", upDownClass(t.amount === 0 ? null : t.amount))}>
                      {t.amount === 0 ? "" : yuan(t.amount, 2)}
                    </td>
                    <td className="max-w-80 text-xs text-zinc-500" title={fmtDateTime(t.created_at)}>
                      {t.note}
                    </td>
                  </tr>
                ))}
              </tbody>
            </Table>
          )}
        </QueryView>
      </Card>
    </div>
  );
}

const BACKTEST_RANGES = [
  { value: AccountBacktestRange["3m"], label: "近 3 个月" },
  { value: AccountBacktestRange["6m"], label: "近 6 个月" },
  { value: AccountBacktestRange["1y"], label: "近一年" },
  { value: AccountBacktestRange.ytd, label: "今年以来" },
  { value: AccountBacktestRange["2y"], label: "近两年" },
  { value: AccountBacktestRange["3y"], label: "近 3 年" },
];

function Settings({ d }: { d: AccountDetail }) {
  const acc = d.summary.account;
  const id = acc.account_id;
  const nav = useNavigate();
  const qc = useQueryClient();
  const [name, setName] = useState(acc.name);
  const [rules, setRules] = useState<FullRules>(acc.rules as FullRules);
  const [rate, setRate] = useState(String(acc.fees.commission_rate * 10000));
  const [minFee, setMinFee] = useState(String(acc.fees.min_commission));
  const [note, setNote] = useState(acc.note);
  const [confirm, setConfirm] = useState("");
  const [saved, setSaved] = useState(false);
  const update = useUpdateAccount({
    mutation: {
      onSuccess: (r) => {
        qc.setQueryData(getGetAccountQueryKey(id), r);
        qc.invalidateQueries();
        setSaved(true);
      },
    },
  });
  const remove = useDeleteAccount({
    mutation: {
      onSuccess: () => {
        qc.invalidateQueries({ queryKey: getListAccountsQueryKey() });
        nav("/accounts");
      },
    },
  });
  const backtest = useAccountBacktest();
  const [range, setRange] = useState<AccountBacktestRange>(AccountBacktestRange.ytd);
  return (
    <div className="flex flex-col gap-4">
      <Card title="名字和备注">
        <div className="flex flex-wrap items-end gap-3">
          <Field label="账户名">
            <input
              className={`${inputClass} w-72`}
              maxLength={30}
              value={name}
              onChange={(e) => setName(e.target.value)}
            />
          </Field>
          <Field label="备注">
            <input className={`${inputClass} w-72`} value={note} onChange={(e) => setNote(e.target.value)} />
          </Field>
        </div>
      </Card>
      <Card title="选股和风控规则（高级设置）">
        <RulesEditor value={rules} onChange={setRules} />
      </Card>
      <Card title="费率">
        <div className="flex flex-wrap items-end gap-3">
          <Field label="佣金率（万分之几）">
            <input
              className={`${inputClass} w-28`}
              inputMode="decimal"
              value={rate}
              onChange={(e) => setRate(e.target.value)}
            />
          </Field>
          <Field label="每笔最低佣金（元）">
            <input
              className={`${inputClass} w-28`}
              inputMode="decimal"
              value={minFee}
              onChange={(e) => setMinFee(e.target.value)}
            />
          </Field>
        </div>
        <p className="mt-2 text-xs text-zinc-500">印花税按国家税率（卖出时千分之 0.5）自动算。</p>
      </Card>
      <div className="flex items-center gap-3">
        <button
          type="button"
          className={buttonClass()}
          disabled={update.isPending || !name.trim() || rules.strategies?.length === 0}
          onClick={() => {
            setSaved(false);
            update.mutate({
              accountId: id,
              data: {
                name: name.trim(),
                rules,
                fees: { commission_rate: (Number(rate) || 0) / 10000, min_commission: Number(minFee) || 0 },
                note,
              },
            });
          }}
        >
          {update.isPending ? "保存中…" : "保存，并按新设置重新生成建议"}
        </button>
        {saved && <span className="text-sm text-emerald-600">已保存</span>}
        {!!update.error && <span className="text-sm text-red-600">{errorText(update.error)}</span>}
      </div>
      <Card title="用这个账户的规则回测">
        <p className="text-sm text-zinc-600 dark:text-zinc-400">
          按保存后的规则，假设这段时间每天都照着建议做，账户会怎样。哪些策略算有效，每年只用之前 3
          年的数据判断。全市场计算，要几十秒到一两分钟。
        </p>
        <div className="mt-2 flex flex-wrap items-center gap-3">
          <Segmented value={range} options={BACKTEST_RANGES} onChange={setRange} />
          <button
            type="button"
            className={buttonClass("ghost")}
            disabled={backtest.isPending}
            onClick={() => backtest.mutate({ accountId: id, params: { range } })}
          >
            {backtest.isPending ? "计算中…" : "开始回测"}
          </button>
        </div>
        {backtest.data && (
          <div className="mt-2">
            <BacktestLine b={backtest.data} />
          </div>
        )}
        {!!backtest.error && <p className="mt-2 text-sm text-red-600">{errorText(backtest.error)}</p>}
      </Card>
      <Card title="删除账户">
        <p className="text-sm text-zinc-600 dark:text-zinc-400">
          持仓、流水和所有历史建议都会删除，不能恢复。输入账户名「{acc.name}」确认：
        </p>
        <div className="mt-2 flex items-center gap-2">
          <input className={`${inputClass} w-60`} value={confirm} onChange={(e) => setConfirm(e.target.value)} />
          <button
            type="button"
            className="rounded-lg bg-red-600 px-3 py-1.5 text-sm text-white disabled:opacity-40"
            disabled={confirm !== acc.name || remove.isPending}
            onClick={() => remove.mutate({ accountId: id })}
          >
            删除
          </button>
        </div>
      </Card>
    </div>
  );
}
