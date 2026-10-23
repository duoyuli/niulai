/** 因子研究页的纯函数：选中的因子和权重在 URL 里的写法、结论的颜色。规则见 docs/domains/factor_research.md */
import type { FactorTestStats } from "../api/generated/model";
import type { Tone } from "./labels";

/** 默认选中的因子：A 股文献里较稳健的价量因子，不需要财务数据 */
export const DEFAULT_FACTORS = "reversal,idio_vol,max_return,liquidity,abnormal_turnover";

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
