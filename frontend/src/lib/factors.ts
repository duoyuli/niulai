/** 因子研究页的纯函数：选中的因子和权重在 URL 里的写法、结论的颜色。规则见 docs/domains/factor_research.md */
import type { FactorTestStats } from "../api/generated/model";
import type { Tone } from "./labels";

/** 默认选中的因子：A 股文献里较稳健的价量因子，不需要财务数据 */
export const DEFAULT_FACTORS = "reversal,idio_vol,max_return,liquidity,abnormal_turnover";

export type BuiltinFactorPreset = {
  id: string;
  name: string;
  desc: string;
  factors: readonly FactorWeight[];
  max_price?: number;
};

/** 预设的多因子组合：点一下替换当前选择，按各因子的预期方向合成。
 * 描述只讲选股逻辑，不写来历和回测数字；第一个与后端 `Rules::default` 相同 */
export const FACTOR_PRESETS: readonly BuiltinFactorPreset[] = [
  {
    id: "e",
    name: "低波动 + 盈利质量 + 小盘",
    desc: "挑选同时满足这几点的公司：最近一个月股价涨跌平稳；最新一个季度的利润比去年同期多；利润是实打实收到的现金，不是账面上的应收款；公司规模偏小（用成交额衡量）。平稳的股票回撤小，利润增长且质量高的公司基本面好，小公司长期涨得比大公司多",
    factors: [
      { id: "volatility", weight: 1 },
      { id: "idio_vol", weight: 1 },
      { id: "profit_growth", weight: 1 },
      { id: "accruals", weight: 1 },
      { id: "liquidity", weight: 1 },
    ],
  },
  {
    id: "price",
    name: "短期价量",
    desc: "只看股价和成交量：挑最近一周跌得多的股票（A 股短期涨多了容易跌回去，跌多了容易涨回来），同时要求它最近一个月走势平稳、没有出现过单日暴涨、规模偏小、最近几天成交没有突然放大。这些指标变化快，持仓换得比较勤",
    factors: [
      { id: "reversal", weight: 1 },
      { id: "idio_vol", weight: 1 },
      { id: "max_return", weight: 1 },
      { id: "liquidity", weight: 1 },
      { id: "abnormal_turnover", weight: 1 },
    ],
  },
  {
    id: "price_fundamental",
    name: "价量 + 财务",
    desc: "股价方面，挑最近一个月没怎么涨、走势平稳、没有出现过单日暴涨、成交没有突然放大的股票，避开追涨和炒作过热的；财报方面，再要求最新一个季度利润比去年同期增长、利润是实打实收到的现金。财报指标变化慢，能让持仓换得少一些",
    factors: [
      { id: "momentum", weight: 1 },
      { id: "idio_vol", weight: 1 },
      { id: "max_return", weight: 1 },
      { id: "abnormal_turnover", weight: 1 },
      { id: "profit_growth", weight: 1 },
      { id: "accruals", weight: 1 },
    ],
  },
  {
    id: "fundamental_reversal",
    name: "基本面困境反转",
    desc: "挑选最近一周明显下跌、但最新季度利润正在改善且现金质量较好的股票，再用毛盈利和低彩票特征过滤基本面恶化与短线炒作。适合先做研究实验，调仓周期应与短期反转信号匹配",
    factors: [
      { id: "reversal", weight: 1 },
      { id: "profit_growth", weight: 1 },
      { id: "accruals", weight: 0.75 },
      { id: "max_return", weight: 0.75 },
      { id: "gross_profitability", weight: 0.5 },
    ],
  },
  {
    id: "residual_momentum_quality",
    name: "特质动量 + 景气质量",
    desc: "挑选剥离市场共同波动后仍然走强、利润正在改善且经营质量较好的股票，同时过滤特质波动和成交过热。适合先做研究实验，趋势反转期需要重点观察回撤",
    factors: [
      { id: "residual_momentum", weight: 1 },
      { id: "profit_growth", weight: 1 },
      { id: "gross_profitability", weight: 0.75 },
      { id: "idio_vol", weight: 0.75 },
      { id: "abnormal_turnover", weight: 0.5 },
    ],
  },
  {
    id: "low_vol",
    name: "低波动防守",
    desc: "只看股价涨跌是否平稳：挑最近一个月每天涨跌幅度小、扣掉大盘整体涨跌后自身波动也小、没有出现过单日暴涨的股票。不看公司大小，也不看财报",
    factors: [
      { id: "volatility", weight: 1 },
      { id: "idio_vol", weight: 1 },
      { id: "max_return", weight: 1 },
    ],
  },
  {
    id: "quality",
    name: "盈利质量",
    desc: "只看财报：挑用股东的钱赚钱效率高（最近一年的净利润 ÷ 净资产高）、最新一个季度利润比去年同期增长快、利润是实打实收到的现金的公司。财报每个季度才更新一次，持仓换得少",
    factors: [
      { id: "roe", weight: 1 },
      { id: "profit_growth", weight: 1 },
      { id: "accruals", weight: 1 },
    ],
  },
  {
    id: "mechanism_balanced",
    name: "机制分组均衡",
    desc: "低风险、盈利增长、现金质量和规模代理各占一组，避免低波动因子因有多个指标而重复占权",
    factors: [
      { id: "volatility", weight: 0.5 },
      { id: "idio_vol", weight: 0.5 },
      { id: "profit_growth", weight: 1 },
      { id: "accruals", weight: 1 },
      { id: "liquidity", weight: 1 },
    ],
  },
  {
    id: "reversal_quality",
    name: "反转 + 盈利质量",
    desc: "用短期反转和低特质波动过滤炒作，再加入低彩票特征、利润增长和现金质量",
    factors: [
      { id: "reversal", weight: 1 },
      { id: "idio_vol", weight: 1 },
      { id: "max_return", weight: 0.75 },
      { id: "profit_growth", weight: 1 },
      { id: "accruals", weight: 0.75 },
      { id: "liquidity", weight: 0.5 },
    ],
  },
  {
    id: "value_quality_risk",
    name: "价值 + 质量 + 风险",
    desc: "盈利收益率结合毛盈利、现金质量和低风险；估值数据不足的股票会自动缺失",
    factors: [
      { id: "earnings_yield", weight: 1 },
      { id: "gross_profitability", weight: 1 },
      { id: "accruals", weight: 1 },
      { id: "idio_vol", weight: 0.75 },
      { id: "residual_momentum", weight: 0.75 },
    ],
  },
  {
    id: "high_dividend_stable_low_price",
    name: "高股息 + 稳定盈利 + 低价",
    desc: "只研究 2 元以上、10 元以下的股票，优先选择过去 12 个月现金股息率高、持续盈利且利润波动较小的公司",
    max_price: 10,
    factors: [
      { id: "dividend_yield", weight: 1 },
      { id: "profit_stability", weight: 1 },
      { id: "accruals", weight: 0.5 },
      { id: "idio_vol", weight: 0.5 },
    ],
  },
];

/** 当前选择是否正好是某个预设，顺序无关 */
export function matchesPreset(picks: FactorWeight[], factors: readonly FactorWeight[]): boolean {
  return sameFactors(picks, factors);
}

/** 两组因子和权重是否相同，顺序无关 */
export function sameFactors(a: readonly FactorWeight[], b: readonly FactorWeight[]): boolean {
  return a.length === b.length && a.every((x) => b.some((y) => y.id === x.id && y.weight === x.weight));
}

/** 持有只数和缓冲区的说明：因子研究的组合回测和账户的调仓规则共用 */
export const TOP_N_HELP =
  "每次调仓买入合成得分排名最靠前的这么多只，每只大致占总资产的同样比例。只数越多，越接近因子本身的平均表现，单只股票的意外涨跌影响越小；只数越少，单只股票的分量越重";
export const BUFFER_HELP =
  "用来减少不必要的买卖。已经持有的股票，只要排名还在「持有只数 ×（1 + 缓冲区）」以内就继续拿着，掉出这个名次才卖；新买入的仍然只从前「持有只数」名里挑。例如持有 30 只、缓冲区 0.5：只买前 30 名，已持有的掉到第 45 名以后才卖。缓冲区为 0 时，掉出前 30 名就卖，每次调仓换得最多、交易成本最高；越大换得越少，但会多拿一些排名已经下滑的股票。可以填 0 到 2";

export interface FactorWeight {
  id: string;
  weight: number;
}

/** `reversal,idio_vol:0.5` → [{reversal, 1}, {idio_vol, 0.5}]；重复和空项去掉，权重不合法时按 1 */
export function parseFactors(s: string): FactorWeight[] {
  const out: FactorWeight[] = [];
  for (const part of s.split(",")) {
    const [id, w] = part.trim().split(":");
    if (!id || out.some((p) => p.id === id)) continue;
    const weight = w === undefined ? 1 : Number(w);
    out.push({ id, weight: Number.isFinite(weight) ? weight : 1 });
  }
  return out;
}

/** 反过来写成接口参数；权重为 1 时省略 */
export function formatFactors(picks: FactorWeight[], withWeights = true): string {
  return picks.map((p) => (withWeights && p.weight !== 1 ? `${p.id}:${p.weight}` : p.id)).join(",");
}

/** 勾选或取消一个因子，保持原有顺序和权重 */
export function toggleFactor(picks: FactorWeight[], id: string): FactorWeight[] {
  return picks.some((p) => p.id === id) ? picks.filter((p) => p.id !== id) : [...picks, { id, weight: 1 }];
}

/** 结论的颜色：显著且方向一致为绿，方向一致但不显著为黄，显著相反为红，其余灰 */
export function verdictTone(f: Pick<FactorTestStats, "significant" | "consistent" | "rank_ic_t">): Tone {
  if (f.significant) return "ok";
  const t = f.rank_ic_t ?? 0;
  if (t <= -3) return "bad";
  if (f.consistent && t >= 2) return "warn";
  return "muted";
}

/** 0 到 1 的概率按百分比显示，不带正号 */
export function fmtProb(v: number | null | undefined): string {
  if (v === null || v === undefined || !Number.isFinite(v)) return "—";
  return `${(v * 100).toFixed(0)}%`;
}

/** 普通小数，保留 digits 位 */
export function fmtNum(v: number | null | undefined, digits = 2): string {
  if (v === null || v === undefined || !Number.isFinite(v)) return "—";
  return v.toFixed(digits);
}

/** 多因子账户的规则换成因子研究页「组合回测」的链接：同样的因子、持有只数和缓冲区，每 20 个交易日调仓 */
export function planResearchLink(plan: {
  factors: { factor: string; weight: number }[];
  top_n: number;
  buffer: number;
}) {
  const f = formatFactors(plan.factors.map((x) => ({ id: x.factor, weight: x.weight })));
  const q = new URLSearchParams({
    tab: "portfolio",
    f,
    top: String(plan.top_n),
    buffer: String(plan.buffer),
    reb: "20",
    range: "5",
  });
  return `/research?${q.toString()}`;
}
