import type { Tone } from "../api/generated/model";

/** 信号偏多、偏空、中性的展示：偏多红色向上、偏空绿色向下（与 A 股红涨绿跌一致），中性（十字星、缩量等本身不说明方向）橙色圆点 */
export interface ToneStyle {
  arrow: string;
  /** 偏多 / 偏空 / 中性 */
  word: string;
  /** 详情页的一句话说明 */
  explain: string;
  text: string;
  chip: string;
  color: string;
  marker: { position: "above" | "below"; shape: "arrowUp" | "arrowDown" | "circle"; word: string };
}

export const TONES: Record<Tone, ToneStyle> = {
  bullish: {
    arrow: "▲",
    word: "偏多",
    explain: "偏多：常被看作走强",
    text: "text-up",
    chip: "bg-up/10 text-up",
    color: "#d93a3a",
    marker: { position: "below", shape: "arrowUp", word: "红色向上箭头" },
  },
  bearish: {
    arrow: "▼",
    word: "偏空",
    explain: "偏空：常被看作走弱",
    text: "text-down",
    chip: "bg-down/10 text-down",
    color: "#16a05d",
    marker: { position: "above", shape: "arrowDown", word: "绿色向下箭头" },
  },
  neutral: {
    arrow: "◆",
    word: "中性",
    explain: "中性：本身不说明方向，要结合位置和其他信号看",
    text: "text-amber-600 dark:text-amber-400",
    chip: "bg-amber-500/10 text-amber-700 dark:text-amber-400",
    color: "#d97706",
    marker: { position: "above", shape: "circle", word: "橙色圆点" },
  },
};

export function toneOf(tone: Tone | undefined): ToneStyle {
  return TONES[tone ?? "bullish"];
}
