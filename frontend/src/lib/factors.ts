/** 因子研究页的纯函数：选中的因子和权重在 URL 里的写法、结论的颜色。规则见 docs/domains/factor_research.md */
import type { FactorTestStats } from "../api/generated/model";
import type { Tone } from "./labels";

/** 默认选中的因子：A 股文献里较稳健的价量因子，不需要财务数据 */
export const DEFAULT_FACTORS = "reversal,idio_vol,max_return,liquidity,abnormal_turnover";

/** 预设的多因子组合：点一下替换当前选择，权重都是 1（按各因子的预期方向合成）。
 * 只写思路，不写回测数字；组合 E 与后端 `Rules::default` 相同 */
export const FACTOR_PRESETS: readonly { id: string; name: string; desc: string; factors: readonly string[] }[] = [
  {
    id: "e",
    name: "低波动 + 盈利质量 + 小盘",
    desc: "账户的默认规则（研究记录里的组合 E）：挑波动小、利润在增长、利润里现金含量高的小公司",
    factors: ["volatility", "idio_vol", "profit_growth", "accruals", "liquidity"],
  },
  {
    id: "price",
    name: "短期价量",
    desc: "只用日线：最近跌得多、走势平稳、没有暴涨、成交没有突然放大的小盘股。换手较高",
    factors: ["reversal", "idio_vol", "max_return", "liquidity", "abnormal_turnover"],
  },
  {
    id: "price_fundamental",
    name: "价量 + 财务",
    desc: "研究记录里的组合 B：价量因子避开追涨和过热，再加两个换手低的财务因子",
    factors: ["momentum", "idio_vol", "max_return", "abnormal_turnover", "profit_growth", "accruals"],
  },
  {
    id: "low_vol",
    name: "低波动防守",
    desc: "只挑走势平稳、没有暴涨的股票，不看公司大小",
    factors: ["volatility", "idio_vol", "max_return"],
  },
  {
    id: "quality",
    name: "盈利质量",
    desc: "只看财报：净资产收益率高、利润在增长、利润里现金含量高。调仓少",
    factors: ["roe", "profit_growth", "accruals"],
  },
];

/** 当前选择是否正好是某个预设（同样的因子，权重都是 1） */
export function matchesPreset(picks: FactorWeight[], factors: readonly string[]): boolean {
  return picks.length === factors.length && picks.every((p) => p.weight === 1 && factors.includes(p.id));
}

/** 两组因子和权重是否相同，顺序无关 */
export function sameFactors(a: FactorWeight[], b: FactorWeight[]): boolean {
  return a.length === b.length && a.every((x) => b.some((y) => y.id === x.id && y.weight === x.weight));
}

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
