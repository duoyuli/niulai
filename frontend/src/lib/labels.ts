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
  backfill: "补历史数据",
  incremental: "定时同步",
  repair: "补齐缺口",
};

/** 数据同步页的分类顺序，与 datasets.yaml 的 category 一致 */
export const DATASET_CATEGORIES = [
  "基础信息",
  "行情",
  "公司行动",
  "估值与市值",
  "财务",
  "指数与板块",
  "涨跌停",
  "龙虎榜",
  "热点与异动",
  "新股",
  "基金",
  "期货",
  "期权",
  "港股新股",
];

/** 更新频率 */
/** 同步计划，与后端 scheduler::Plan 一致 */
export const PLANS: Record<string, string> = {
  daily: "每日收盘后",
  weekly: "每周",
  monthly: "每月",
  report_season: "财报季",
  manual: "不自动同步",
};

export const WEEKDAYS = ["周一", "周二", "周三", "周四", "周五", "周六", "周日"];

/** 一次同步是谁发起的：看运行的 params.trigger；以前的运行没有记，按类型显示 */
export function runSource(kind: string, params: unknown): string {
  const trigger = (params as { trigger?: string | null } | null)?.trigger;
  switch (trigger) {
    case "daily":
      return "每日收盘后";
    case "catch_up":
      return "补发每日同步";
    case "weekly":
      return "每周";
    case "weekly+monthly":
      return "每周 + 每月";
    case "manual":
      return kind === "backfill" ? "手动回补" : "手动同步";
    case "setup":
      return "首次补历史";
    case "cli":
      return "命令行";
    case "repair":
      return "补齐缺口";
    default:
      return RUN_KINDS[kind] ?? kind;
  }
}

export const PRIORITIES: Record<string, string> = {
  P0: "P0 核心",
  P1: "P1 重要",
  P2: "P2 补充",
  P3: "P3 可选",
};

export type Tone = "ok" | "warn" | "bad" | "info" | "muted";
