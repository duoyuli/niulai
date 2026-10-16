/** 后端枚举值的中文名 */

export const BOARDS: Record<string, string> = {
  main: "主板",
  chinext: "创业板",
  star: "科创板",
  bse: "北交所",
};

export const INDEX_CATEGORIES: Record<string, string> = {
  exchange: "交易所指数",
  industry: "同花顺行业",
  cn_concept: "同花顺概念",
  region: "地域",
  tszs: "同花顺特色",
  ths: "同花顺指数",
  unknown: "其他",
};

export const ADJUSTS = [
  { value: "qfq", label: "等比前复权", hint: "等比前复权：最新价不变，历史价按除权除息调整，均线和信号都用这个口径" },
  { value: "none", label: "不复权", hint: "交易所实际成交价，除权日会出现跳空缺口" },
  { value: "hfq", label: "等比后复权", hint: "等比后复权：上市首日价不变，适合看长期真实涨幅" },
  {
    value: "qfq_ths",
    label: "同花顺前复权",
    hint: "同花顺的减法复权口径（历史价减去分红），只用于和同花顺软件对账；早年价格可能为负，不适合看涨跌和算指标",
  },
  { value: "hfq_ths", label: "同花顺后复权", hint: "同花顺的减法后复权口径，只用于和同花顺软件对账" },
] as const;

export type Adjust = (typeof ADJUSTS)[number]["value"];

export const RUN_STATUS: Record<string, { label: string; tone: Tone }> = {
  running: { label: "进行中", tone: "info" },
  succeeded: { label: "成功", tone: "ok" },
  partial: { label: "部分失败", tone: "warn" },
  failed: { label: "失败", tone: "bad" },
  cancelled: { label: "已取消", tone: "muted" },
};

export const TASK_STATUS: Record<string, { label: string; tone: Tone }> = {
  pending: { label: "等待", tone: "muted" },
  running: { label: "执行中", tone: "info" },
  done: { label: "完成", tone: "ok" },
  empty: { label: "无数据", tone: "muted" },
  skipped: { label: "跳过", tone: "muted" },
  failed: { label: "待重试", tone: "warn" },
  dead: { label: "失败", tone: "bad" },
};

export const RUN_KINDS: Record<string, string> = {
  backfill: "历史回补",
  incremental: "增量",
};

export const PRIORITIES: Record<string, string> = {
  P0: "P0 核心",
  P1: "P1 重要",
  P2: "P2 补充",
  P3: "P3 可选",
};

export type Tone = "ok" | "warn" | "bad" | "info" | "muted";
