/** 展示用的数字和日期格式。后端价格是十进制字符串，见 docs/interfaces/rest_api.md#conventions */

export function toNumber(v: string | number | null | undefined): number | null {
  if (v === null || v === undefined || v === "") return null;
  const n = typeof v === "number" ? v : Number(v);
  return Number.isFinite(n) ? n : null;
}

export function fmtPrice(v: string | number | null | undefined, digits = 2): string {
  const n = toNumber(v);
  return n === null ? "—" : n.toFixed(digits);
}

/** 大数用「万」「亿」表示，例如成交额 7,537,706,223.71 → 75.38 亿 */
export function fmtBig(v: string | number | null | undefined): string {
  const n = toNumber(v);
  if (n === null) return "—";
  const abs = Math.abs(n);
  if (abs >= 1e12) return `${(n / 1e12).toFixed(2)} 万亿`;
  if (abs >= 1e8) return `${(n / 1e8).toFixed(2)} 亿`;
  if (abs >= 1e4) return `${(n / 1e4).toFixed(2)} 万`;
  return n.toFixed(0);
}

export function fmtInt(v: number | null | undefined): string {
  return v === null || v === undefined ? "—" : v.toLocaleString("zh-CN");
}

export function fmtPercent(v: number | null | undefined, digits = 2): string {
  if (v === null || v === undefined || !Number.isFinite(v)) return "—";
  const s = (v * 100).toFixed(digits);
  return `${v > 0 ? "+" : ""}${s}%`;
}

/** UTC 时间串按上海时间展示到分钟 */
export function fmtDateTime(iso: string | null | undefined): string {
  if (!iso) return "—";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return d.toLocaleString("zh-CN", {
    timeZone: "Asia/Shanghai",
    hour12: false,
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  });
}

/** 时刻按上海时间展示成「10-19 周一 18:00」，用于下次同步这类近期的时间 */
export function fmtSlot(iso: string | null | undefined): string {
  if (!iso) return "—";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  const p = Object.fromEntries(
    new Intl.DateTimeFormat("zh-CN", {
      timeZone: "Asia/Shanghai",
      hour12: false,
      month: "2-digit",
      day: "2-digit",
      weekday: "short",
      hour: "2-digit",
      minute: "2-digit",
    })
      .formatToParts(d)
      .map((x) => [x.type, x.value]),
  );
  return `${p.month}-${p.day} ${p.weekday} ${p.hour}:${p.minute}`;
}

/** 相对现在过了多久，例如「3 分钟前」 */
export function fmtAgo(iso: string | null | undefined, now = Date.now()): string {
  if (!iso) return "—";
  const t = new Date(iso).getTime();
  if (Number.isNaN(t)) return "—";
  const s = Math.max(0, Math.round((now - t) / 1000));
  if (s < 60) return "刚刚";
  if (s < 3600) return `${Math.floor(s / 60)} 分钟前`;
  if (s < 86400) return `${Math.floor(s / 3600)} 小时前`;
  return `${Math.floor(s / 86400)} 天前`;
}

/** 两个时间之间的时长，例如「1 小时 5 分」 */
export function fmtDuration(fromIso: string, toIso?: string | null, now = Date.now()): string {
  const ms = (toIso ? new Date(toIso).getTime() : now) - new Date(fromIso).getTime();
  if (!Number.isFinite(ms) || ms < 0) return "—";
  const m = Math.floor(ms / 60000);
  if (m < 1) return `${Math.max(1, Math.round(ms / 1000))} 秒`;
  if (m < 60) return `${m} 分`;
  const h = Math.floor(m / 60);
  if (h < 48) return `${h} 小时 ${m % 60} 分`;
  return `${Math.floor(h / 24)} 天 ${h % 24} 小时`;
}

/** 上海时区的今天，yyyy-MM-dd */
const WEEK = ["周日", "周一", "周二", "周三", "周四", "周五", "周六"];

/** 交易日的短写法：2026-10-19 → 10-19（周一） */
export function dayLabel(d: string): string {
  return `${d.slice(5)}（${WEEK[new Date(`${d}T00:00:00Z`).getUTCDay()]}）`;
}

export function todayCn(now = new Date()): string {
  return now.toLocaleDateString("sv-SE", { timeZone: "Asia/Shanghai" });
}

/** yyyy-MM-dd 往前推 years 年 */
export function yearsBefore(date: string, years: number): string {
  const [y, m, d] = date.split("-").map(Number) as [number, number, number];
  const dt = new Date(Date.UTC(y - years, m - 1, d));
  return dt.toISOString().slice(0, 10);
}

/** 涨跌的文字颜色：上涨红、下跌绿（A 股习惯），没有数值时不着色 */
export function upDownClass(v: number | null | undefined): string {
  return v === null || v === undefined ? "" : v >= 0 ? "text-up" : "text-down";
}

/** yyyy-MM-dd 往前推 months 个月；目标月份没有这一天时取月末 */
export function monthsBefore(date: string, months: number): string {
  const [y, m, d] = date.split("-").map(Number) as [number, number, number];
  const first = new Date(Date.UTC(y, m - 1 - months, 1));
  const last = new Date(Date.UTC(first.getUTCFullYear(), first.getUTCMonth() + 1, 0)).getUTCDate();
  return new Date(Date.UTC(first.getUTCFullYear(), first.getUTCMonth(), Math.min(d, last))).toISOString().slice(0, 10);
}

/** K 线图的时间区间：月和年两种粒度；「自定义」由 RangePicker 另外处理 */
export const CHART_RANGES = [
  { value: "1m", label: "1 月" },
  { value: "3m", label: "3 月" },
  { value: "6m", label: "6 月" },
  { value: "1", label: "1 年" },
  { value: "3", label: "3 年" },
  { value: "5", label: "5 年" },
  { value: "10", label: "10 年" },
  { value: "all", label: "全部" },
] as const;

/** 回测的时间区间（按年） */
export const RANGES = [
  { value: "1", label: "1 年" },
  { value: "3", label: "3 年" },
  { value: "5", label: "5 年" },
  { value: "10", label: "10 年" },
  { value: "all", label: "全部" },
] as const;

/** 规则回测的时间区间：默认今年以来 */
export const BACKTEST_RANGES = [
  { value: "3m", label: "近 3 个月" },
  { value: "6m", label: "近 6 个月" },
  { value: "1", label: "近一年" },
  { value: "ytd", label: "今年以来" },
  { value: "2", label: "近两年" },
  { value: "3", label: "近 3 年" },
] as const;

/** 区间对应的开始日期：「1m」是 1 个月，「3」是 3 年，「ytd」是今年 1 月 1 日；「全部」和「自定义」在这里不限制 */
export function rangeStart(range: string, today = todayCn()): string | undefined {
  if (range === "all" || range === "custom") return undefined;
  if (range === "ytd") return `${today.slice(0, 4)}-01-01`;
  if (range.endsWith("m")) return monthsBefore(today, Number(range.slice(0, -1)));
  return yearsBefore(today, Number(range));
}

/** 已经是百分数的值（5 表示 5%），例如策略接口里的涨跌幅 */
export function fmtPctValue(v: number | null | undefined, digits = 2): string {
  if (v === null || v === undefined || !Number.isFinite(v)) return "—";
  return `${v > 0 ? "+" : ""}${v.toFixed(digits)}%`;
}
