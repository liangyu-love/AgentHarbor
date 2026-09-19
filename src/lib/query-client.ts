import { MutationCache, QueryClient } from "@tanstack/react-query";
import { toast } from "@/hooks/use-toast";
import i18n from "@/lib/i18n";

export function createAppQueryClient() {
  return new QueryClient({
    mutationCache: new MutationCache({
      onError: (error, _variables, _context, mutation) => {
        if (mutation.options.onError || (error instanceof Error && error.message === "cancelled")) return;
        toast({ title: i18n.t("common.toastErrorGenericDesc"), description: String(error), variant: "destructive" });
      },
    }),
    defaultOptions: {
      queries: {
        retry: 1,
        refetchOnWindowFocus: false,
        staleTime: 30_000,
      },
    },
  });
}
