import { useQuery, useQueryClient } from "@tanstack/react-query";
import { clsx } from "clsx";
import { useState } from "react";
import { Link, useNavigate, useParams } from "react-router";
import {
  accountBacktest,
  getGetAccountQueryKey,
  getListAccountsQueryKey,
  getListCorpActionsQueryKey,
  getListTradesQueryKey,
  useAccountReview,
  useAddTrade,
  useAdviceHistory,
  useDeleteAccount,
  useGetAccount,
  useGetEquity,
  useIgnoreRights,
  useListCorpActions,
  useListTrades,
  useMarkChecked,
  useSubscribeRights,
  useUndoTrade,
  useUpdateAccount,
} from "../api/generated/accounts/accounts";
import type { AccountDetail, AccountTrade, TradeKind } from "../api/generated/model";
import { AccountEquityChart } from "../components/AccountEquityChart";
import { AdviceView } from "../components/AdviceView";
import { BacktestTrades } from "../components/BacktestTrades";
import { TaskRunning, useElapsedSeconds } from "../components/longTask";
import { PositionsEditor } from "../components/PositionsEditor";
import { type ChartRange, RangePicker, rangeQuery } from "../components/RangePicker";
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
import { BACKTEST_RANGES, fmtDateTime, fmtPctValue, fmtPrice, upDownClass } from "../lib/format";
import { ASSET_TYPES } from "../lib/labels";

const TABS = [
  { value: "advice", label: "运营" },
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
                label="当日盈亏"
                value={<span className={upDownClass(d.summary.day_pnl)}>{yuan(d.summary.day_pnl)}</span>}
              />
              <Stat label="现金" value={yuan(d.summary.account.cash)} />
              <Stat
                label="已实现盈亏"
                value={
                  d.summary.realized_pnl === null || d.summary.realized_pnl === undefined ? (
                    <span className="text-xs font-normal text-zinc-500">算不出（有卖出没填成本）</span>
                  ) : (
                    <span className={upDownClass(d.summary.realized_pnl)}>{yuan(d.summary.realized_pnl)}</span>
                  )
                }
                sub="已经卖掉的赚亏"
              />
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
  return (
    <>
      <RightsNotice d={d} />
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
                <th>证券</th>
                <th>股数</th>
                <th>现价</th>
                <th>市值 / 占比</th>
                <th>盈亏</th>
                <th title="开启账户保护线后，跌到保护价会建议卖出；策略自己的卖出规则单独显示在运营建议里">
                  账户保护价
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
                    <span className="ml-1 rounded bg-zinc-100 px-1 text-xs text-zinc-500 dark:bg-zinc-800">
                      {ASSET_TYPES[v.asset_type] ?? v.asset_type}
                    </span>
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
                  <td className="num text-xs">{v.stop_price ? fmtPrice(v.stop_price) : "已关闭"}</td>
                  <td className="text-xs text-zinc-500">{v.strategy_name ?? "手动录入"}</td>
                </tr>
              ))}
            </tbody>
          </Table>
        )}
        <p className="mt-2 text-xs text-zinc-500">
          上面的「盈亏」是还拿在手里的股票按现价算的浮动盈亏。已经卖掉的赚亏看页面顶部的「已实现盈亏」：按卖出时的平均成本算，买卖费用都扣掉了。
        </p>
      </Card>
    </>
  );
}

/** 待处理的配股：不自动买、不自动扣钱，由用户选「记一笔配股认购」或「暂不处理」 */
function RightsNotice({ d }: { d: AccountDetail }) {
  const id = d.summary.account.account_id;
  const qc = useQueryClient();
  const list = useListCorpActions(id);
  const [error, setError] = useState<string | null>(null);
  const refresh = (r: AccountDetail) => {
    qc.setQueryData(getGetAccountQueryKey(id), r);
    qc.invalidateQueries({ queryKey: getListCorpActionsQueryKey(id) });
    qc.invalidateQueries();
  };
  const subscribe = useSubscribeRights({
    mutation: {
      onSuccess: (r) => {
        setError(null);
        refresh(r);
      },
      onError: (e) => setError(errorText(e)),
    },
  });
  const ignore = useIgnoreRights({
    mutation: {
      onSuccess: () => {
        setError(null);
        qc.invalidateQueries({ queryKey: getListCorpActionsQueryKey(id) });
        qc.invalidateQueries();
      },
      onError: (e) => setError(errorText(e)),
    },
  });
  const items = list.data ?? [];
  if (items.length === 0) return null;
  return (
    <Card title="待处理的公司行动（配股）">
      <p className="mb-3 text-sm text-zinc-600 dark:text-zinc-400">
        配股是公司让老股东按持股数量、用比市场价便宜的价格再买一些新股。要不要认购由你决定：系统不会自动买、也不会自动扣钱。要认购就点「记一笔配股认购」，会从现金里扣配股款和费用；不想认购就点「暂不处理」，之后不再提醒。
      </p>
      <div className="flex flex-col gap-3">
        {items.map((r) => {
          const pay = r.amount + r.fee;
          return (
            <div key={`${r.thscode}-${r.ex_date}`} className="rounded-lg bg-amber-50 p-3 text-sm dark:bg-amber-500/10">
              <p>
                <span className="font-medium">{r.name}</span>
                <span className="ml-1 text-xs text-zinc-500">{r.thscode}</span>
                <span className="ml-2 text-xs text-zinc-500">除权日 {r.ex_date}</span>
              </p>
              <p className="mt-1">
                每 10 股配 {r.rights_per_10} 股，配股价 ¥{fmtPrice(r.rights_price)}；除权日持有 {r.held_shares}{" "}
                股，可以认购 {r.rights_shares} 股。
              </p>
              <p className="mt-1 text-xs text-zinc-500">
                要付 ¥{yuan(r.amount, 2)}（配股款）加约 ¥{yuan(r.fee, 2)}（费用），共 ¥{yuan(pay, 2)}。
              </p>
              <div className="mt-2 flex flex-wrap gap-2">
                <button
                  type="button"
                  className={buttonClass()}
                  disabled={subscribe.isPending || ignore.isPending}
                  onClick={() => {
                    const ok = window.confirm(
                      `认购 ${r.rights_shares} 股 ${r.name}？\n会从现金里扣 ¥${pay.toFixed(2)}（配股款 ¥${r.amount.toFixed(2)} 加费用 ¥${r.fee.toFixed(2)}），持仓成本按平均成本重新算。`,
                    );
                    if (ok) subscribe.mutate({ accountId: id, thscode: r.thscode, exDate: r.ex_date });
                  }}
                >
                  记一笔配股认购
                </button>
                <button
                  type="button"
                  className={buttonClass("ghost")}
                  disabled={subscribe.isPending || ignore.isPending}
                  onClick={() => {
                    if (window.confirm("暂不处理这次配股？不会买入、不会扣钱，之后不再提醒。"))
                      ignore.mutate({ accountId: id, thscode: r.thscode, exDate: r.ex_date });
                  }}
                >
                  暂不处理
                </button>
              </div>
            </div>
          );
        })}
      </div>
      {error && <p className="mt-2 text-sm text-red-600">{error}</p>}
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
      <p className="basis-full text-xs text-zinc-500">买卖的费用按账户费率估算；记完后运营建议会按新的持仓重新生成。</p>
      {!!add.error && <p className="basis-full text-sm text-red-600">{errorText(add.error)}</p>}
    </div>
  );
}

function Records({ id }: { id: number }) {
  const qc = useQueryClient();
  const history = useAdviceHistory(id);
  const review = useAccountReview(id);
  const trades = useListTrades(id);
  const [day, setDay] = useState<string | null>(null);
  const [undoError, setUndoError] = useState<string | null>(null);
  const undo = useUndoTrade({
    mutation: {
      onSuccess: (r) => {
        qc.setQueryData(getGetAccountQueryKey(id), r);
        qc.invalidateQueries({ queryKey: getListAccountsQueryKey() });
        qc.invalidateQueries({ queryKey: getListTradesQueryKey(id) });
        qc.invalidateQueries();
        setUndoError(null);
      },
      onError: (e) => setUndoError(errorText(e)),
    },
  });
  // 只有手动记的买入、卖出、转入、转出能撤销；按建议执行、分红送转、校准的流水用校准改
  const canUndo = (t: AccountTrade) =>
    t.source === "user" &&
    !t.reversed_at &&
    (t.kind === "buy" || t.kind === "sell" || t.kind === "cash_in" || t.kind === "cash_out");
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
      <EquityCard id={id} />
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
                {r.buys_executed.samples + r.sells_executed.samples > 0 && (
                  <>
                    其中你执行了的：买入之后平均{" "}
                    <span className={clsx("num", upDownClass(r.buys_executed.avg_20d_pct))}>
                      {fmtPctValue(r.buys_executed.avg_20d_pct, 1)}
                    </span>
                    （{r.buys_executed.samples} 条），卖出之后平均{" "}
                    <span className={clsx("num", upDownClass(r.sells_executed.avg_20d_pct))}>
                      {fmtPctValue(r.sells_executed.avg_20d_pct, 1)}
                    </span>
                    （{r.sells_executed.samples} 条）。
                  </>
                )}
              </p>
              <p className="text-xs text-zinc-500">
                从操作日开盘算起（建议出来后最早能成交的价格）；同一只股票连续几天的同一个建议只算第一天。
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
        <p className="mb-2 text-xs text-zinc-500">
          手动记错的买入、卖出、转入、转出可以点「撤销」：这笔流水会标成已撤销，现金和持仓按剩下的流水重新算一遍，已实现盈亏也跟着变。按建议执行、分红送转、校准的流水不能单独撤销，用「校准持仓和现金」改。
          分红按税前金额记账，没有扣红利税（税在卖出时由券商代扣），所以这里算出的收益会比券商实际看到的高一点。
        </p>
        <QueryView query={trades}>
          {(list) => (
            <>
              <Table>
                <thead>
                  <tr>
                    <th>日期</th>
                    <th>类型</th>
                    <th>股票</th>
                    <th>股数</th>
                    <th>价格</th>
                    <th>现金变化</th>
                    <th>已实现盈亏</th>
                    <th>说明</th>
                    <th>操作</th>
                  </tr>
                </thead>
                <tbody>
                  {list.map((t) => (
                    <tr key={t.trade_id} className={t.reversed_at ? "opacity-50" : undefined}>
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
                      <td className={clsx("num", upDownClass(t.realized_pnl))}>
                        {t.realized_pnl === null || t.realized_pnl === undefined ? "" : yuan(t.realized_pnl, 2)}
                      </td>
                      <td className="max-w-80 text-xs text-zinc-500" title={fmtDateTime(t.created_at)}>
                        {t.note}
                        {t.reversed_at && <span className="ml-1 text-red-600">已撤销</span>}
                      </td>
                      <td className="text-xs">
                        {canUndo(t) && (
                          <button
                            type="button"
                            className="text-red-600 hover:underline disabled:opacity-40"
                            disabled={undo.isPending}
                            onClick={() => {
                              const what = t.thscode ? `${t.name ?? t.thscode} 的持仓和成本` : "现金";
                              const ok = window.confirm(
                                `撤销这笔${TRADE_KINDS[t.kind]}？\n会把这笔流水标成已撤销，${what}按剩下的流水重新算一遍，已实现盈亏也会跟着变。`,
                              );
                              if (ok) {
                                setUndoError(null);
                                undo.mutate({ accountId: id, tradeId: t.trade_id });
                              }
                            }}
                          >
                            撤销
                          </button>
                        )}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </Table>
              {undoError && <p className="mt-2 text-sm text-red-600">{undoError}</p>}
            </>
          )}
        </QueryView>
      </Card>
    </div>
  );
}

const EQUITY_RANGES = [
  { days: 90, label: "近 3 个月" },
  { days: 180, label: "近 6 个月" },
  { days: 365, label: "近 1 年" },
  { days: 730, label: "近 2 年" },
] as const;

/** 每日净值曲线：每个交易日收盘后的总资产，以及剔除转入转出后的区间收益率 */
function EquityCard({ id }: { id: number }) {
  const [days, setDays] = useState(180);
  const equity = useGetEquity(id, { days });
  return (
    <Card
      title="账户净值（每天收盘后的总资产）"
      extra={
        <div className="flex flex-wrap gap-2">
          {EQUITY_RANGES.map((r) => (
            <button
              key={r.days}
              type="button"
              className={buttonClass(days === r.days ? "primary" : "ghost")}
              onClick={() => setDays(r.days)}
            >
              {r.label}
            </button>
          ))}
        </div>
      }
    >
      <QueryView query={equity}>
        {(c) =>
          c.points.length === 0 ? (
            <Empty>还没有净值数据。账户里有现金或持仓、并且日线同步到最新交易日之后，这里会按每个交易日画出来。</Empty>
          ) : (
            <>
              <p className="mb-2 text-sm">
                区间收益率{" "}
                <span className={clsx("num", upDownClass(c.return_pct))}>{fmtPctValue(c.return_pct, 1)}</span>
                <span className="ml-1 text-xs text-zinc-500">
                  （{c.since ?? "—"} 开始，共 {c.points.length} 个交易日
                  {Math.abs(c.net_transfer) >= 0.01 ? `，期间转入减转出 ${yuan(c.net_transfer, 2)}` : ""}）
                </span>
              </p>
              <AccountEquityChart points={c.points} />
              <p className="mt-2 text-xs text-zinc-500">
                收益率已经剔除转入、转出：（期末利润 − 期初利润）÷ 期初总资产，利润 = 总资产 −
                累计转入（减转出）；钱在区间中间进出时是简单估算。每天的总资产按当天收盘价算，停牌用之前最近的收盘价，不取盘中价格。每次打开都按流水和收盘价重新算，补记或撤销流水后历史也会跟着改。
              </p>
            </>
          )
        }
      </QueryView>
    </Card>
  );
}

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
  const [range, setRange] = useState<ChartRange>({ range: "ytd", from: "", to: "" });
  // 规则回测要算全市场几年，只有点按钮才开始；改区间不会自动重算，算的时候可以取消
  const backtestParams = rangeQuery(range);
  const backtestParamsKey = JSON.stringify(backtestParams);
  const [backtestRun, setBacktestRun] = useState<{ params: { start?: string; end?: string }; key: string } | null>(
    null,
  );
  const backtest = useQuery({
    queryKey: ["account-backtest", id, backtestRun?.key ?? ""],
    queryFn: ({ signal }) => accountBacktest(id, backtestRun?.params, { signal }),
    enabled: backtestRun !== null,
    staleTime: 5 * 60_000,
    retry: false,
    refetchOnWindowFocus: false,
  });
  const backtestChanged = backtestRun !== null && backtestRun.key !== backtestParamsKey;
  const backtestRunning = backtestRun !== null && backtest.isFetching;
  const backtestSeconds = useElapsedSeconds(backtestRunning);
  const startBacktest = () => {
    if (backtestRun && !backtestChanged) {
      void backtest.refetch();
      return;
    }
    setBacktestRun({ params: backtestParams, key: backtestParamsKey });
  };
  const cancelBacktest = () => {
    if (backtestRun) void qc.cancelQueries({ queryKey: ["account-backtest", id, backtestRun.key] });
    setBacktestRun(null);
  };
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
          按保存后的规则，假设这段时间每天都照着建议做，账户会怎样。点快捷区间会把起止日期填进框里，也可以直接改日期（最长
          5
          年，结束日期最晚到最新交易日）。勾选的策略都会参与，按同时命中的策略数和成交额排序。全市场计算，通常要几十秒到一两分钟，算的时候可以取消；改日期不会自动重算。
        </p>
        <div className="mt-2 flex flex-wrap items-center gap-3">
          <RangePicker value={range} options={BACKTEST_RANGES} onChange={(p) => setRange((r) => ({ ...r, ...p }))} />
          {backtestRunning ? (
            <TaskRunning
              text="正在按账户规则逐只计算全市场的日线"
              seconds={backtestSeconds}
              onCancel={cancelBacktest}
            />
          ) : (
            <>
              <button type="button" className={buttonClass("ghost")} onClick={startBacktest}>
                {backtestRun && backtestChanged ? "按新日期重新计算" : backtestRun ? "重新回测" : "开始回测"}
              </button>
              <span className="text-xs text-zinc-500" title="依据：文档实测账户规则回测本地约几十秒到一两分钟">
                全市场计算，通常几十秒到一两分钟
              </span>
            </>
          )}
        </div>
        {backtestChanged && !backtestRunning && (
          <p className="mt-2 text-xs text-amber-600">
            日期已改，下面显示的还是上一次按旧日期算的结果；点「按新日期重新计算」。
          </p>
        )}
        {backtest.data && (
          <div className="mt-2">
            <BacktestLine b={backtest.data} />
            <BacktestTrades trades={backtest.data.trades ?? []} />
          </div>
        )}
        {!!backtest.error && !backtestRunning && (
          <p className="mt-2 text-sm text-red-600">{errorText(backtest.error)}</p>
        )}
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
