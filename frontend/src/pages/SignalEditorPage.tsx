import { useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { Link, useNavigate, useParams } from "react-router";
import type { Catalog, Condition, SignalDef, SignalEntry, SignalInput } from "../api/generated/model";
import { useCreateSignal, useGetSignal, useUpdateSignal } from "../api/generated/signal/signal";
import { useCatalog } from "../api/generated/strategy/strategy";
import { ConditionList, Labeled } from "../components/conditions";
import { SignalStatsCard, SignalTodayCard } from "../components/SignalStats";
import { StrategyNav } from "../components/StrategyNav";
import { buttonClass, Card, ErrorState, errorText, inputClass, PageHeader, Segmented, Spinner } from "../components/ui";
import { useSearchState } from "../hooks/useSearchState";
import { describeCondition, findSignal } from "../lib/strategy";

type Draft = Omit<SignalInput, "definition"> & { definition: Required<SignalDef> };

/** 新建或修改自定义信号。/signals/new?from=<信号> 复制一个已有信号，?cond=<条件 JSON> 从指标页的常用写法开始 */
export function SignalEditorPage() {
  const { key } = useParams();
  const [s] = useSearchState({ from: "", cond: "" });
  const catalog = useCatalog({ query: { staleTime: Number.POSITIVE_INFINITY } });
  const source = key ?? s.from;
  const existing = useGetSignal(source, { query: { enabled: source !== "" } });
  if (catalog.isPending || (source && existing.isPending)) return <Spinner />;
  if (!catalog.data) return <ErrorState error={catalog.error} onRetry={() => catalog.refetch()} />;
  if (existing.error) return <ErrorState error={existing.error} onRetry={() => existing.refetch()} />;
  const e = existing.data;
  const initial: Draft = e
    ? {
        name: key ? e.name : `${e.name}（我的）`,
        description: e.description,
        // 中性的内置信号复制后先按偏多，可以在编辑页改
        bullish: e.tone !== "bearish",
        definition: { match_all: e.definition?.match_all ?? true, conditions: e.definition?.conditions ?? [] },
        based_on: key ? e.based_on : e.builtin ? e.key : e.based_on,
      }
    : { name: "", description: "", bullish: true, definition: { match_all: true, conditions: seed(s.cond) } };
  return (
    <Editor
      key={`${key ?? ""}|${s.from}|${s.cond}`}
      catalog={catalog.data}
      initial={initial}
      editing={key && e ? e : undefined}
    />
  );
}

function seed(cond: string): Condition[] {
  if (!cond) return [];
  try {
    return [JSON.parse(cond) as Condition];
  } catch {
    return [];
  }
}

/** 和后端 SignalDef::is_event 一致：含上穿下穿或事件类信号时是事件 */
function isEvent(catalog: Catalog, d: SignalDef): boolean {
  return d.conditions.some((c) =>
    c.kind === "compare"
      ? c.op === "crosses_above" || c.op === "crosses_below"
      : c.kind === "signal" && findSignal(catalog, c.signal)?.kind === "event",
  );
}

function Editor({ catalog, initial, editing }: { catalog: Catalog; initial: Draft; editing?: SignalEntry }) {
  const nav = useNavigate();
  const qc = useQueryClient();
  const [draft, setDraft] = useState(initial);
  // 「试一试」时的定义快照：改了条件要重新点
  const [tried, setTried] = useState<SignalDef | null>(null);
  const def = draft.definition;
  const done = (r: SignalEntry) => {
    qc.invalidateQueries();
    nav(`/signals/${r.key}`);
  };
  const create = useCreateSignal({ mutation: { onSuccess: done } });
  const update = useUpdateSignal({ mutation: { onSuccess: done } });
  const saving = create.isPending || update.isPending;
  const error = create.error ?? update.error;
  const save = () => {
    const data = { ...draft, name: draft.name.trim() };
    if (editing?.signal_id) update.mutate({ signalId: editing.signal_id, data });
    else create.mutate({ data });
  };
  const texts = def.conditions.map((c) => describeCondition(catalog, c, new Map()));
  const stale = tried !== null && JSON.stringify(tried) !== JSON.stringify(def);
  return (
    <>
      <StrategyNav />
      <PageHeader
        title={editing ? `编辑「${editing.name}」` : "新建信号"}
        desc="信号就是起了名字的条件。用指标比较（例如「收盘价上穿 60 日均线」）和内置信号组合出来，保存后可以在策略里用，也能画在 K 线上、看过去表现。"
        extra={
          <Link
            to={editing ? `/signals/${editing.key}` : "/signals"}
            className="text-sm text-zinc-500 hover:text-zinc-900 dark:hover:text-zinc-100"
          >
            ← 返回
          </Link>
        }
      />
      <Card>
        <div className="flex flex-wrap items-end gap-4">
          <Labeled label="名称">
            <input
              className={`${inputClass} w-56`}
              value={draft.name}
              maxLength={30}
              placeholder="例如：放量站上 60 日线"
              onChange={(e) => setDraft({ ...draft, name: e.target.value })}
            />
          </Labeled>
          <Labeled label="一句话说明（可选）" className="min-w-64 flex-1">
            <input
              className={inputClass}
              value={draft.description ?? ""}
              maxLength={200}
              placeholder="它想抓住什么样的时刻"
              onChange={(e) => setDraft({ ...draft, description: e.target.value })}
            />
          </Labeled>
          <div className="flex flex-col gap-1 text-xs text-zinc-500">
            <span>偏多还是偏空</span>
            <Segmented
              value={draft.bullish === false ? "bear" : "bull"}
              options={[
                { value: "bull", label: "▲ 偏多", hint: "看作走强，K 线上用红色向上箭头标出" },
                { value: "bear", label: "▼ 偏空", hint: "看作走弱，K 线上用绿色向下箭头标出" },
              ]}
              onChange={(v) => setDraft({ ...draft, bullish: v === "bull" })}
            />
          </div>
        </div>
      </Card>

      <Card title="条件">
        <div className="mb-3 flex flex-wrap items-center gap-2 text-sm">
          <span className="text-zinc-500">下面的条件</span>
          <Segmented
            value={def.match_all ? "all" : "any"}
            options={[
              { value: "all", label: "全部满足" },
              { value: "any", label: "任一满足" },
            ]}
            onChange={(v) => setDraft({ ...draft, definition: { ...def, match_all: v === "all" } })}
          />
          <span className="text-zinc-500">时，就算出现了这个信号</span>
        </div>
        <ConditionList
          items={def.conditions}
          onChange={(conditions) => setDraft({ ...draft, definition: { ...def, conditions } })}
          catalog={catalog}
          empty="还没有条件。点下面的按钮加一条。"
        />
        <p className="mt-3 text-xs text-zinc-500">
          信号里只能用指标和内置信号，不能再套别的自定义信号；最多 10 条。要更复杂的组合，在策略里把几个信号放在一起。
        </p>
      </Card>

      {def.conditions.length > 0 && (
        <Card title="合起来的意思">
          <p className="text-sm">
            {texts.join(def.match_all ? "，并且" : "，或者")}，就算出现「{draft.name.trim() || "这个信号"}」。
          </p>
          <p className="mt-1 text-xs text-zinc-500">
            {isEvent(catalog, def)
              ? "这是一个事件：只在某一天成立（例如穿过去的那天），适合当买点、卖点。"
              : "这是一个状态：可能连续很多天都成立，适合当过滤条件；想要只在开始那天成立，可以用「上穿」「下穿」。"}
          </p>
        </Card>
      )}

      <div className="flex flex-wrap items-center gap-3">
        <button
          type="button"
          className={buttonClass("ghost")}
          disabled={def.conditions.length === 0}
          title="不保存，先看看今天哪些股票满足、过去出现后表现如何"
          onClick={() => setTried(structuredClone(def))}
        >
          {tried && !stale ? "已试算" : "试一试"}
        </button>
        <button
          type="button"
          className={buttonClass()}
          disabled={saving || draft.name.trim() === "" || def.conditions.length === 0}
          onClick={save}
        >
          {saving ? "保存中…" : "保存"}
        </button>
        {draft.name.trim() === "" && <span className="text-xs text-zinc-500">先填名称</span>}
        {stale && <span className="text-xs text-amber-600">条件改过了，再点一次「试一试」看新的结果</span>}
        {!!error && <span className="text-sm text-red-600">{errorText(error)}</span>}
      </div>

      {tried && (
        <>
          <SignalTodayCard target={{ draft: tried }} />
          <SignalStatsCard target={{ draft: tried }} />
        </>
      )}
    </>
  );
}
