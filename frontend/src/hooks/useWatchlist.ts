import { useQueryClient } from "@tanstack/react-query";
import { useMemo } from "react";
import {
  getListWatchlistQueryKey,
  useAddWatch,
  useListWatchlist,
  useRemoveWatch,
} from "../api/generated/watchlist/watchlist";

/** 收藏状态与切换。各页面的星标共用同一份自选股列表；收藏变化后刷新所有自选股查询（含不同均线周期的） */
export function useWatchlist() {
  const qc = useQueryClient();
  const list = useListWatchlist();
  const onSuccess = () => qc.invalidateQueries({ queryKey: getListWatchlistQueryKey() });
  const add = useAddWatch({ mutation: { onSuccess } });
  const remove = useRemoveWatch({ mutation: { onSuccess } });
  const watched = useMemo(() => new Set(list.data?.items.map((x) => x.thscode)), [list.data]);
  const pendingCode = add.isPending ? add.variables?.thscode : remove.isPending ? remove.variables?.thscode : undefined;
  return {
    watched,
    ready: list.isSuccess,
    pendingCode,
    toggle: (thscode: string) => (watched.has(thscode) ? remove.mutate({ thscode }) : add.mutate({ thscode })),
  };
}
