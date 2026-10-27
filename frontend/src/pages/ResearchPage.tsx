/** 因子研究：先检验因子有没有预测力（信息系数 RankIC、分组收益、挑选偏差），再用多个因子合成打分做组合回测。
 * 规则见 docs/domains/factor_research.md */
import { keepPreviousData, useQueryClient } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import { Link } from "react-router";
import type {
  FactorId,
  FactorPortfolioParams,
  FactorPortfolioResult,
  FactorTestParams,
  FactorTestResult,
  FactorTestStats,
  FactorTrade,
  FactorTradeSide,
} from "../api/generated/model";
import {
  getFactorPortfolioQueryKey,
  getFactorTestQueryKey,
  getListPresetsQueryKey,
  useFactorPortfolio,
  useFactorTest,
  useSavePreset,
} from "../api/generated/research/research";
import { EquityChart } from "../components/EquityChart";
import { type Line, MultiLineChart, quantileColors } from "../components/FactorCharts";
import { FactorPicker } from "../components/FactorPicker";
import { TaskRunning, useElapsedSeconds } from "../components/longTask";
import { RangePicker, rangeQuery } from "../components/RangePicker";
import { CostFields, costParams, LogToggle, NumberField, RulesNote } from "../components/researchFields";
import {
  Badge,
  buttonClass,
  Card,
  Empty,
  ErrorState,
  errorText,
  inputClass,
  PageHeader,
  Segmented,
  Spinner,
  Stat,
  Table,
  Term,
} from "../components/ui";
import { useSearchState } from "../hooks/useSearchState";
import {
  BUFFER_HELP,
  DEFAULT_FACTORS,
  type FactorWeight,
  fmtNum,
  fmtProb,
  formatFactors,
  parseFactors,
  TOP_N_HELP,
  verdictTone,
} from "../lib/factors";
import { fmtBig, fmtInt, fmtPercent, fmtPrice, upDownClass } from "../lib/format";

const TABS = [
  { value: "test", label: "① 因子检验", hint: "每个因子单独打分，看它有没有预测力" },
  { value: "portfolio", label: "② 多因子组合", hint: "几个因子合成一个分数，模拟定期持有得分最高的一批股票" },
] as const;

/** 两个标签各自回答的问题，显示在标签下面 */
const TAB_INTRO = {
  test: "第一步：逐个体检因子。每个勾选的因子单独给全市场排名，看排名靠前的股票之后是否真的涨得更多，因子之间互不影响；不算交易成本，也不是一个能直接用的选股方案。",
  portfolio:
    "第二步：把几个有效的因子合成一个选股方案。勾选的因子按权重合成一个分数，模拟每隔一段时间买入得分最高的若干只、扣掉交易成本，看这套方案能赚多少。调好后可以存成预设，在账户设置里一键套用。",
} as const;

/** 持有只数和缓冲区：组合回测的参数，套用自己保存的预设时一起改 */
const HOLDING = { top: "30", buffer: "0.5" };

/** 计算区间的快捷选项；更长或更早的区间在日期框里选，因子检验最长 8 年、组合回测最长 5 年 */
const RANGES = [
  { value: "3m", label: "3 个月" },
  { value: "6m", label: "6 个月" },
  { value: "1", label: "1 年" },
  { value: "3", label: "3 年" },
] as const;
const HORIZONS = [
  { value: "5", label: "5 日", hint: "每周取样一次，看之后 5 个交易日的收益" },
  { value: "10", label: "10 日" },
  { value: "20", label: "20 日", hint: "每月取样一次，看之后 20 个交易日的收益" },
] as const;
const REBALANCES = [
  { value: "5", label: "每周" },
  { value: "10", label: "两周" },
  { value: "20", label: "每月" },
  { value: "60", label: "每季" },
] as const;
const NEUTRALIZE_HELP =
  "把行业和公司大小对因子的影响扣掉，只留下因子自己的部分。比如波动率低的股票多是银行、公用事业和大盘股，不中性化时「波动率」选出来的大半是这几类，赚赔的其实是行业和大小盘的涨跌。做法是每个取样日把因子值对行业和规模（20 日平均成交额的对数）回归，用残差代替原值。规模因子本身只扣行业";

const SMALL = [
  { value: "0", label: "不剔除" },
  { value: "30", label: "剔除最小 30%", hint: "按成交额剔除最小的 30%：小盘股里有一部分是壳价值，不是因子带来的收益" },
] as const;
const ON_OFF = [
  { value: "1", label: "行业和规模中性" },
  { value: "", label: "不中性化", hint: "因子里会混入行业和大小盘的影响" },
] as const;

export function ResearchPage() {
  const [s, set] = useSearchState({ tab: "test", f: DEFAULT_FACTORS });
  const [, setHolding] = useSearchState(HOLDING);
  const picks = parseFactors(s.f);
  const portfolio = s.tab === "portfolio";
  return (
    <>
      <PageHeader
        title="因子研究"
        desc="因子是用来给股票打分的可量化特征，例如过去 20 日的波动率、单季利润增速。因子检验在每个取样日按因子给全市场股票排名，衡量排名与之后收益的相关程度；多因子组合把多个因子按权重合成一个得分，模拟定期持有得分最高的一批股票，并计入交易成本。"
      />
      <Card>
        <div className="flex flex-col gap-3">
          <Segmented value={s.tab as "test" | "portfolio"} options={TABS} onChange={(tab) => set({ tab })} />
          <p className="text-xs text-zinc-500">{TAB_INTRO[portfolio ? "portfolio" : "test"]}</p>
          <FactorPicker
            picks={picks}
            onChange={(next) => set({ f: formatFactors(next) })}
            onPreset={(p) => {
              set({ f: formatFactors(p.factors) });
              if (p.top_n !== undefined) setHolding({ top: String(p.top_n), buffer: String(p.buffer ?? 0.5) });
            }}
            presets={portfolio}
            weights={portfolio}
          />
        </div>
      </Card>
      {picks.length === 0 ? (
        <Card>
          <Empty>至少选一个因子</Empty>
        </Card>
      ) : s.tab === "portfolio" ? (
        <PortfolioPanel picks={picks} />
      ) : (
        <TestPanel picks={picks} onCompose={(f) => set({ tab: "portfolio", f: formatFactors(f) })} />
      )}
    </>
  );
}

/** 计算的通用状态：只有点按钮时才把参数固定下来，算的时候可以取消 */
function useManualRun<P>(form: P, queryKey: (p: P) => readonly unknown[]) {
  const qc = useQueryClient();
  const [run, setRun] = useState<P | null>(null);
  const changed = run !== null && JSON.stringify(run) !== JSON.stringify(form);
  return {
    run,
    changed,
    start: (refetch: () => unknown) => (run && !changed ? refetch() : setRun(form)),
    cancel: () => {
      if (run) void qc.cancelQueries({ queryKey: queryKey(run) });
      setRun(null);
    },
  };
}

const QUERY_OPTIONS = {
  staleTime: 10 * 60_000,
  placeholderData: keepPreviousData,
  retry: false,
  refetchOnWindowFocus: false,
} as const;

function RunBar({
  running,
  seconds,
  changed,
  hasRun,
  onStart,
  onCancel,
  estimate,
}: {
  running: boolean;
  seconds: number;
  changed: boolean;
  hasRun: boolean;
  onStart: () => void;
  onCancel: () => void;
  estimate: string;
}) {
  return (
    <>
      <div className="mt-4 flex flex-wrap items-center gap-3">
        {running ? (
          <TaskRunning text="正在逐只计算全市场几年的日线和因子" seconds={seconds} onCancel={onCancel} />
        ) : (
          <>
            <button type="button" className={buttonClass()} onClick={onStart}>
              {hasRun && changed ? "按新参数重新计算" : hasRun ? "重新计算" : "开始计算"}
            </button>
            <span className="text-xs text-zinc-500">{estimate}</span>
          </>
        )}
      </div>
      {changed && !running && (
        <p className="mt-2 text-xs text-amber-600">参数已改，下面还是上一次的结果；点上面的按钮按新参数重新计算。</p>
      )}
    </>
  );
}

function TestPanel({ picks, onCompose }: { picks: FactorWeight[]; onCompose: (f: FactorWeight[]) => void }) {
  const [s, set] = useSearchState({ range: "3", from: "", to: "", h: "5", q: "5", n: "1", small: "0" });
  const form: FactorTestParams = {
    factors: formatFactors(picks, false),
    ...rangeQuery(s),
    horizon: Number(s.h),
    quantiles: Number(s.q),
    neutralize: s.n === "1",
    exclude_small_pct: Number(s.small),
  };
  const m = useManualRun(form, getFactorTestQueryKey);
  const query = useFactorTest(m.run ?? form, { query: { ...QUERY_OPTIONS, enabled: m.run !== null } });
  const running = m.run !== null && query.isFetching;
  const seconds = useElapsedSeconds(running);
  return (
    <>
      <Card>
        <div className="flex flex-wrap items-end gap-4">
          <Labeled label="计算区间（最长 8 年）">
            <RangePicker value={s} ranges={RANGES} onChange={set} />
          </Labeled>
          <Labeled label="持有期">
            <Segmented value={s.h} options={HORIZONS} onChange={(h) => set({ h })} />
          </Labeled>
          <Labeled label="分几组">
            <Segmented
              value={s.q}
              options={[
                { value: "5", label: "5 组" },
                { value: "10", label: "10 组" },
              ]}
              onChange={(q) => set({ q })}
            />
          </Labeled>
          <Labeled label="中性化" help={NEUTRALIZE_HELP}>
            <Segmented value={s.n} options={ON_OFF} onChange={(n) => set({ n })} />
          </Labeled>
          <Labeled label="小盘股">
            <Segmented value={s.small} options={SMALL} onChange={(small) => set({ small })} />
          </Labeled>
        </div>
        <RunBar
          running={running}
          seconds={seconds}
          changed={m.changed}
          hasRun={m.run !== null}
          onStart={() => m.start(query.refetch)}
          onCancel={m.cancel}
          estimate="区间越长越慢：全市场 5 年通常 20 到 40 秒；含财务因子时更久"
        />
      </Card>
      {m.run !== null &&
        (query.error && !running ? (
          <Card>
            <ErrorState error={query.error} onRetry={() => query.refetch()} />
          </Card>
        ) : query.data !== undefined ? (
          <TestResult data={query.data} onCompose={onCompose} />
        ) : running ? null : (
          <Card>
            <Spinner />
          </Card>
        ))}
      <RulesNote
        items={[
          "每隔「持有期」个交易日取样一次：当天收盘时给股票池里的每只股票算因子值，下一个交易日开盘买入、持有期结束后的开盘卖出，用前复权价算收益。买入日停牌或一字涨停的股票不计入，卖出日一字跌停顺延。",
          "因子值先去极值（中位数 ± 3 倍稳健标准差），再标准化；选择中性化时对同花顺行业和规模（20 日平均成交额的对数）回归取残差。规模因子本身只做行业中性化。",
          "信息系数（RankIC）是因子排名和之后收益排名的相关系数，A 股里单个因子稳定在 0.03 以上就算有用。t 值衡量平均信息系数离 0 有多远，按 Harvey、Liu、Zhu 的建议要达到 3 才算显著：因子被试得越多，偶然显著的就越多。",
          "过拟合概率（PBO）用组合对称交叉验证估计：把时间切成 8 段，枚举一半做样本内、一半做样本外，样本内最好的候选在样本外排到后一半的比例。真实有效的概率（Deflated Sharpe，校正夏普比率）是考虑挑选次数之后真实夏普比率大于 0 的概率。",
          "财务因子按法定披露截止日（一季报 4 月底、半年报 8 月底、三季报 10 月底、年报次年 4 月底）之后才使用；供应商的报告日期会被之后的报告覆盖，不能直接用。",
          "已知偏差：本地没有开始采集之前就退市的股票（幸存者偏差，结果偏乐观）；行业用当前归属；没有历史股本，用成交额代替市值。",
        ]}
      />
    </>
  );
}

/** 参数名和控件；help 是参数名后面小问号里的说明 */
function Labeled({ label, help, children }: { label: string; help?: React.ReactNode; children: React.ReactNode }) {
  return (
    <div className="flex flex-col gap-1 text-xs text-zinc-500">
      {help ? <Term help={help}>{label}</Term> : <span>{label}</span>}
      {children}
    </div>
  );
}

function TestResult({ data, onCompose }: { data: FactorTestResult; onCompose: (f: FactorWeight[]) => void }) {
  const [selected, setSelected] = useState(0);
  const f = data.factors[Math.min(selected, data.factors.length - 1)];
  const o = data.overfit;
  if (!f) return null;
  return (
    <>
      <Card
        title={
          <span className="text-xs font-normal text-zinc-500">
            {data.start} 至 {data.end} · 持有期 {data.horizon} 个交易日 · 分 {data.quantiles} 组 ·{" "}
            {data.config.neutralize ? "行业和规模中性" : "未中性化"}
            {data.thin_days.length > 0 && ` · 跳过同步缺口 ${data.thin_days.join("、")}`}
          </span>
        }
      >
        <Table>
          <thead>
            <tr>
              <th>因子</th>
              <th>
                <Term help="RankIC：每期因子排名和之后收益排名的相关系数，这里是各期平均。按预期方向统一，正数表示符合预期；A 股里单个因子稳定在 0.03 以上就算有用">
                  信息系数
                </Term>
              </th>
              <th>
                <Term help="ICIR：信息系数的均值 ÷ 标准差。越大说明每期的预测力越稳定，不是靠少数几期撑起来的">
                  IC 稳定性
                </Term>
              </th>
              <th>
                <Term help="统计检验里的 t 统计量：平均信息系数离 0 有多远。3 以上才算显著，越大越不像偶然">t 值</Term>
              </th>
              <th>
                <Term help="信息系数为正的期数占比">胜率</Term>
              </th>
              <th>
                <Term help="多空组合：买最好一组、卖空最差一组的年化收益，不含交易成本。A 股个股不能随意卖空，这个数只用来衡量因子的区分能力">
                  多空年化
                </Term>
              </th>
              <th>
                <Term help="最好一组相对股票池平均的年化超额收益，不含交易成本">最好组超额</Term>
              </th>
              <th>
                <Term help="最好一组每期换掉的股票比例，越高交易成本越大">换手</Term>
              </th>
              <th>结论</th>
            </tr>
          </thead>
          <tbody>
            {data.factors.map((x, i) => (
              <tr
                key={x.name}
                onClick={() => setSelected(i)}
                className={`cursor-pointer ${i === selected ? "bg-accent/5" : "hover:bg-zinc-50 dark:hover:bg-zinc-800/50"}`}
              >
                <td className="font-medium">
                  {x.name}
                  {x.id === null || x.id === undefined ? (
                    <span className="ml-1 text-xs text-zinc-400">（以上因子等权）</span>
                  ) : null}
                </td>
                <td className={`num ${upDownClass(x.rank_ic_mean)}`}>{fmtNum(x.rank_ic_mean, 3)}</td>
                <td className="num">{fmtNum(x.rank_icir)}</td>
                <td className="num">{fmtNum(x.rank_ic_t)}</td>
                <td className="num">{fmtProb(x.hit_rate)}</td>
                <td className={`num ${upDownClass(x.long_short_annual)}`}>{fmtPercent(x.long_short_annual, 1)}</td>
                <td className={`num ${upDownClass(x.top_excess_annual)}`}>{fmtPercent(x.top_excess_annual, 1)}</td>
                <td className="num">{fmtProb(x.top_turnover)}</td>
                <td>
                  <Badge tone={verdictTone(x)}>{x.verdict}</Badge>
                </td>
              </tr>
            ))}
          </tbody>
        </Table>
        <p className="mt-3 text-xs text-zinc-500">
          点一行看它的分组净值、预测力衰减；收益都不含交易成本，换手越高实际越打折。
        </p>
        <ComposeBar data={data} onCompose={onCompose} />
      </Card>
      <div className="grid gap-4 lg:grid-cols-3">
        <Card title={`${f.name}：各组净值`} className="lg:col-span-2">
          <QuantileChart f={f} />
        </Card>
        <Card title="挑选偏差">
          <div className="grid grid-cols-2 gap-4">
            <Stat
              label={
                <Term help="PBO（Probability of Backtest Overfitting）：把历史切成多段反复交叉验证，看前一半时间里最好的候选，到后一半时间里排到下半区的比例。高说明是从噪声里挑出来的">
                  过拟合概率
                </Term>
              }
              value={fmtProb(o.pbo)}
              sub={o.pbo === null || o.pbo === undefined ? "至少两个候选才能估计" : "越低越好，高于 50% 说明在挑噪声"}
            />
            <Stat
              label={<Term help={DEFLATED_SHARPE_HELP}>真实有效的概率</Term>}
              value={fmtProb(o.deflated_sharpe)}
              sub={o.best ? `最好的候选「${o.best}」扣除挑选次数后，真实有效的概率；95% 以上才可信` : undefined}
            />
          </div>
          <p className="mt-3 text-xs text-zinc-500">
            本次比较 {o.candidates} 个候选（各因子的多空组合）；这类检验已试过 {data.manifest.trial_count} 组不同配置。
          </p>
          <h3 className="mt-4 mb-1 text-xs font-medium text-zinc-500">
            <Term help="IC 衰减：同一个因子在不同持有期下的信息系数。下降得慢说明因子的预测力能维持较久">
              预测力衰减（不同持有期的信息系数）
            </Term>
          </h3>
          <div className="flex gap-3 text-sm">
            {f.decay.map((d) => (
              <span key={d.horizon} className="num">
                {d.horizon} 日 <b className={upDownClass(d.rank_ic)}>{fmtNum(d.rank_ic, 3)}</b>
              </span>
            ))}
          </div>
          <p className="mt-1 text-xs text-zinc-500">持有期越长信息系数仍高，说明因子衰减慢，可以少调仓、省成本。</p>
        </Card>
      </div>
      {data.factors.length > 1 && <Correlation data={data} />}
    </>
  );
}

/** 检验完带着因子去第二步：默认只带显著且方向符合预期的因子 */
function ComposeBar({ data, onCompose }: { data: FactorTestResult; onCompose: (f: FactorWeight[]) => void }) {
  const single = data.factors.filter((x) => x.id !== null && x.id !== undefined);
  const good = single.filter((x) => x.significant && x.consistent);
  const pick = (xs: typeof single) => xs.map((x) => ({ id: x.id as string, weight: 1 }));
  return (
    <div className="mt-3 flex flex-wrap items-center gap-2 border-t border-zinc-100 pt-3 text-xs text-zinc-500 dark:border-zinc-800">
      <span>下一步：</span>
      {good.length > 0 ? (
        <button type="button" className={buttonClass()} onClick={() => onCompose(pick(good))}>
          用 {good.length} 个有效因子做组合回测
        </button>
      ) : (
        <span>这次没有显著且方向符合预期的因子。</span>
      )}
      <button type="button" className={buttonClass("ghost")} onClick={() => onCompose(pick(single))}>
        用全部 {single.length} 个因子做组合回测
      </button>
    </div>
  );
}

function QuantileChart({ f }: { f: FactorTestStats }) {
  const lines: Line[] = useMemo(() => {
    const n = f.quantile_returns.length;
    const colors = quantileColors(n);
    const out: Line[] = Array.from({ length: n }, (_, g) => ({
      label: g === 0 ? "第 1 组（预期最差）" : g === n - 1 ? `第 ${n} 组（预期最好）` : `第 ${g + 1} 组`,
      color: colors[g] ?? "#a1a1aa",
      width: g === 0 || g === n - 1 ? 2 : 1,
      points: f.points.map((p) => ({ date: p.date, value: p.quantile_navs[g] ?? Number.NaN })),
    }));
    out.push({
      label: "多空（最好减最差）",
      color: "#3b6ef5",
      width: 2,
      points: f.points.map((p) => ({ date: p.date, value: p.long_short_nav })),
    });
    return out;
  }, [f]);
  if (f.points.length === 0) return <Empty>有效期数太少</Empty>;
  return (
    <>
      <MultiLineChart lines={lines} />
      <p className="mt-2 text-xs text-zinc-500">
        好因子的各组从下到上依次排开，多空线稳定向上。各组年化（不含成本）：
        {f.quantile_annual.map((v, g) => (
          // biome-ignore lint/suspicious/noArrayIndexKey: 组序号就是身份
          <span key={g} className={`num ml-2 ${upDownClass(v)}`}>
            {g + 1} 组 {fmtPercent(v, 1)}
          </span>
        ))}
      </p>
    </>
  );
}

function Correlation({ data }: { data: FactorTestResult }) {
  const cell = (v: number | null | undefined) => {
    if (v === null || v === undefined) return "";
    const a = Math.min(Math.abs(v), 1);
    return v >= 0 ? `rgba(59, 110, 245, ${a * 0.5})` : `rgba(220, 38, 38, ${a * 0.5})`;
  };
  return (
    <Card title="因子之间的相关性">
      <Table>
        <thead>
          <tr>
            <th />
            {data.factors.map((f) => (
              <th key={f.name}>{f.name}</th>
            ))}
          </tr>
        </thead>
        <tbody>
          {data.correlation.map((row, i) => (
            <tr key={data.factors[i]?.name ?? i}>
              <td className="text-xs text-zinc-500">{data.factors[i]?.name}</td>
              {row.map((v, j) => (
                <td key={data.factors[j]?.name ?? j} className="num text-center" style={{ background: cell(v) }}>
                  {fmtNum(v)}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </Table>
      <p className="mt-3 text-xs text-zinc-500">
        平均横截面秩相关（方向已统一）。两个因子相关性高于 0.7 时信息大量重复，合成时只留一个或降低权重。
      </p>
    </Card>
  );
}

const DEFLATED_SHARPE_HELP =
  "Deflated Sharpe（校正夏普比率）：同样的数据试过很多组参数后，总有几组碰巧好看。这个数按试过的次数打折，估计真实夏普比率大于 0 的概率；95% 以上才可信";

const PORTFOLIO_COSTS = { capital: "1000000", commission: "2.5", minFee: "5", stamp: "0.5", slippage: "10" };

function PortfolioPanel({ picks }: { picks: FactorWeight[] }) {
  const [s, set] = useSearchState({
    range: "3",
    from: "",
    to: "",
    reb: "20",
    ...HOLDING,
    n: "1",
    small: "0",
    log: "",
    ...PORTFOLIO_COSTS,
  });
  const form: FactorPortfolioParams = {
    factors: formatFactors(picks),
    ...rangeQuery(s),
    rebalance: Number(s.reb),
    top_n: Number(s.top) || 30,
    buffer: Number(s.buffer),
    neutralize: s.n === "1",
    exclude_small_pct: Number(s.small),
    ...costParams(s),
  };
  const m = useManualRun(form, getFactorPortfolioQueryKey);
  const query = useFactorPortfolio(m.run ?? form, { query: { ...QUERY_OPTIONS, enabled: m.run !== null } });
  const running = m.run !== null && query.isFetching;
  const seconds = useElapsedSeconds(running);
  return (
    <>
      <Card>
        <div className="flex flex-wrap items-end gap-4">
          <Labeled label="计算区间（最长 5 年）">
            <RangePicker value={s} ranges={RANGES} onChange={set} />
          </Labeled>
          <Labeled label="多久调仓一次">
            <Segmented value={s.reb} options={REBALANCES} onChange={(reb) => set({ reb })} />
          </Labeled>
          <NumberField
            label="持有几只"
            hint={`${TOP_N_HELP}。可以填 5 到 100，要套用到账户时不超过 50`}
            value={s.top}
            onCommit={(top) => set({ top })}
            width="w-16"
          />
          <NumberField
            label="缓冲区"
            hint={BUFFER_HELP}
            value={s.buffer}
            onCommit={(buffer) => set({ buffer })}
            width="w-16"
          />
          <Labeled label="中性化" help={NEUTRALIZE_HELP}>
            <Segmented value={s.n} options={ON_OFF} onChange={(n) => set({ n })} />
          </Labeled>
          <Labeled label="小盘股">
            <Segmented value={s.small} options={SMALL} onChange={(small) => set({ small })} />
          </Labeled>
          <CostFields s={s} set={set} />
        </div>
        <RunBar
          running={running}
          seconds={seconds}
          changed={m.changed}
          hasRun={m.run !== null}
          onStart={() => m.start(query.refetch)}
          onCancel={m.cancel}
          estimate="区间越长越慢：全市场 3 年通常 20 到 40 秒"
        />
        <SavePreset picks={picks} topN={Number(s.top) || 30} buffer={Number(s.buffer)} />
      </Card>
      {m.run !== null &&
        (query.error && !running ? (
          <Card>
            <ErrorState error={query.error} onRetry={() => query.refetch()} />
          </Card>
        ) : query.data !== undefined ? (
          <PortfolioResultView data={query.data} log={s.log === "1"} onLog={(v) => set({ log: v ? "1" : "" })} />
        ) : running ? null : (
          <Card>
            <Spinner />
          </Card>
        ))}
      <RulesNote
        items={[
          "每到调仓日收盘，给股票池里的股票按选中因子的合成得分排名：各因子先标准化并统一成「越大越好」，再按权重加权平均（至少一半权重的因子有值才参与）。",
          "持有得分最高的若干只；已持有的股票排名还在「持有只数 ×（1 + 缓冲区）」以内时继续持有，掉出才卖。下一个交易日开盘先卖后买，新买的每只投入「账户总值 ÷ 持有只数」和「现金 ÷ 待买只数」中较小的一个。",
          "卖出遇到停牌、一字跌停时顺延；买入遇到停牌、一字涨停时顺延，到下一次调仓时取消。之后再没有日线的股票按最后收盘价了结。收益按前复权价计算（相当于分红再投资），允许零股。",
          "佣金（含最低佣金）、印花税、滑点照常扣除。资金少、持有只数多时，最低佣金会明显拖累收益，所以这里默认用 100 万。",
          "基准是全市场等权（含分红），和组合回测相同。真实有效的概率（Deflated Sharpe）按这类回测已经试过的配置数打折：试得越多，偶然好看的结果越多。",
          "已知偏差：没有开始采集之前就退市的股票（幸存者偏差，结果偏乐观）；行业用当前归属；没有历史股本，用成交额代替市值。",
        ]}
      />
    </>
  );
}

/** 把当前的因子和权重、持有只数、缓冲区存成预设，账户设置里一键套用；同名覆盖 */
function SavePreset({ picks, topN, buffer }: { picks: FactorWeight[]; topN: number; buffer: number }) {
  const qc = useQueryClient();
  const [name, setName] = useState("");
  const [saved, setSaved] = useState("");
  const save = useSavePreset({
    mutation: {
      onSuccess: (p) => {
        setSaved(p.name);
        setName("");
        void qc.invalidateQueries({ queryKey: getListPresetsQueryKey() });
      },
    },
  });
  const submit = () => {
    if (!name.trim()) return;
    setSaved("");
    save.mutate({
      data: {
        name: name.trim(),
        factors: picks.map((p) => ({ factor: p.id as FactorId, weight: p.weight })),
        top_n: topN,
        buffer,
      },
    });
  };
  return (
    <div className="mt-4 flex flex-wrap items-center gap-2 border-t border-zinc-100 pt-4 text-xs text-zinc-500 dark:border-zinc-800">
      <span>保存为预设：</span>
      <input
        aria-label="预设名"
        className={`${inputClass} w-48 py-1 text-xs`}
        maxLength={30}
        placeholder="起个名字，例如：稳健 30 只"
        value={name}
        onChange={(e) => setName(e.target.value)}
        onKeyDown={(e) => e.key === "Enter" && submit()}
      />
      <button type="button" className={buttonClass("ghost")} disabled={!name.trim() || save.isPending} onClick={submit}>
        保存
      </button>
      <span>
        存下 {picks.length} 个因子和权重、持有 {topN} 只、缓冲区 {buffer}
        ；同名会覆盖。账户设置和这里的「我的预设」都能一键套用。
      </span>
      {saved && <span className="text-emerald-600">已保存「{saved}」</span>}
      {save.error && <span className="text-red-500">{errorText(save.error)}</span>}
    </div>
  );
}

function PortfolioResultView({
  data,
  log,
  onLog,
}: {
  data: FactorPortfolioResult;
  log: boolean;
  onLog: (v: boolean) => void;
}) {
  const m = data.summary;
  const excess = m.excess_annual ?? null;
  return (
    <>
      <Card
        title={
          <span className="text-xs font-normal text-zinc-500">
            {m.start} 至 {m.end}（{fmtInt(m.days)} 个交易日）· 持有 {data.config.top_n} 只 · 每 {data.config.horizon}{" "}
            个交易日调仓 · 调仓 {m.rebalances} 次
          </span>
        }
      >
        <p className="mb-4 text-base">
          {fmtBig(m.capital)} 元按这组因子持有，变成{" "}
          <b className={upDownClass(m.total_return)}>{fmtBig(m.final_equity)} 元</b>（{fmtPercent(m.total_return)}）；
          {m.benchmark_name}是 <b className={upDownClass(m.benchmark_return)}>{fmtPercent(m.benchmark_return)}</b>。
          {excess !== null &&
            (excess >= 0 ? (
              <span className="text-up">每年多赚 {fmtPercent(excess)}。</span>
            ) : (
              <span className="text-down">每年少赚 {fmtPercent(-excess)}。</span>
            ))}
        </p>
        <div className="grid grid-cols-2 gap-4 sm:grid-cols-4">
          <Stat
            label="年化收益"
            value={<span className={upDownClass(m.annual_return)}>{fmtPercent(m.annual_return)}</span>}
            sub={`基准年化 ${fmtPercent(m.benchmark_annual)}`}
          />
          <Stat
            label="最多亏过"
            value={<span className="text-down">{fmtPercent(-m.max_drawdown)}</span>}
            sub={
              <Term help="夏普比率（Sharpe）：年化收益 ÷ 年化波动，每承担一份波动换来多少收益。1 以上算不错">
                夏普比率 {fmtNum(m.risk.sharpe)}
              </Term>
            }
          />
          <Stat
            label={
              <Term help="信息比率（IR）：年化超额收益 ÷ 跟踪误差，衡量相对基准多赚的部分稳不稳。跟踪误差是相对基准的超额收益的年化波动">
                信息比率
              </Term>
            }
            value={fmtNum(m.information_ratio)}
            sub={`跟踪误差 ${fmtPercent(m.tracking_error, 1).replace("+", "")}；0.5 以上算不错`}
          />
          <Stat
            label={<Term help={DEFLATED_SHARPE_HELP}>真实有效的概率</Term>}
            value={fmtProb(data.deflated_sharpe)}
            sub={`已按试过的 ${data.manifest.trial_count} 组配置打折；95% 以上才可信`}
          />
        </div>
        <p className="mt-3 text-xs text-zinc-500">
          交易成本 {fmtBig(m.fees)} 元；年换手 {fmtNum(m.turnover_annual, 1)} 倍（每年买入金额 ÷ 平均账户总值）；
          平均持有 {fmtNum(m.avg_positions, 1)} 只；买不进被取消 {m.skipped_buys} 次。
        </p>
        <div className="mt-5">
          <EquityChart points={data.equity} capital={m.capital} log={log} benchmarkLabel={m.benchmark_name} />
          <LogToggle log={log} onLog={onLog} note={`蓝线是因子组合，灰线是${m.benchmark_name}。`} />
        </div>
      </Card>
      <TradeHistory trades={data.trades} />
      <Card title={data.picks_date ? `${data.picks_date} 收盘后得分最高的 ${data.picks.length} 只` : "最新得分"}>
        {data.picks.length === 0 ? (
          <Empty>最近一个交易日没有足够的股票参与排名</Empty>
        ) : (
          <Table>
            <thead>
              <tr>
                <th>排名</th>
                <th>股票</th>
                <th>
                  <Term help="合成得分：各因子标准化后按权重加权平均，越大越好">得分</Term>
                </th>
                <th>组合里已持有</th>
              </tr>
            </thead>
            <tbody>
              {data.picks.map((p) => (
                <tr key={p.thscode}>
                  <td className="num">{p.rank}</td>
                  <td>
                    <Link to={`/securities/${p.thscode}`} className="hover:text-accent">
                      {p.name || p.thscode} <span className="text-xs text-zinc-400">{p.thscode}</span>
                    </Link>
                  </td>
                  <td className="num">{fmtNum(p.score)}</td>
                  <td>{p.held ? <Badge tone="info">已持有</Badge> : null}</td>
                </tr>
              ))}
            </tbody>
          </Table>
        )}
        <p className="mt-3 text-xs text-zinc-500">只是按历史规律排出的名单，不是买卖建议；历史表现不代表未来。</p>
      </Card>
    </>
  );
}

const SIDE_LABEL: Record<FactorTradeSide, { label: string; tone: "bad" | "ok" | "muted" }> = {
  buy: { label: "买入", tone: "bad" },
  sell: { label: "卖出", tone: "ok" },
  liquidate: { label: "了结", tone: "muted" },
};
const SIDE_FILTERS = [
  { value: "all", label: "全部" },
  { value: "buy", label: "买入" },
  { value: "sell", label: "卖出", hint: "含按最后收盘价了结的" },
] as const;
const TRADE_PAGE = 50;

function signedBig(v: number | null | undefined): string {
  return v === null || v === undefined ? "—" : `${v > 0 ? "+" : ""}${fmtBig(v)}`;
}

/** 组合回测的成交记录：最新的在前，可以按买卖方向和股票筛选 */
function TradeHistory({ trades }: { trades: FactorTrade[] }) {
  const [side, setSide] = useState<(typeof SIDE_FILTERS)[number]["value"]>("all");
  const [keyword, setKeyword] = useState("");
  const [shown, setShown] = useState(TRADE_PAGE);
  const closed = trades.filter((t) => t.side !== "buy");
  const wins = closed.filter((t) => (t.profit ?? 0) > 0).length;
  const realized = closed.reduce((sum, t) => sum + (t.profit ?? 0), 0);
  const rows = useMemo(() => {
    const k = keyword.trim().toUpperCase();
    return trades
      .filter((t) => side === "all" || (side === "buy" ? t.side === "buy" : t.side !== "buy"))
      .filter((t) => !k || t.thscode.includes(k) || t.name.toUpperCase().includes(k))
      .reverse();
  }, [trades, side, keyword]);
  return (
    <Card
      title={`成交记录（${fmtInt(trades.length)} 笔）`}
      extra={
        <div className="flex flex-wrap items-center gap-2">
          <input
            aria-label="按股票筛选"
            className={`${inputClass} w-40 py-1 text-xs`}
            placeholder="股票名称或代码"
            value={keyword}
            onChange={(e) => {
              setKeyword(e.target.value);
              setShown(TRADE_PAGE);
            }}
          />
          <Segmented
            value={side}
            options={SIDE_FILTERS}
            onChange={(v) => {
              setSide(v);
              setShown(TRADE_PAGE);
            }}
          />
        </div>
      }
    >
      <p className="mb-3 text-xs text-zinc-500">
        买入 {fmtInt(trades.length - closed.length)} 笔，卖出 {fmtInt(closed.length)} 笔
        {closed.length > 0 && (
          <>
            ；卖出的里面赚钱的 {fmtInt(wins)} 笔（{Math.round((wins / closed.length) * 100)}%），合计{" "}
            <span className={upDownClass(realized)}>{signedBig(realized)} 元</span>
          </>
        )}
        。区间结束时还拿着的股票不在卖出里，它们的盈亏算在最终市值中。
      </p>
      {rows.length === 0 ? (
        <Empty>{trades.length === 0 ? "区间内没有成交" : "没有符合条件的成交"}</Empty>
      ) : (
        <Table>
          <thead>
            <tr>
              <th>日期</th>
              <th>操作</th>
              <th>股票</th>
              <th>
                <Term help="前复权价：把之后的分红送转折算进去的价格，和当时行情软件上的实际价格不同，但算出的涨跌幅一致。已含滑点">
                  成交价
                </Term>
              </th>
              <th>金额（元）</th>
              <th>费用（元）</th>
              <th>
                <Term help="卖出所得减去当初买入的花费，买卖两边的佣金、印花税、过户费都已扣除">盈亏（元）</Term>
              </th>
              <th>持有</th>
            </tr>
          </thead>
          <tbody>
            {rows.slice(0, shown).map((t) => (
              <tr key={`${t.date}-${t.thscode}-${t.side}`}>
                <td className="num">{t.date}</td>
                <td>
                  <Badge tone={SIDE_LABEL[t.side].tone}>{SIDE_LABEL[t.side].label}</Badge>
                </td>
                <td>
                  <Link to={`/securities/${t.thscode}`} className="hover:text-accent">
                    {t.name || t.thscode} <span className="text-xs text-zinc-400">{t.thscode}</span>
                  </Link>
                </td>
                <td className="num">{fmtPrice(t.price)}</td>
                <td className="num">{fmtBig(t.amount)}</td>
                <td className="num">{t.fee > 0 ? fmtNum(t.fee, 0) : "—"}</td>
                <td className={`num ${upDownClass(t.profit)}`}>
                  {t.profit === null || t.profit === undefined ? (
                    "—"
                  ) : (
                    <>
                      {signedBig(t.profit)}
                      <span className="ml-1 text-xs">（{fmtPercent(t.profit_pct)}）</span>
                    </>
                  )}
                </td>
                <td className="num">{t.held_days === null || t.held_days === undefined ? "—" : `${t.held_days} 天`}</td>
              </tr>
            ))}
          </tbody>
        </Table>
      )}
      {rows.length > shown && (
        <div className="mt-3 flex items-center gap-3 text-xs text-zinc-500">
          <button type="button" className={buttonClass("ghost")} onClick={() => setShown(shown + TRADE_PAGE * 4)}>
            再显示 {Math.min(TRADE_PAGE * 4, rows.length - shown)} 笔
          </button>
          <span>
            已显示 {fmtInt(shown)} / {fmtInt(rows.length)} 笔
          </span>
        </div>
      )}
      <p className="mt-3 text-xs text-zinc-500">
        「持有」是交易日数。「了结」是之后再没有日线（退市或一直停牌到区间结束）的股票，按最后收盘价算钱，不收费用。
      </p>
    </Card>
  );
}
