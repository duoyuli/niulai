//! 运营建议的推送：每个用户可以填一个群机器人的地址（企业微信、飞书、钉钉），
//! 每个账户每个交易日的建议生成后推送一次：要卖什么、要买什么、最高买入价，以及打开网页的链接。
//! 服务器要主动请求这个地址，所以只接受这三家的 https 地址，不能填任意网址。规则见 docs/domains/accounts.md#notify。

use std::time::Duration;

use serde_json::{Value, json};
use sqlx::PgPool;

use crate::advice::generate::{Action, Advice};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    WeCom,
    Feishu,
    DingTalk,
}

/// 认出是哪一家的机器人地址；不是这三家的 https 地址时说明原因
pub fn channel(url: &str) -> Result<Channel, String> {
    let parsed = reqwest::Url::parse(url.trim()).map_err(|_| "地址格式不对".to_string())?;
    if parsed.scheme() != "https" {
        return Err("只支持 https 地址".into());
    }
    match parsed.host_str().unwrap_or_default() {
        "qyapi.weixin.qq.com" => Ok(Channel::WeCom),
        "open.feishu.cn" | "open.larksuite.com" => Ok(Channel::Feishu),
        "oapi.dingtalk.com" => Ok(Channel::DingTalk),
        _ => Err("只支持企业微信、飞书、钉钉群机器人的地址".into()),
    }
}

/// 发一条文字消息
pub async fn send(url: &str, text: &str) -> anyhow::Result<()> {
    let ch = channel(url).map_err(|e| anyhow::anyhow!(e))?;
    let body = match ch {
        Channel::WeCom | Channel::DingTalk => json!({ "msgtype": "text", "text": { "content": text } }),
        Channel::Feishu => json!({ "msg_type": "text", "content": { "text": text } }),
    };
    let http = reqwest::Client::builder().timeout(Duration::from_secs(10)).build()?;
    let resp = http.post(url.trim()).json(&body).send().await?;
    let status = resp.status();
    let v: Value = resp.json().await.unwrap_or(Value::Null);
    // 三家都在 HTTP 200 里用 errcode / code 表示失败
    let code = v.get("errcode").or_else(|| v.get("code")).and_then(Value::as_i64).unwrap_or(0);
    if !status.is_success() || code != 0 {
        let msg = v.get("errmsg").or_else(|| v.get("msg")).and_then(Value::as_str).unwrap_or_default();
        anyhow::bail!("机器人返回 {status}，错误码 {code}{}", if msg.is_empty() { String::new() } else { format!("：{msg}") });
    }
    Ok(())
}

const WEEKDAYS: [&str; 7] = ["周一", "周二", "周三", "周四", "周五", "周六", "周日"];

/// 推送的文字：哪一天、结论、每一条卖出和买入，最后是链接
pub fn advice_text(account: &str, a: &Advice, link: Option<&str>) -> String {
    use chrono::Datelike;
    let day = format!("{}（{}）", a.next_date.format("%m-%d"), WEEKDAYS[a.next_date.weekday().num_days_from_monday() as usize]);
    let mut lines = vec![format!("【牛来】{account} · {day}运营建议：{}", a.headline)];
    for i in &a.items {
        match i.action {
            Action::Sell => lines.push(format!("卖出 {} {} 股（开盘后，全部）：{}", i.name, i.shares, i.headline)),
            Action::Reduce => lines.push(format!("减仓 {} {} 股（开盘后）：{}", i.name, i.shares, i.headline)),
            Action::Buy => {
                let limit = i.limit_price.map(|p| format!("，开盘价不超过 {p:.2} 才买")).unwrap_or_default();
                let stop = i.stop_price.map(|p| format!("，止损价 {p:.2}")).unwrap_or_default();
                lines.push(format!("买入 {} {} 股（参考价 {:.2}{limit}{stop}）", i.name, i.shares, i.ref_price));
            }
            _ => {}
        }
    }
    if let Some(w) = &a.why_nothing {
        lines.push(w.clone());
    }
    if let Some(h) = &a.settings_hint {
        lines.push(format!("{h}。"));
    }
    lines.push(format!("根据 {} 收盘的数据，仅供参考，不构成投资建议。", a.trade_date));
    if let Some(l) = link {
        lines.push(l.to_string());
    }
    lines.join("\n")
}

/// 执行循环每分钟调一次：给填了机器人地址的用户，推送每个账户还没推送过的最新一份建议（同一个账户同一个交易日只推一次，
/// 之后改持仓重新生成也不再推）。只推一天内生成的、操作日还没过的。失败也记为已推，不反复重试，原因显示在「我的」页面
pub async fn push_new_advice(pool: &PgPool, public_url: Option<&str>) -> anyhow::Result<()> {
    let rows: Vec<(i64, i64, String, i64, String, Value)> = sqlx::query_as(
        "SELECT a.advice_id, a.account_id, acc.name, u.user_id, u.notify_webhook, a.result
         FROM app.advice a JOIN app.account acc USING (account_id) JOIN app.user u ON u.user_id = acc.user_id
         WHERE a.superseded_by IS NULL AND u.notify_webhook IS NOT NULL AND NOT u.disabled
           AND a.created_at > now() - interval '1 day'
           AND a.trade_date = (SELECT max(trade_date) FROM app.advice m WHERE m.account_id = a.account_id)
           AND NOT EXISTS (SELECT 1 FROM app.advice b WHERE b.account_id = a.account_id AND b.trade_date = a.trade_date AND b.notified_at IS NOT NULL)
         ORDER BY a.advice_id",
    )
    .fetch_all(pool)
    .await?;
    let today = crate::time::today_cn();
    for (advice_id, account_id, name, user_id, url, result) in rows {
        sqlx::query("UPDATE app.advice SET notified_at = now() WHERE advice_id = $1").bind(advice_id).execute(pool).await?;
        let Ok(a) = serde_json::from_value::<Advice>(result) else { continue };
        if a.next_date < today {
            continue;
        }
        let link = public_url.map(|u| format!("{}/accounts/{account_id}", u.trim_end_matches('/')));
        let err = send(&url, &advice_text(&name, &a, link.as_deref())).await.err().map(|e| format!("{e:#}"));
        if let Some(e) = &err {
            tracing::warn!(user_id, account_id, error = %e, "推送运营建议失败");
        }
        sqlx::query("UPDATE app.user SET notify_error = $2 WHERE user_id = $1").bind(user_id).bind(&err).execute(pool).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_known_bot_hosts() {
        assert_eq!(channel("https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=x"), Ok(Channel::WeCom));
        assert_eq!(channel("https://open.feishu.cn/open-apis/bot/v2/hook/x"), Ok(Channel::Feishu));
        assert_eq!(channel("https://oapi.dingtalk.com/robot/send?access_token=x"), Ok(Channel::DingTalk));
        assert!(channel("http://qyapi.weixin.qq.com/x").is_err(), "要 https");
        assert!(channel("https://127.0.0.1/x").is_err(), "不能让服务器请求任意地址");
        assert!(channel("https://qyapi.weixin.qq.com.evil.com/x").is_err());
    }
}
