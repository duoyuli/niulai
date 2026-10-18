//! 从券商 App 复制的持仓文字里认出每只股票的代码、股数和成本价。只是预览，用户确认后才保存。
//!
//! 券商的格式各不相同，按行宽松地认：一行里有 6 位代码（`600519`、`600519.SH`、`SH600519` 都行），
//! 没有代码时找这一行里出现的股票简称；代码之外第一个整数是股数，第一个带小数点的数是成本价。
//! 没有代码也没有简称的行（表头、合计）跳过；有 6 位代码却不是本地的 A 股个股（ETF、可转债等）的行照样返回，
//! 写明为什么不记，不悄悄丢掉。规则见 docs/domains/accounts.md#positions。

use serde::Serialize;
use utoipa::ToSchema;

/// 认出的一行
#[derive(Serialize, ToSchema, Clone, Debug, PartialEq)]
pub struct ParsedRow {
    /// 原文
    pub line: String,
    pub thscode: Option<String>,
    pub name: Option<String>,
    pub shares: Option<i64>,
    pub cost_price: Option<f64>,
    /// 认不全时的说明，例如「没有找到股数」；为空表示可以直接保存
    pub problem: Option<String>,
}

/// 一只股票：代码、6 位数字代码、简称
pub struct Known<'a> {
    pub thscode: &'a str,
    pub ticker: &'a str,
    pub name: &'a str,
}

pub fn parse(text: &str, known: &[Known]) -> Vec<ParsedRow> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .filter_map(|line| parse_line(line, known))
        .collect()
}

fn parse_line(line: &str, known: &[Known]) -> Option<ParsedRow> {
    let cleaned = drop_thousands_separators(line);
    let tokens: Vec<&str> = cleaned
        .split(|c: char| c.is_whitespace() || c == ',' || c == '，' || c == '|' || c == ';')
        .filter(|t| !t.is_empty())
        .collect();
    // 先找代码
    let mut code_token = None;
    let mut stock = None;
    for (i, t) in tokens.iter().enumerate() {
        if let Some(ticker) = ticker_of(t)
            && let Some(k) = known.iter().find(|k| k.ticker == ticker)
        {
            code_token = Some(i);
            stock = Some(k);
            break;
        }
    }
    // 没有代码时按简称找，取最长的匹配（「中国平安」不要匹配成「平安」）
    if stock.is_none() {
        stock = known
            .iter()
            .filter(|k| k.name.chars().count() >= 2 && line.contains(k.name))
            .max_by_key(|k| k.name.chars().count());
    }
    let Some(stock) = stock else {
        // 只认像代码的写法（6 位数字，或带交易所前后缀），金额「123456.78」不算
        let code = tokens
            .iter()
            .filter(|t| t.len() == 6 || t.chars().any(|c| c.is_ascii_alphabetic()))
            .find_map(|t| ticker_of(t))?;
        return Some(ParsedRow {
            line: line.to_string(),
            thscode: None,
            name: None,
            shares: None,
            cost_price: None,
            problem: Some(format!(
                "{code} 不是本地有的 A 股个股（ETF、可转债、港股通等不记），这一行不加"
            )),
        });
    };
    let mut shares = None;
    let mut cost = None;
    for (i, t) in tokens.iter().enumerate() {
        if Some(i) == code_token {
            continue;
        }
        let clean: String = t
            .chars()
            .filter(|c| c.is_ascii_digit() || *c == '.' || *c == '-')
            .collect();
        if clean.is_empty()
            || clean.len() + 2 < t.chars().filter(|c| !c.is_ascii_punctuation()).count()
        {
            continue; // 不是数字（例如简称）
        }
        if clean.contains('.') {
            if cost.is_none()
                && let Ok(v) = clean.parse::<f64>()
                && v > 0.0
            {
                cost = Some(v);
            }
        } else if shares.is_none()
            && let Ok(v) = clean.parse::<i64>()
            && v > 0
        {
            shares = Some(v);
        }
    }
    let problem = if shares.is_none() {
        Some("没有找到股数".to_string())
    } else {
        None
    };
    Some(ParsedRow {
        line: line.to_string(),
        thscode: Some(stock.thscode.to_string()),
        name: Some(stock.name.to_string()),
        shares,
        cost_price: cost,
        problem,
    })
}

/// 去掉数字中的千位分隔符：`1,498.00` → `1498.00`；其余逗号仍是分隔符
fn drop_thousands_separators(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::with_capacity(line.len());
    for (i, &c) in chars.iter().enumerate() {
        let between_digits = i > 0
            && chars[i - 1].is_ascii_digit()
            && chars
                .get(i + 1..i + 4)
                .is_some_and(|w| w.iter().all(char::is_ascii_digit));
        if c == ',' && between_digits {
            continue;
        }
        out.push(c);
    }
    out
}

/// 6 位数字代码：`600519`、`600519.SH`、`SH600519`、`sz000001`
fn ticker_of(t: &str) -> Option<String> {
    let upper = t.to_ascii_uppercase();
    let core = upper
        .trim_start_matches("SH")
        .trim_start_matches("SZ")
        .trim_start_matches("BJ");
    let core = core.split('.').next().unwrap_or(core);
    (core.len() == 6 && core.chars().all(|c| c.is_ascii_digit())).then(|| core.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn known() -> Vec<Known<'static>> {
        vec![
            Known {
                thscode: "600519.SH",
                ticker: "600519",
                name: "贵州茅台",
            },
            Known {
                thscode: "000858.SZ",
                ticker: "000858",
                name: "五粮液",
            },
            Known {
                thscode: "601318.SH",
                ticker: "601318",
                name: "中国平安",
            },
            Known {
                thscode: "000001.SZ",
                ticker: "000001",
                name: "平安银行",
            },
        ]
    }

    #[test]
    fn reads_common_broker_layouts() {
        let text = "证券代码 证券名称 股票余额 可用余额 成本价 市价\n\
                    600519 贵州茅台 100 100 1,498.000 1680.00\n\
                    SZ000858\t五粮液\t200\t200\t142.50\n\
                    中国平安 300股 成本 45.20\n\
                    平安银行 1000\n";
        let rows = parse(text, &known());
        assert_eq!(rows.len(), 4, "表头没有代码，跳过");
        assert_eq!(
            (
                rows[0].thscode.as_deref(),
                rows[0].shares,
                rows[0].cost_price
            ),
            (Some("600519.SH"), Some(100), Some(1498.0))
        );
        assert_eq!(
            (
                rows[1].thscode.as_deref(),
                rows[1].shares,
                rows[1].cost_price
            ),
            (Some("000858.SZ"), Some(200), Some(142.5))
        );
        assert_eq!(
            (
                rows[2].thscode.as_deref(),
                rows[2].shares,
                rows[2].cost_price
            ),
            (Some("601318.SH"), Some(300), Some(45.2)),
            "按简称认，取最长的匹配"
        );
        assert_eq!(
            (
                rows[3].thscode.as_deref(),
                rows[3].shares,
                rows[3].cost_price
            ),
            (Some("000001.SZ"), Some(1000), None)
        );
        assert!(rows.iter().all(|r| r.problem.is_none()));
        let rows = parse("600519 贵州茅台", &known());
        assert_eq!(rows[0].problem.as_deref(), Some("没有找到股数"));
        // ETF 认不出：照样返回并说明，不悄悄丢掉
        assert!(parse("合计 123456.78", &known()).is_empty(), "金额不是代码");
        let rows = parse("510300 沪深300ETF 1000 4.012", &known());
        assert_eq!(rows.len(), 1);
        assert!(
            rows[0].thscode.is_none()
                && rows[0].problem.as_deref().unwrap().contains("510300 不是"),
            "{rows:?}"
        );
    }
}
