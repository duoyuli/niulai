//! 个股诊断：用几句白话说明一只股票最近的状态——趋势、动量、量能、位置各一句结论，
//! 最近出现过的信号，几个关键价位，以及当天符合哪些策略。规则见 docs/domains/strategies.md#diagnosis。

use chrono::NaiveDate;
use serde::Serialize;
use utoipa::ToSchema;

use super::indicators::{Frame, IndicatorId, IndicatorRef, StockMeta};
use super::limit_pool::LimitPool;
use super::prices::DailyBar;
use super::signals::{self, Params, SignalKind, Tone};
use super::strategy::Definition;

/// 最近信号看多少个交易日
pub const RECENT_DAYS: usize = 20;
/// 诊断需要的日线根数：覆盖每个信号的预热（最多的是 MACD 背离，约 190 根）再加最近 20 天
pub const BARS_NEEDED: i64 = 300;

#[derive(Serialize, ToSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Strong,
    Neutral,
    Weak,
}

#[derive(Serialize, ToSchema)]
pub struct Dimension {
    /// trend / momentum / volume / position
    pub key: &'static str,
    pub title: &'static str,
    pub verdict: Verdict,
    /// 一句结论
    pub headline: String,
    /// 补充说明
    pub detail: String,
}

#[derive(Serialize, ToSchema, Clone)]
pub struct SignalEvent {
    pub trade_date: NaiveDate,
    pub signal: signals::SignalId,
    pub name: String,
    pub tone: Tone,
}

#[derive(Serialize, ToSchema)]
pub struct Level {
    pub name: &'static str,
    pub price: f64,
    /// support：在当前价下方；resistance：在上方
    pub kind: &'static str,
    /// 相对收盘价（%），正数在上方
    pub distance_pct: f64,
}

#[derive(Serialize, ToSchema)]
pub struct StrategyMatch {
    pub strategy_id: i64,
    pub name: String,
    pub reasons: Vec<String>,
}

#[derive(Serialize, ToSchema)]
pub struct Diagnosis {
    pub trade_date: NaiveDate,
    /// 收盘价（未复权）
    pub close: f64,
    pub dimensions: Vec<Dimension>,
    /// 最近 20 个交易日出现过的事件类信号，新的在前
    pub recent_signals: Vec<SignalEvent>,
    pub levels: Vec<Level>,
    /// 当天满足的策略（按股票池和全部条件判断）
    pub strategies: Vec<StrategyMatch>,
}

fn pct(v: f64) -> String {
    if v.is_nan() {
        "—".into()
    } else {
        format!("{}{:.1}%", if v > 0.0 { "+" } else { "" }, v)
    }
}

/// 最近 days 个交易日内出现过的事件类信号（每个信号用默认参数），新的在前。
/// 中性的信号（十字星、缩量）很常见、本身不说明方向，不列出
pub fn recent_signals(f: &Frame, days: usize) -> Vec<SignalEvent> {
    let n = f.len();
    let from = n.saturating_sub(days);
    let mut out = vec![];
    for s in signals::CATALOG
        .iter()
        .filter(|s| s.kind == SignalKind::Event && s.tone != signals::Tone::Neutral)
    {
        let params = Params::new();
        let series = signals::series(f, s.id, &params);
        for i in from..n {
            if series[i] {
                out.push(SignalEvent {
                    trade_date: f.bars[i].date,
                    signal: s.id,
                    name: signals::label(s.id, &params),
                    tone: s.tone,
                });
            }
        }
    }
    out.sort_by_key(|e| std::cmp::Reverse(e.trade_date));
    out
}

// @project-doc docs/domains/strategies.md#diagnosis
/// authority：权威涨跌停池和这只股票的代码；有它时最近的涨跌停信号以供应商的名单为准
pub fn diagnose(
    bars: &[DailyBar],
    meta: &StockMeta,
    strategies: &[(i64, String, Definition)],
    authority: Option<(&LimitPool, &str)>,
) -> Option<Diagnosis> {
    let last = bars.last()?;
    let f = match authority {
        Some((pool, code)) => Frame::with_authority(bars, Some(meta), pool, code),
        None => Frame::new(bars, Some(meta)),
    };
    let i = bars.len() - 1;
    let v = |id, n| f.at(IndicatorRef::new(id, n), i);
    let close = last.q_close;
    let (ma5, ma20, ma60) = (
        v(IndicatorId::Ma, Some(5)),
        v(IndicatorId::Ma, Some(20)),
        v(IndicatorId::Ma, Some(60)),
    );

    let trend = if ma60.is_nan() {
        Dimension {
            key: "trend",
            title: "趋势",
            verdict: Verdict::Neutral,
            headline: "日线不足 60 根，暂时判断不了趋势".into(),
            detail: String::new(),
        }
    } else {
        let gap = (close / ma60 - 1.0) * 100.0;
        let side = if gap >= 0.0 { "上方" } else { "下方" };
        let detail = format!("收盘价在 60 日均线{side} {:.1}%", gap.abs());
        if ma5 > ma20 && ma20 > ma60 {
            Dimension {
                key: "trend",
                title: "趋势",
                verdict: Verdict::Strong,
                headline: "上升趋势：5、20、60 日均线多头排列".into(),
                detail,
            }
        } else if ma5 < ma20 && ma20 < ma60 {
            Dimension {
                key: "trend",
                title: "趋势",
                verdict: Verdict::Weak,
                headline: "下降趋势：5、20、60 日均线空头排列".into(),
                detail,
            }
        } else {
            Dimension {
                key: "trend",
                title: "趋势",
                verdict: Verdict::Neutral,
                headline: "均线交织，趋势不明显".into(),
                detail,
            }
        }
    };

    let m20 = v(IndicatorId::Momentum, Some(20));
    let rsi = v(IndicatorId::Rsi, Some(14));
    let rsi_text = if rsi.is_nan() {
        String::new()
    } else if rsi >= 70.0 {
        format!("RSI（14）为 {rsi:.0}，进入超买区，短期涨得较多")
    } else if rsi <= 30.0 {
        format!("RSI（14）为 {rsi:.0}，进入超卖区，短期跌得较多")
    } else {
        format!("RSI（14）为 {rsi:.0}，处在正常区间")
    };
    let momentum = Dimension {
        key: "momentum",
        title: "动量",
        verdict: if m20 >= 10.0 {
            Verdict::Strong
        } else if m20 <= -10.0 {
            Verdict::Weak
        } else {
            Verdict::Neutral
        },
        headline: format!(
            "近 20 个交易日{}",
            if m20.is_nan() {
                "数据不足".into()
            } else {
                format!("涨跌 {}", pct(m20))
            }
        ),
        detail: rsi_text,
    };

    let vr = v(IndicatorId::VolRatio, Some(5));
    let ch = v(IndicatorId::ChangePct, None);
    let volume = if vr.is_nan() {
        Dimension {
            key: "volume",
            title: "量能",
            verdict: Verdict::Neutral,
            headline: "数据不足".into(),
            detail: String::new(),
        }
    } else if vr >= 1.5 {
        let verdict = if ch > 0.0 {
            Verdict::Strong
        } else if ch < 0.0 {
            Verdict::Weak
        } else {
            Verdict::Neutral
        };
        let mood = if ch > 0.0 {
            "放量上涨，买的人明显增多"
        } else if ch < 0.0 {
            "放量下跌，卖压较重"
        } else {
            "放量但价格没动"
        };
        Dimension {
            key: "volume",
            title: "量能",
            verdict,
            headline: mood.into(),
            detail: format!("成交量是前 5 个交易日平均的 {vr:.1} 倍"),
        }
    } else if vr <= 0.7 {
        Dimension {
            key: "volume",
            title: "量能",
            verdict: Verdict::Neutral,
            headline: "缩量，交易清淡".into(),
            detail: format!("成交量只有前 5 个交易日平均的 {vr:.1} 倍"),
        }
    } else {
        Dimension {
            key: "volume",
            title: "量能",
            verdict: Verdict::Neutral,
            headline: "量能平稳".into(),
            detail: format!("成交量是前 5 个交易日平均的 {vr:.1} 倍"),
        }
    };

    let (hi, lo) = (
        v(IndicatorId::HighN, Some(60)),
        v(IndicatorId::LowN, Some(60)),
    );
    let position = if hi.is_nan() || lo.is_nan() || hi <= lo {
        Dimension {
            key: "position",
            title: "位置",
            verdict: Verdict::Neutral,
            headline: "日线不足 60 根".into(),
            detail: String::new(),
        }
    } else {
        let p = (close - lo) / (hi - lo);
        let detail = format!(
            "在近 60 个交易日的价格区间中处于 {:.0}% 的位置，距最高价 {}",
            p * 100.0,
            pct((close / hi - 1.0) * 100.0)
        );
        let (verdict, headline) = if p >= 0.8 {
            (Verdict::Strong, "接近 60 日高点")
        } else if p <= 0.2 {
            (Verdict::Weak, "接近 60 日低点")
        } else {
            (Verdict::Neutral, "处在 60 日区间中部")
        };
        Dimension {
            key: "position",
            title: "位置",
            verdict,
            headline: headline.into(),
            detail,
        }
    };

    let mut levels = vec![];
    let mut level = |name: &'static str, price: f64| {
        if price.is_finite() && price > 0.0 {
            levels.push(Level {
                name,
                price,
                kind: if price >= close {
                    "resistance"
                } else {
                    "support"
                },
                distance_pct: (price / close - 1.0) * 100.0,
            });
        }
    };
    level("60 日最高价", hi);
    level("60 日最低价", lo);
    level("20 日均线", ma20);
    level("60 日均线", ma60);
    level("布林带上轨", v(IndicatorId::BollUpper, None));
    level("布林带下轨", v(IndicatorId::BollLower, None));
    levels.sort_by(|a, b| b.price.total_cmp(&a.price));

    let strategies = strategies
        .iter()
        .filter(|(_, _, d)| d.in_universe(&f, meta, i))
        .filter_map(|(id, name, d)| {
            let e = d.eval(&f, i);
            e.matched.then(|| StrategyMatch {
                strategy_id: *id,
                name: name.clone(),
                reasons: e.reasons,
            })
        })
        .collect();

    let mut recent = recent_signals(&f, RECENT_DAYS);
    recent.truncate(15);
    Some(Diagnosis {
        trade_date: last.date,
        close: last.close,
        dimensions: vec![trend, momentum, volume, position],
        recent_signals: recent,
        levels,
        strategies,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::indicators::tests::bars_from;

    #[test]
    fn enough_history_for_every_signal() {
        let most = signals::CATALOG
            .iter()
            .map(|s| signals::warmup(s.id, &Params::new()))
            .max()
            .unwrap();
        assert!(
            most as i64 + RECENT_DAYS as i64 <= BARS_NEEDED,
            "诊断要读 {} 根",
            most as usize + RECENT_DAYS
        );
    }

    #[test]
    fn rising_stock_is_diagnosed_as_uptrend() {
        let closes: Vec<f64> = (0..120).map(|i| 10.0 * 1.01f64.powi(i)).collect();
        let b = bars_from(&closes);
        let meta = StockMeta {
            board: "main".into(),
            is_st: false,
            list_date: None,
            asset_type: crate::analysis::indicators::AssetType::Stock,
        };
        let d = diagnose(&b, &meta, &[], None).unwrap();
        assert_eq!(
            d.dimensions[0].verdict,
            Verdict::Strong,
            "{}",
            d.dimensions[0].headline
        );
        assert_eq!(d.dimensions[3].headline, "接近 60 日高点");
        assert!(
            d.levels
                .iter()
                .any(|l| l.name == "20 日均线" && l.kind == "support")
        );
    }
}
