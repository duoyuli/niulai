import type { QueryClient } from "@tanstack/react-query";
import {
  getAccountReviewQueryKey,
  getAdviceHistoryQueryKey,
  getCurrentAdviceQueryKey,
  getGetAccountQueryKey,
  getGetEquityQueryKey,
  getListAccountsQueryKey,
  getListCorpActionsQueryKey,
  getListTradesQueryKey,
} from "../api/generated/accounts/accounts";
import { getGetStrategyQueryKey, getListStrategiesQueryKey } from "../api/generated/strategy/strategy";

/** 账户写操作只刷新该账户相关查询，避免把全市场长任务也判脏。 */
export function invalidateAccountQueries(queryClient: QueryClient, accountId: number) {
  for (const queryKey of [
    getGetAccountQueryKey(accountId),
    getCurrentAdviceQueryKey(accountId),
    getAdviceHistoryQueryKey(accountId),
    getGetEquityQueryKey(accountId),
    getAccountReviewQueryKey(accountId),
    getListTradesQueryKey(accountId),
    getListCorpActionsQueryKey(accountId),
    getListAccountsQueryKey(),
  ]) {
    void queryClient.invalidateQueries({ queryKey });
  }
}

export function invalidateStrategyQueries(queryClient: QueryClient, strategyId?: number) {
  void queryClient.invalidateQueries({ queryKey: getListStrategiesQueryKey() });
  if (strategyId !== undefined) {
    void queryClient.invalidateQueries({ queryKey: getGetStrategyQueryKey(strategyId) });
  }
}
