import { useEffect } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";

/** Both provider cards and the usage page consume the saved balance snapshot. */
export function useProviderBalanceUpdates() {
  const queryClient = useQueryClient();
  useEffect(() => {
    let disposed = false;
    let stop: (() => void) | undefined;
    void api.providers.onBalanceUpdated(() => {
      void queryClient.invalidateQueries({ queryKey: queryKeys.providers });
    }).then(unlisten => {
      if (disposed) unlisten();
      else stop = unlisten;
    }).catch(() => undefined);
    return () => { disposed = true; stop?.(); };
  }, [queryClient]);
}
