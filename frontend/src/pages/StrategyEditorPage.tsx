import { useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { Link, useNavigate, useParams } from "react-router";
import type {
  Catalog,
  Condition,
  Definition,
  ExitRule,
  SignalEntry,
  Sort,
  StrategyInput,
  Universe,
} from "../api/generated/model";
import { useListSignals } from "../api/generated/signal/signal";
import {
  useCatalog,
  useCreateStrategy,
  useGetStrategy,
  usePreview,
  useUpdateStrategy,
} from "../api/generated/strategy/strategy";
import { ConditionList, IndicatorPicker, Labeled, NumInput } from "../components/conditions";
import {
  buttonClass,
  Card,
  Empty,
  ErrorState,
  errorText,
  inputClass,
  PageHeader,
  Spinner,
  Table,
} from "../components/ui";
import { useSearchState } from "../hooks/useSearchState";
import { fmtBig, fmtInt, fmtPctValue, fmtPrice, upDownClass } from "../lib/format";
import { ASSET_TYPES, BOARDS } from "../lib/labels";
import { findSignal, newSignalCondition } from "../lib/strategy";
import { ExplainedView } from "./StrategyPage";

/** 编辑中的策略：定义的每个部分都补齐，控件不用处理缺省 */
type Draft = Omit<StrategyInput, "definition"> & {
  definition: Required<Omit<Definition, "exit" | "universe">> & {
    universe: Universe;
    exit: Required<ExitRule>;
  };
};

const DEFAULT_EXIT: Required<ExitRule> = {
  any: [],
  stop_loss_pct: 8,
  take_profit_pct: null,
  trailing_stop_atr: null,
  max_hold_days: 20,
};
const DEFAULT_SORT: Sort = { by: { id: "amount" }, desc: true };

function toDraft(catalog: Catalog, x: StrategyInput): Draft {
  const d = x.definition;
  return {
    name: x.name,
    description: x.description ?? "",
    screening: x.screening ?? true,
    show_on_today: x.show_on_today ?? true,
    template: x.template ?? null,
    definition: {
      universe: { ...catalog.default_universe, ...d.universe },
      all: d.all ?? [],
      any: d.any ?? [],
      exit: { ...DEFAULT_EXIT, ...d.exit, any: d.exit?.any ?? [] },
      sort: d.sort ?? DEFAULT_SORT,
      limit: d.limit ?? 50,
    },
  };
}

export function StrategyEditorPage() {
  const params = useParams();
  const editId = params.id === undefined ? undefined : Number(params.id);
  const catalog = useCatalog({ query: { staleTime: Number.POSITIVE_INFINITY } });
  const existing = useGetStrategy(editId ?? 0, { query: { enabled: editId !== undefined } });
  const [s, set] = useSearchState({ from: "", signal: "" });

  if (catalog.isPending || (editId !== undefined && existing.isPending)) return <Spinner />;
  const cat = catalog.data;
  if (!cat) return <ErrorState error={catalog.error} onRetry={() => catalog.refetch()} />;
  if (existing.error) return <ErrorState error={existing.error} onRetry={() => existing.refetch()} />;

  if (editId !== undefined && existing.data) {
    return <Editor key={editId} catalog={cat} initial={toDraft(cat, existing.data)} editId={editId} />;
  }
  if (!s.from) return <TemplatePicker catalog={cat} onPick={(from) => set({ from })} />;
  const t = cat.templates.find((x) => x.key === s.from);
  const initial: StrategyInput = t
    ? { name: t.name, description: t.description, definition: t.definition, template: t.key }
    : { name: "", description: "", definition: { all: seedCondition(cat, s.signal) } };
  return <Editor key={s.from} catalog={cat} initial={toDraft(cat, initial)} onBack={() => set({ from: "" })} />;
}

/** 从信号页的「用它新建策略」进来时，先放好这一个信号条件；my-<编号> 是自定义信号 */
function seedCondition(catalog: Catalog, signal: string): Condition[] {
  if (signal.startsWith("my-")) return [{ kind: "my_signal", signal_id: Number(signal.slice(3)) }];
  const info = findSignal(catalog, signal);
  return info ? [newSignalCondition(info)] : [];
}

function TemplatePicker({ catalog, onPick }: { catalog: Catalog; onPick: (key: string) => void }) {
  return (
    <>
      <PageHeader
        title="新建策略"
        desc="先选一个最接近想法的模板，再改条件；也可以从空白开始。"
        extra={
          <Link to="/strategies" className="text-sm text-zinc-500 hover:text-zinc-900 dark:hover:text-zinc-100">
            ← 返回
          </Link>
        }
      />
      <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
        {catalog.templates.map((t) => (
          <button
            key={t.key}
            type="button"
            onClick={() => onPick(t.key)}
            className="flex flex-col gap-1 rounded-xl border border-zinc-200 bg-white p-4 text-left transition hover:border-accent/60 dark:border-zinc-800 dark:bg-zinc-900"
          >
            <span className="font-medium">{t.name}</span>
            <span className="text-sm text-zinc-500">{t.description}</span>
          </button>
        ))}
        <button
          type="button"
          onClick={() => onPick("blank")}
          className="flex flex-col gap-1 rounded-xl border border-dashed border-zinc-300 p-4 text-left transition hover:border-accent/60 dark:border-zinc-700"
        >
          <span className="font-medium">从空白开始</span>
          <span className="text-sm text-zinc-500">自己组合信号和数值条件</span>
        </button>
      </div>
    </>
  );
}

function Editor({
  catalog,
  initial,
  editId,
  onBack,
}: {
  catalog: Catalog;
  initial: Draft;
  editId?: number;
  onBack?: () => void;
}) {
  const nav = useNavigate();
  const qc = useQueryClient();
  const signals = useListSignals();
  const mine: SignalEntry[] = (signals.data ?? []).filter((x) => !x.builtin);
  const [draft, setDraft] = useState(initial);
  const def = draft.definition;
  const setDef = (patch: Partial<Draft["definition"]>) =>
    setDraft((d) => ({ ...d, definition: { ...d.definition, ...patch } }));
  const preview = usePreview();
  const done = (id: number) => {
    qc.invalidateQueries();
    nav(`/strategies/${id}`);
  };
  const create = useCreateStrategy({ mutation: { onSuccess: (r) => done(r.strategy_id) } });
  const update = useUpdateStrategy({ mutation: { onSuccess: (r) => done(r.strategy_id) } });
  const saving = create.isPending || update.isPending;
  const saveError = create.error ?? update.error;
  const save = () => {
    const data = { ...draft, name: draft.name.trim() };
    if (editId === undefined) create.mutate({ data });
    else update.mutate({ strategyId: editId, data });
  };

  return (
    <>
      <PageHeader
        title={editId === undefined ? "新建策略" : `编辑「${initial.name}」`}
        extra={
          onBack ? (
            <button
              type="button"
              onClick={onBack}
              className="text-sm text-zinc-500 hover:text-zinc-900 dark:hover:text-zinc-100"
            >
              ← 换一个模板
            </button>
          ) : (
            <Link
              to={`/strategies/${editId}`}
              className="text-sm text-zinc-500 hover:text-zinc-900 dark:hover:text-zinc-100"
            >
              ← 返回
            </Link>
          )
        }
      />

      <Card>
        <div className="flex flex-wrap items-end gap-4">
          <Labeled label="名称">
            <input
              className={`${inputClass} w-56`}
              value={draft.name}
              maxLength={40}
              placeholder="例如：放量突破"
              onChange={(e) => setDraft({ ...draft, name: e.target.value })}
            />
          </Labeled>
          <Labeled label="一句话说明（可选）" className="min-w-64 flex-1">
            <input
              className={inputClass}
              value={draft.description ?? ""}
              maxLength={200}
              placeholder="它在找什么样的股票"
              onChange={(e) => setDraft({ ...draft, description: e.target.value })}
            />
          </Labeled>
          <label className="flex items-center gap-2 pb-1.5 text-sm">
            <input
              type="checkbox"
              checked={draft.screening ?? true}
              onChange={(e) => setDraft({ ...draft, screening: e.target.checked })}
            />
            每天收盘后自动巡检
          </label>
          <label
            className="flex items-center gap-2 pb-1.5 text-sm"
            title="只是试试的策略可以不勾，它照常巡检，但不出现在首页"
          >
            <input
              type="checkbox"
              checked={draft.show_on_today ?? true}
              onChange={(e) => setDraft({ ...draft, show_on_today: e.target.checked })}
            />
            在首页展示
          </label>
        </div>
      </Card>

      <Card title="① 股票池：先排除不想看的股票">
        <UniverseEditor u={def.universe} onChange={(universe) => setDef({ universe })} />
      </Card>

      <Card title="② 买入条件：满足时就是买点">
        <p className="-mt-1 mb-4 text-xs text-zinc-500">
          收盘后判断，满足的股票出现在「最新买点」，下一个交易日开盘买入。已经持有的股票再次满足不算新的买点。
        </p>
        <div className="flex flex-col gap-5">
          <ConditionList
            title="全部满足"
            hint="每一条都要满足"
            items={def.all}
            onChange={(all) => setDef({ all })}
            catalog={catalog}
            mine={mine}
          />
          <ConditionList
            title="至少满足一个（可选）"
            hint="这一组里满足任意一条就行；不加条件表示不要求"
            items={def.any}
            onChange={(any) => setDef({ any })}
            catalog={catalog}
            mine={mine}
          />
        </div>
      </Card>

      <Card title="③ 排序：符合的股票太多时先看哪些">
        <div className="flex flex-wrap items-end gap-4 text-sm">
          <Labeled label="按">
            <IndicatorPicker
              catalog={catalog}
              value={def.sort.by}
              onChange={(by) => setDef({ sort: { ...def.sort, by } })}
            />
          </Labeled>
          <Labeled label="顺序">
            <select
              className={inputClass}
              value={def.sort.desc ? "desc" : "asc"}
              onChange={(e) => setDef({ sort: { ...def.sort, desc: e.target.value === "desc" } })}
            >
              <option value="desc">从大到小</option>
              <option value="asc">从小到大</option>
            </select>
          </Labeled>
          <Labeled label="最多保留（只）">
            <NumInput value={def.limit} onChange={(v) => setDef({ limit: v ?? 50 })} integer width="w-20" />
          </Labeled>
        </div>
      </Card>

      <Card>
        <details open>
          <summary className="cursor-pointer text-sm font-semibold select-none">④ 卖出规则：买入后什么时候卖</summary>
          <p className="mt-2 text-xs text-zinc-500">
            买入后满足下面任意一条，就是卖点，下一个交易日开盘卖出。每日巡检按它算出「最新卖点」和「持有中」，回测也按它卖出；一条都不设时，买入后一直持有。
          </p>
          <div className="mt-4 flex flex-col gap-5">
            <div className="flex flex-wrap items-end gap-4">
              <Labeled label="亏损达到（%）时止损">
                <NumInput
                  value={def.exit.stop_loss_pct}
                  onChange={(v) => setDef({ exit: { ...def.exit, stop_loss_pct: v } })}
                  allowEmpty
                />
              </Labeled>
              <Labeled label="盈利达到（%）时止盈">
                <NumInput
                  value={def.exit.take_profit_pct}
                  onChange={(v) => setDef({ exit: { ...def.exit, take_profit_pct: v } })}
                  allowEmpty
                />
              </Labeled>
              <Labeled label="回撤（倍 ATR）时止损">
                <NumInput
                  value={def.exit.trailing_stop_atr}
                  onChange={(v) => setDef({ exit: { ...def.exit, trailing_stop_atr: v } })}
                  allowEmpty
                />
              </Labeled>
              <Labeled label="最多持有（交易日）">
                <NumInput
                  value={def.exit.max_hold_days}
                  onChange={(v) => setDef({ exit: { ...def.exit, max_hold_days: v } })}
                  allowEmpty
                  integer
                />
              </Labeled>
            </div>
            <p className="text-xs text-zinc-500">
              回撤止损用 ATR(14) 的吊灯线：买入次日起，收盘价跌破「持有期最高价 − N 倍
              ATR(14)」时卖出，适合让趋势单多拿一段；和上面的固定百分比止损可以同时设，先触发哪条就用哪条。
            </p>
            <ConditionList
              title="出现这些情况时卖出"
              hint="满足任意一条就卖出"
              items={def.exit.any}
              onChange={(any) => setDef({ exit: { ...def.exit, any } })}
              catalog={catalog}
              mine={mine}
            />
          </div>
        </details>
      </Card>

      <div className="flex flex-wrap items-center gap-3">
        <button
          type="button"
          className={buttonClass("ghost")}
          disabled={preview.isPending}
          onClick={() => preview.mutate({ data: { definition: def } })}
          title="用最新交易日的数据试一下，看看会选出哪些股票；不会保存"
        >
          {preview.isPending ? "试算中…" : "预览结果"}
        </button>
        <button type="button" className={buttonClass()} disabled={saving || draft.name.trim() === ""} onClick={save}>
          {saving ? "保存中…" : "保存"}
        </button>
        {draft.name.trim() === "" && <span className="text-xs text-zinc-500">先填名称</span>}
        {saveError && <span className="text-sm text-red-600">{errorText(saveError)}</span>}
      </div>

      {preview.error && (
        <Card>
          <p className="text-sm text-red-600">{errorText(preview.error)}</p>
        </Card>
      )}
      {preview.data && (
        <Card title={`预览：${preview.data.trade_date} 有 ${fmtInt(preview.data.matched)} 只符合`}>
          <p className="mb-4 text-xs text-zinc-500">
            预览只看这一天是否满足买入条件：不考虑是否已经持有，也不判断下一个交易日开盘能不能买到。结论以每日巡检的买卖点为准。
          </p>
          <div className="mb-4 rounded-lg bg-zinc-50 p-3 dark:bg-zinc-800/50">
            <ExplainedView e={preview.data.explained} />
          </div>
          {preview.data.hits.length === 0 ? (
            <Empty>这一天没有符合条件的股票，可以放宽条件试试</Empty>
          ) : (
            <Table>
              <thead>
                <tr>
                  <th>股票</th>
                  <th className="text-right!">收盘</th>
                  <th className="text-right!">涨跌幅</th>
                  <th className="text-right!">成交额</th>
                  <th>符合的条件</th>
                </tr>
              </thead>
              <tbody>
                {preview.data.hits.slice(0, 20).map((h) => (
                  <tr key={h.thscode}>
                    <td>
                      <Link to={`/securities/${h.thscode}`} className="hover:underline">
                        {h.name} <span className="num font-mono text-xs text-zinc-500">{h.thscode}</span>
                      </Link>
                    </td>
                    <td className="num text-right">{fmtPrice(h.close)}</td>
                    <td className={`num text-right ${upDownClass(h.change_pct)}`}>{fmtPctValue(h.change_pct)}</td>
                    <td className="num text-right">{fmtBig(h.amount)}</td>
                    <td className="text-xs whitespace-normal text-zinc-600 dark:text-zinc-400">
                      {h.reasons.join("；")}
                    </td>
                  </tr>
                ))}
              </tbody>
            </Table>
          )}
          {preview.data.hits.length > 20 && (
            <p className="mt-2 text-xs text-zinc-500">只显示排序靠前的 20 只，保存后巡检会保留前 {def.limit} 只。</p>
          )}
        </Card>
      )}
    </>
  );
}

const SECTOR_DAY_OPTIONS = [1, 3, 5, 10, 20, 60];

function UniverseEditor({ u, onChange }: { u: Universe; onChange: (u: Universe) => void }) {
  const assetTypes = u.asset_types?.length ? u.asset_types : ["stock"];
  const hasStock = assetTypes.includes("stock");
  const toggleAsset = (asset: string) =>
    onChange({
      ...u,
      asset_types: assetTypes.includes(asset)
        ? assetTypes.filter((x) => x !== asset).length > 0
          ? assetTypes.filter((x) => x !== asset)
          : assetTypes
        : [...assetTypes, asset],
    });
  const boards = u.boards ?? [];
  const toggleBoard = (b: string) =>
    onChange({ ...u, boards: boards.includes(b) ? boards.filter((x) => x !== b) : [...boards, b] });
  return (
    <div className="flex flex-col gap-4 text-sm">
      <div className="flex flex-wrap items-center gap-x-4 gap-y-2">
        <span className="text-xs text-zinc-500">资产</span>
        {Object.entries(ASSET_TYPES).map(([k, v]) => (
          <label key={k} className="flex items-center gap-1.5">
            <input type="checkbox" checked={assetTypes.includes(k)} onChange={() => toggleAsset(k)} />
            {v}
          </label>
        ))}
      </div>
      {hasStock && (
        <div className="flex flex-wrap items-center gap-x-4 gap-y-2">
          <span className="text-xs text-zinc-500">板块</span>
          {Object.entries(BOARDS).map(([k, v]) => (
            <label key={k} className="flex items-center gap-1.5">
              <input type="checkbox" checked={boards.includes(k)} onChange={() => toggleBoard(k)} />
              {v}
            </label>
          ))}
          <span className="text-xs text-zinc-400">{boards.length === 0 ? "都不勾表示全部板块" : ""}</span>
        </div>
      )}
      <div className="flex flex-wrap items-end gap-4">
        {hasStock && (
          <label className="flex items-center gap-1.5 pb-1.5">
            <input
              type="checkbox"
              checked={u.exclude_st ?? true}
              onChange={(e) => onChange({ ...u, exclude_st: e.target.checked })}
            />
            排除 ST 股
          </label>
        )}
        <Labeled label="最低股价（元）">
          <NumInput value={u.min_price} onChange={(min_price) => onChange({ ...u, min_price })} allowEmpty />
        </Labeled>
        <Labeled label="最低成交额（万元）">
          <NumInput
            value={u.min_amount === null || u.min_amount === undefined ? null : u.min_amount / 1e4}
            onChange={(v) => onChange({ ...u, min_amount: v === null ? null : v * 1e4 })}
            allowEmpty
          />
        </Labeled>
        <Labeled label="上市满（交易日）">
          <NumInput
            value={u.min_listed_days}
            onChange={(min_listed_days) => onChange({ ...u, min_listed_days })}
            allowEmpty
            integer
          />
        </Labeled>
      </div>
      {hasStock && (
        <div className="flex flex-wrap items-center gap-2">
          <label className="flex items-center gap-1.5">
            <input
              type="checkbox"
              checked={!!u.sector}
              onChange={(e) =>
                onChange({ ...u, sector: e.target.checked ? { category: "industry", days: 5, top_pct: 20 } : null })
              }
            />
            只看强势板块的股票
          </label>
          {u.sector && (
            <>
              <span className="text-zinc-500">：按</span>
              <select
                className={inputClass}
                value={u.sector.category}
                onChange={(e) => u.sector && onChange({ ...u, sector: { ...u.sector, category: e.target.value } })}
              >
                <option value="industry">同花顺行业</option>
                <option value="cn_concept">同花顺概念</option>
              </select>
              <select
                className={inputClass}
                value={u.sector.days}
                onChange={(e) => u.sector && onChange({ ...u, sector: { ...u.sector, days: Number(e.target.value) } })}
              >
                {SECTOR_DAY_OPTIONS.map((d) => (
                  <option key={d} value={d}>
                    {d === 1 ? "当天" : `近 ${d} 个交易日`}
                  </option>
                ))}
              </select>
              <span className="text-zinc-500">的涨幅排名，前</span>
              <NumInput
                value={u.sector.top_pct}
                onChange={(v) => u.sector && v !== null && onChange({ ...u, sector: { ...u.sector, top_pct: v } })}
                width="w-16"
              />
              <span className="text-zinc-500">%</span>
            </>
          )}
        </div>
      )}
    </div>
  );
}
