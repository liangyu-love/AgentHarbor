import { useContext } from "react";
import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { ChartColumn, RefreshCw } from "lucide-react";

import { PageHero } from "@/components/layout/page-hero";
import { PageActiveContext } from "@/components/layout/page-stage";
import { TokenUsagePanel } from "@/components/usage/token-usage-panel";
import { Button } from "@/components/ui/button";
import { REFRESH_INTERVAL_MS, type RefreshInterval } from "@/hooks/use-auto-refresh";
import { toast } from "@/hooks/use-toast";
import { api } from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";
import { cn } from "@/lib/utils";

// Token monitor on its own page: local Codex / Claude Code turns and relay
// traffic, polled only while the page is active.
export function UsagePage({ refreshInterval, onManageProviders }: { refreshInterval: RefreshInterval; onManageProviders: () => void }) {
  const { t } = useTranslation();
  const pageActive = useContext(PageActiveContext);
  const usage = useQuery({
    queryKey: queryKeys.tokenUsage,
    queryFn: api.usage.overview,
    enabled: pageActive,
    refetchInterval: pageActive ? REFRESH_INTERVAL_MS[refreshInterval] : false,
    refetchIntervalInBackground: false,
    refetchOnMount: "always",
    refetchOnWindowFocus: false,
    refetchOnReconnect: false,
    retry: false,
  });
  const refresh = async () => {
    const result = await usage.refetch();
    if (result.data) toast({ title: t("usage.refreshSuccess"), variant: "success" });
  };

  return (
    <div className="mx-auto max-w-7xl space-y-5 pb-8">
      <PageHero kicker="Usage" icon={<ChartColumn />} titleAs="h2" title={t("usage.title")}
        description={t("usage.heroDesc")}
        actions={<Button variant="outline" onClick={() => void refresh()} disabled={usage.isFetching}>
          <RefreshCw className={cn(usage.isFetching && "animate-spin")} />
          {t("common.refresh")}
        </Button>} />
      <TokenUsagePanel
        data={usage.data}
        loading={usage.isPending}
        error={usage.error}
        onRetry={() => void usage.refetch()}
        onManageProviders={onManageProviders}
      />
    </div>
  );
}
