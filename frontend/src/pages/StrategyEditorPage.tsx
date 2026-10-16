import { useQueryClient } from "@tanstack/react-query";
import { clsx } from "clsx";
import { type ReactNode, useEffect, useState } from "react";
import { Link, useNavigate, useParams } from "react-router";
import type {
  Catalog,
  Condition,
  Definition,
  ExitRule,
  IndicatorRef,
  Op,
  Operand,
  Sort,
  StrategyInput,
  Universe,
} from "../api/generated/model";
import {
  useCatalog,
  useCreateStrategy,
  useGetStrategy,
  usePreview,
  useUpdateStrategy,
} from "../api/generated/strategy/strategy";
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
import { BOARDS } from "../lib/labels";
import {
  defaults,
  fillName,
  findIndicator,
  findSignal,
  groupBy,
  indicatorName,
  isIndicatorOperand,
  newSignalCondition,
  signalName,
  UNIT_SUFFIX,
} from "../lib/strategy";
import { ExplainedView } from "./StrategyPage";

/** 编辑中的策略：定义的每个部分都补齐，控件不用处理缺省 */
type Draft = Omit<StrategyInput, "definition"> & {
  definition: Required<Omit<Definition, "exit" | "universe">> & { universe: Universe; exit: Required<ExitRule> };
};

const DEFAULT_EXIT: Required<ExitRule> = { any: [], stop_loss_pct: 8, take_profit_pct: null, max_hold_days: 20 };
const DEFAULT_SORT: Sort = { by: { id: "amount" }, desc: true };

function toDraft(catalog: Catalog, x: StrategyInput): Draft {
  const d = x.definition;
  return {
    name: x.name,
    description: x.description ?? "",
    screening: x.screening ?? true,
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
  const [s, set] = useSearchState({ from: "" });

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
    : { name: "", description: "", definition: { all: [] } };
  return <Editor key={s.from} catalog={cat} initial={toDraft(cat, initial)} onBack={() => set({ from: "" })} />;
}

function TemplatePicker({ catalog, onPick }: { catalog: Catalog; onPick: (key: string) => void }) {
  return (
    <>
      <PageHeader
        title="新建策略"
        desc="先选一个最接近想法的模板，再改条件；也可以从空白开始。"
        extra={
          <Link to="/screener" className="text-sm text-zinc-500 hover:text-zinc-900 dark:hover:text-zinc-100">
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
  const [draft, setDraft] = useState(initial);
  const def = draft.definition;
  const setDef = (patch: Partial<Draft["definition"]>) =>
    setDraft((d) => ({ ...d, definition: { ...d.definition, ...patch } }));
  const preview = usePreview();
  const done = (id: number) => {
    qc.invalidateQueries();
    nav(`/screener/${id}`);
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
              to={`/screener/${editId}`}
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
        </div>
      </Card>

      <Card title="① 股票池：先排除不想看的股票">
        <UniverseEditor u={def.universe} onChange={(universe) => setDef({ universe })} />
      </Card>

      <Card title="② 条件：股票要满足什么">
        <div className="flex flex-col gap-5">
          <ConditionList
            title="全部满足"
            hint="每一条都要满足"
            items={def.all}
            onChange={(all) => setDef({ all })}
            catalog={catalog}
          />
          <ConditionList
            title="至少满足一个（可选）"
            hint="这一组里满足任意一条就行；不加条件表示不要求"
            items={def.any}
            onChange={(any) => setDef({ any })}
            catalog={catalog}
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
        <details>
          <summary className="cursor-pointer text-sm font-semibold select-none">
            ④ 卖出规则（回测时使用，选股时不用）
          </summary>
          <p className="mt-2 text-xs text-zinc-500">
            回测时，买入后满足下面任意一条就在下一个交易日开盘卖出；一条都不设时，买入后一直持有到回测结束。
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
              <Labeled label="最多持有（交易日）">
                <NumInput
                  value={def.exit.max_hold_days}
                  onChange={(v) => setDef({ exit: { ...def.exit, max_hold_days: v } })}
                  allowEmpty
                  integer
                />
              </Labeled>
            </div>
            <ConditionList
              title="出现这些情况时卖出"
              hint="满足任意一条就卖出"
              items={def.exit.any}
              onChange={(any) => setDef({ exit: { ...def.exit, any } })}
              catalog={catalog}
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

function Labeled({ label, children, className }: { label: string; children: ReactNode; className?: string }) {
  return (
    // biome-ignore lint/a11y/noLabelWithoutControl: 控件通过 children 传入
    <label className={clsx("flex flex-col gap-1 text-xs text-zinc-500", className)}>
      <span>{label}</span>
      {children}
    </label>
  );
}

/** 数字输入：输入合法数字时立即生效；allowEmpty 时清空表示不设置 */
function NumInput({
  value,
  onChange,
  allowEmpty = false,
  integer = false,
  width = "w-24",
}: {
  value: number | null | undefined;
  onChange: (v: number | null) => void;
  allowEmpty?: boolean;
  integer?: boolean;
  width?: string;
}) {
  const [text, setText] = useState(value === null || value === undefined ? "" : String(value));
  // 外部改了值（例如换了指标、参数被重置）时同步显示
  useEffect(() => {
    setText((t) => (t !== "" && Number(t) === value ? t : value === null || value === undefined ? "" : String(value)));
  }, [value]);
  const bad =
    text === "" ? !allowEmpty : !Number.isFinite(Number(text)) || (integer && !Number.isInteger(Number(text)));
  return (
    <input
      className={clsx(inputClass, "num", width, bad && "border-red-400!")}
      inputMode="decimal"
      value={text}
      placeholder={allowEmpty ? "不设置" : undefined}
      onChange={(e) => {
        const t = e.target.value.trim();
        setText(t);
        if (t === "") {
          if (allowEmpty) onChange(null);
        } else if (Number.isFinite(Number(t)) && (!integer || Number.isInteger(Number(t)))) onChange(Number(t));
      }}
    />
  );
}

const SECTOR_DAY_OPTIONS = [1, 3, 5, 10, 20, 60];

function UniverseEditor({ u, onChange }: { u: Universe; onChange: (u: Universe) => void }) {
  const boards = u.boards ?? [];
  const toggleBoard = (b: string) =>
    onChange({ ...u, boards: boards.includes(b) ? boards.filter((x) => x !== b) : [...boards, b] });
  return (
    <div className="flex flex-col gap-4 text-sm">
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
      <div className="flex flex-wrap items-end gap-4">
        <label className="flex items-center gap-1.5 pb-1.5">
          <input
            type="checkbox"
            checked={u.exclude_st ?? true}
            onChange={(e) => onChange({ ...u, exclude_st: e.target.checked })}
          />
          排除 ST 股
        </label>
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
    </div>
  );
}

function ConditionList({
  title,
  hint,
  items,
  onChange,
  catalog,
}: {
  title: string;
  hint: string;
  items: Condition[];
  onChange: (items: Condition[]) => void;
  catalog: Catalog;
}) {
  const first = catalog.signals[0];
  const add = (c: Condition) => onChange([...items, c]);
  return (
    <div>
      <h3 className="text-sm font-medium">{title}</h3>
      <p className="mb-2 text-xs text-zinc-500">{hint}</p>
      <div className="flex flex-col gap-2">
        {items.map((c, i) => (
          <div
            // biome-ignore lint/suspicious/noArrayIndexKey: 条件没有自己的 id，顺序就是身份
            key={i}
            className="flex flex-wrap items-center gap-2 rounded-lg border border-zinc-200 bg-zinc-50/50 p-2 text-sm dark:border-zinc-800 dark:bg-zinc-800/30"
          >
            <ConditionRow
              c={c}
              catalog={catalog}
              onChange={(nc) => onChange(items.map((x, j) => (j === i ? nc : x)))}
            />
            <button
              type="button"
              className="ml-auto rounded px-2 py-1 text-zinc-400 hover:bg-zinc-100 hover:text-red-600 dark:hover:bg-zinc-800"
              title="删除这条"
              aria-label="删除这条"
              onClick={() => onChange(items.filter((_, j) => j !== i))}
            >
              ×
            </button>
          </div>
        ))}
        <div className="flex flex-wrap gap-2">
          {first && (
            <button type="button" className={buttonClass("ghost")} onClick={() => add(newSignalCondition(first))}>
              + 信号
            </button>
          )}
          <button
            type="button"
            className={buttonClass("ghost")}
            title="比较一个指标和一个数值或另一个指标，例如「RSI（14）< 30」「收盘价 > 60 日均线」"
            onClick={() => add({ kind: "compare", left: { id: "rsi", n: 14 }, op: "<", right: { value: 30 } })}
          >
            + 数值比较
          </button>
        </div>
      </div>
    </div>
  );
}

const WITHIN = [1, 2, 3, 5, 10, 20];

function ConditionRow({ c, catalog, onChange }: { c: Condition; catalog: Catalog; onChange: (c: Condition) => void }) {
  if (c.kind === "signal") {
    const info = findSignal(catalog, c.signal);
    const within = c.within_days ?? 1;
    return (
      <>
        <select
          className={inputClass}
          value={c.signal}
          onChange={(e) => {
            const next = findSignal(catalog, e.target.value);
            if (next) onChange({ ...newSignalCondition(next), within_days: next.kind === "state" ? 1 : within });
          }}
        >
          {groupBy(catalog.signals).map(([g, list]) => (
            <optgroup key={g} label={g}>
              {list.map((s) => (
                <option key={s.id} value={s.id}>
                  {signalName(s)}
                </option>
              ))}
            </optgroup>
          ))}
        </select>
        {info?.params.map((p) => (
          <span key={p.key} className="flex items-center gap-1 text-xs text-zinc-500">
            {p.label}
            <NumInput
              value={c.params?.[p.key] ?? p.default}
              integer={p.integer}
              width="w-16"
              onChange={(v) => v !== null && onChange({ ...c, params: { ...c.params, [p.key]: v } })}
            />
          </span>
        ))}
        {info?.kind === "state" ? (
          <span className="text-xs text-zinc-500">当天处于这个状态</span>
        ) : (
          <select
            className={inputClass}
            value={within}
            onChange={(e) => onChange({ ...c, within_days: Number(e.target.value) })}
          >
            {[...new Set([...WITHIN, within])]
              .sort((a, b) => a - b)
              .map((d) => (
                <option key={d} value={d}>
                  {d === 1 ? "当天出现" : `最近 ${d} 个交易日内出现过`}
                </option>
              ))}
          </select>
        )}
        {info && (
          <span className="w-full text-xs text-zinc-400 sm:w-auto">
            {fillName(info.description, { ...defaults(info), ...c.params })}
          </span>
        )}
      </>
    );
  }
  const right = c.right;
  const unit = findIndicator(catalog, c.left.id)?.unit ?? "";
  return (
    <>
      <IndicatorPicker catalog={catalog} value={c.left} onChange={(left) => onChange({ ...c, left })} />
      <select className={inputClass} value={c.op} onChange={(e) => onChange({ ...c, op: e.target.value as Op })}>
        <option value=">">大于</option>
        <option value=">=">不低于</option>
        <option value="<">小于</option>
        <option value="<=">不高于</option>
      </select>
      <select
        className={inputClass}
        value={isIndicatorOperand(right) ? "indicator" : "value"}
        onChange={(e) =>
          onChange({ ...c, right: (e.target.value === "value" ? { value: 0 } : { id: "ma", n: 20 }) as Operand })
        }
      >
        <option value="value">数值</option>
        <option value="indicator">另一个指标</option>
      </select>
      {isIndicatorOperand(right) ? (
        <IndicatorPicker catalog={catalog} value={right} onChange={(r) => onChange({ ...c, right: r })} />
      ) : (
        <span className="flex items-center gap-1">
          <NumInput value={right.value} onChange={(v) => v !== null && onChange({ ...c, right: { value: v } })} />
          <span className="text-xs text-zinc-500">{UNIT_SUFFIX[unit]}</span>
        </span>
      )}
    </>
  );
}

function IndicatorPicker({
  catalog,
  value,
  onChange,
}: {
  catalog: Catalog;
  value: IndicatorRef;
  onChange: (v: IndicatorRef) => void;
}) {
  const info = findIndicator(catalog, value.id);
  return (
    <span className="flex items-center gap-1" title={info?.description}>
      <select
        className={inputClass}
        value={value.id}
        onChange={(e) => {
          const next = findIndicator(catalog, e.target.value);
          if (next) onChange({ id: next.id, n: next.n_default ?? null });
        }}
      >
        {groupBy(catalog.indicators).map(([g, list]) => (
          <optgroup key={g} label={g}>
            {list.map((x) => (
              <option key={x.id} value={x.id}>
                {indicatorName(x, x.id === value.id ? value.n : undefined)}
              </option>
            ))}
          </optgroup>
        ))}
      </select>
      {info && info.n_default !== null && info.n_default !== undefined && (
        <span className="flex items-center gap-1 text-xs text-zinc-500">
          周期
          <NumInput
            value={value.n ?? info.n_default}
            integer
            width="w-16"
            onChange={(n) => n !== null && onChange({ ...value, n })}
          />
        </span>
      )}
    </span>
  );
}
