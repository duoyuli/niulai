/** K 线上的均线：周期的取值范围、默认值、计算，以及为了让均线从区间第一天就有值要往前多取多少天。
 * 规则见 docs/architecture/frontend.md#conventions */

/** 默认的快线和慢线（交易日） */
export const DEFAULT_MA = { ma1: "15", ma2: "30" };
export const MA_MIN = 2;
export const MA_MAX = 250;

/** URL 里的周期换成数字；不合法时用默认值 */
export function maPeriod(v: string, fallback: string): number {
  const n = Number(v);
  return Number.isInteger(n) && n >= MA_MIN && n <= MA_MAX ? n : Number(fallback);
}

/** 简单移动平均：第 i 个值是到第 i 天为止 n 个收盘价的平均，不满 n 天时为空 */
export function movingAverage(closes: number[], n: number): (number | null)[] {
  const out: (number | null)[] = [];
  let sum = 0;
  closes.forEach((c, i) => {
    sum += c;
    if (i >= n) sum -= closes[i - n] ?? 0;
    out.push(i >= n - 1 ? sum / n : null);
  });
  return out;
}

/** 算 n 日均线要往前多取的自然日：交易日约占自然日的 70%，再留出长假的余量 */
export function warmupDays(n: number): number {
  return Math.ceil(n * 1.6) + 15;
}

/** yyyy-MM-dd 往前推 days 天 */
export function daysBefore(date: string, days: number): string {
  const [y, m, d] = date.split("-").map(Number) as [number, number, number];
  return new Date(Date.UTC(y, m - 1, d - days)).toISOString().slice(0, 10);
}

/** 请求日线时用的开始日期：区间开始日再往前多取，够算最长的那条均线；不限开始日（「全部」）时不变 */
export function fetchStart(start: string | undefined, periods: number[]): string | undefined {
  return start && daysBefore(start, warmupDays(Math.max(...periods)));
}

/** 快线和慢线的交叉：快线从下方上穿慢线（金叉）记为 buy，从上方下穿（死叉）记为 sell。
 * 前一天快线 ≤ 慢线、当天快线 > 慢线算上穿；两条线有一条为空的日子不算 */
export function crosses(fast: (number | null)[], slow: (number | null)[]): ("buy" | "sell" | null)[] {
  return fast.map((f, i) => {
    const s = slow[i];
    const pf = fast[i - 1];
    const ps = slow[i - 1];
    if (f == null || s == null || pf == null || ps == null) return null;
    if (pf <= ps && f > s) return "buy";
    if (pf >= ps && f < s) return "sell";
    return null;
  });
}
