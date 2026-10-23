import type { QueryClient, QueryKey } from "@tanstack/react-query";
import {
  getAccountReviewQueryKey,
  getAdviceHistoryQueryKey,
  getCurrentAdviceQueryKey,
  getGetAccountQueryKey,
  getGetEquityQueryKey,
  getListAccountsQueryKey,
  getListCorpActionsQueryKey,
  getListTradesQueryKey,
  getShadowOrdersQueryKey,
} from "../api/generated/accounts/accounts";
import type { AccountDetail, AdviceResponse } from "../api/generated/model";

/** 账户相关的查询 */
export type AccountQuery =
  | "accounts"
  | "account"
  | "advice"
  | "adviceHistory"
  | "review"
  | "shadowOrders"
  | "equity"
  | "trades"
  | "corpActions";

/**
 * 账户上的一次修改改变了什么。新增账户派生视图时只在这里登记它受哪类修改影响，不用逐个找 mutation 回调：
 * - `ledger`：持仓、现金或流水变了（记一笔、撤销、校准、配股认购或暂不处理），建议在后台重新生成；
 * - `advice`：处理了一条建议，「已执行」会记一笔流水，所以账本相关的也要刷新；
 * - `settings`：名字、规则或费率变了，建议在后台重新生成，账本不变。
 */
export type AccountChange = "ledger" | "advice" | "settings";

const AFFECTS: Record<AccountChange, readonly AccountQuery[]> = {
  ledger: [
    "accounts",
    "account",
    "advice",
    "adviceHistory",
    "review",
    "shadowOrders",
    "equity",
    "trades",
    "corpActions",
  ],
  advice: ["accounts", "account", "advice", "adviceHistory", "review", "shadowOrders", "equity", "trades"],
  settings: ["accounts", "account", "advice", "adviceHistory"],
};

export function affectedQueries(change: AccountChange): readonly AccountQuery[] {
  return AFFECTS[change];
}

function queryKey(q: AccountQuery, accountId: number): QueryKey {
  switch (q) {
    case "accounts":
      return getListAccountsQueryKey();
    case "account":
      return getGetAccountQueryKey(accountId);
    case "advice":
      return getCurrentAdviceQueryKey(accountId);
    case "adviceHistory":
      return getAdviceHistoryQueryKey(accountId);
    case "review":
      return getAccountReviewQueryKey(accountId);
    case "shadowOrders":
      return getShadowOrdersQueryKey(accountId);
    case "equity":
      return getGetEquityQueryKey(accountId);
    case "trades":
      return getListTradesQueryKey(accountId);
    case "corpActions":
      return getListCorpActionsQueryKey(accountId);
  }
}

/**
 * 账户写操作成功后刷新受影响的查询，只动这个账户的，不把全市场的长任务也判脏。
 * 接口已经返回了新的账户详情或建议时传进来，直接放进缓存，不再重新请求这两个。
 */
export function invalidateAccountQueries(
  queryClient: QueryClient,
  accountId: number,
  change: AccountChange,
  fresh: { account?: AccountDetail; advice?: AdviceResponse } = {},
) {
  if (fresh.account) queryClient.setQueryData(getGetAccountQueryKey(accountId), fresh.account);
  if (fresh.advice) queryClient.setQueryData(getCurrentAdviceQueryKey(accountId), fresh.advice);
  for (const q of affectedQueries(change)) {
    if ((q === "account" && fresh.account) || (q === "advice" && fresh.advice)) continue;
    void queryClient.invalidateQueries({ queryKey: queryKey(q, accountId) });
  }
}
