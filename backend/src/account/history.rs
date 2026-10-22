//! 账户买入前回看：只消费策略买卖规则，不改变策略或巡检结果。

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use super::rules::{HistoryCheck, HistoryWindow};
use crate::analysis::backtest::{self, Costs, LotRule, Risk, Signals};
use crate::analysis::indicators::{Frame, StockMeta};
use crate::analysis::prices::{CorpEvent, DailyBar};
use crate::analysis::strategy::Definition;

/// 与建议输入一起保存；百分比与账户门槛同单位，缺历史时不伪造为零收益。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct HistoryReturn {
    pub window: HistoryWindow,
    pub start: NaiveDate,
    pub end: NaiveDate,
    pub return_pct: Option<f64>,
}

pub fn passes(check: &HistoryCheck, returns: &[HistoryReturn], end: NaiveDate) -> bool {
    check.validate().is_ok()
        && check.windows.iter().all(|w| {
            returns.iter().any(|r| {
                r.window == *w
                    && r.end == end
                    && r.start == w.start(end)
                    && r.return_pct
                        .is_some_and(|v| v.is_finite() && v > check.min_return_pct)
            })
        })
}

/// 指标只算一次；账户回测逐日截断交易输入，复用同一只股票、同一策略的信号序列。
pub struct HistorySeries {
    entry: Vec<bool>,
    exit: Vec<bool>,
    risk: Risk,
    warmup: usize,
    list_date: Option<NaiveDate>,
    board: String,
    asset_type: crate::instrument::AssetType,
}

impl HistorySeries {
    pub fn new(bars: &[DailyBar], frame: &Frame<'_>, meta: &StockMeta, def: &Definition) -> Self {
        Self {
            entry: (0..bars.len())
                .map(|i| def.in_universe(frame, meta, i) && def.matches(frame, i))
                .collect(),
            exit: def.exit_series(frame),
            risk: Risk {
                stop_loss_pct: def.exit.stop_loss_pct,
                take_profit_pct: def.exit.take_profit_pct,
                trailing_stop_atr: def.exit.trailing_stop_atr,
                max_hold_days: def.exit.max_hold_days,
            },
            warmup: def.warmup() as usize,
            list_date: meta.list_date,
            board: meta.board.clone(),
            asset_type: meta.asset_type,
        }
    }

    // @project-doc docs/domains/accounts.md#history_check
    pub fn evaluate(
        &self,
        bars: &[DailyBar],
        events: &[CorpEvent],
        windows: &[HistoryWindow],
        end: NaiveDate,
    ) -> Vec<HistoryReturn> {
        // backtest::run 没有 end 参数，必须在调用边界裁掉未来日线和公司行动。
        let n = bars.partition_point(|b| b.date <= end);
        let bars = &bars[..n];
        let events = &events[..events.partition_point(|e| e.ex_date <= end)];
        windows
            .iter()
            .map(|&window| {
                let start = window.start(end);
                let first = bars.partition_point(|b| b.date < start);
                let enough = bars.first().is_some_and(|b| b.date <= start)
                    && bars.last().is_some_and(|b| b.date == end)
                    && self.list_date.is_none_or(|d| d <= start)
                    && first >= self.warmup;
                let return_pct = if enough {
                    backtest::run(
                        bars,
                        events,
                        &Signals {
                            entry: &self.entry[..n],
                            exit: &self.exit[..n],
                        },
                        &backtest::Params {
                            start: Some(start),
                            capital: 100_000.0,
                            costs: if crate::instrument::extension(self.asset_type).stamp_tax_rate()
                                == 0.0
                            {
                                Costs {
                                    stamp_tax_rate: 0.0,
                                    ..Costs::default()
                                }
                            } else {
                                Costs::default()
                            },
                            risk: self.risk,
                            lot: {
                                let (min, step) = crate::instrument::extension(self.asset_type)
                                    .lot_rule(&self.board);
                                LotRule { min, step }
                            },
                        },
                    )
                    .map(|o| o.summary.total_return * 100.0)
                    .filter(|v| v.is_finite())
                } else {
                    None
                };
                HistoryReturn {
                    window,
                    start,
                    end,
                    return_pct,
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_selected_windows_must_pass_strict_percentage_threshold() {
        let end = NaiveDate::from_ymd_opt(2026, 10, 16).unwrap();
        let check = HistoryCheck {
            windows: vec![HistoryWindow::OneYear, HistoryWindow::ThreeYears],
            min_return_pct: 0.0,
        };
        let returns = vec![
            HistoryReturn {
                window: HistoryWindow::OneYear,
                start: HistoryWindow::OneYear.start(end),
                end,
                return_pct: Some(0.01),
            },
            HistoryReturn {
                window: HistoryWindow::ThreeYears,
                start: HistoryWindow::ThreeYears.start(end),
                end,
                return_pct: Some(0.0),
            },
        ];
        assert!(!passes(&check, &returns, end));
        let mut passing = returns;
        passing[1].return_pct = Some(0.01);
        assert!(passes(&check, &passing, end));
    }
}
