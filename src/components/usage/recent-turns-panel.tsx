import { ChevronDown } from "lucide-react";
import { useTranslation } from "react-i18next";

import type { TokenUsagePayload } from "@/types";

type RecentTurn = TokenUsagePayload["local"]["recent"][number];

function formatTokens(value: number) {
  return new Intl.NumberFormat(undefined, {
    notation: value >= 10_000 ? "compact" : "standard",
    maximumFractionDigits: 1,
  }).format(value);
}

export function RecentTurnsPanel({ items }: { items: RecentTurn[] }) {
  const { t } = useTranslation();
  const rows = items.slice(0, 8);
  if (rows.length === 0) return null;

  const totalTokens = rows.reduce((sum, item) => sum + item.totalTokens, 0);
  const latestAt = rows[0]?.timestamp;

  return (
    <details className="group overflow-hidden rounded-xl border border-border/70 bg-background/55">
      <summary className="flex cursor-pointer list-none items-center justify-between gap-3 px-4 py-3 transition-colors hover:bg-muted/25 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary/40 [&::-webkit-details-marker]:hidden">
        <div className="min-w-0">
          <h4 className="text-xs font-semibold uppercase tracking-[0.12em] text-foreground/90">
            {t("accounts.recentAgentTurns")}
          </h4>
          <p className="mt-0.5 text-[10px] text-muted-foreground">
            {t("accounts.recentTurnsSummary", {
              count: rows.length,
              tokens: formatTokens(totalTokens),
            })}
          </p>
        </div>
        <div className="flex shrink-0 items-center gap-2 text-[10px] text-muted-foreground">
          {latestAt ? <span className="hidden font-mono sm:inline">{new Date(latestAt * 1000).toLocaleString()}</span> : null}
          <ChevronDown className="h-4 w-4 transition-transform duration-200 group-open:rotate-180" />
        </div>
      </summary>

      <div className="border-t border-border/70">
        <div className="hidden grid-cols-[120px_minmax(0,1fr)_170px_90px] gap-4 border-b border-border/60 bg-muted/20 px-4 py-2 text-[10px] font-medium text-muted-foreground sm:grid">
          <span>{t("accounts.sourceColumn")}</span>
          <span>{t("accounts.modelColumn")}</span>
          <span>{t("accounts.timeColumn")}</span>
          <span className="text-right">{t("accounts.tokenColumn")}</span>
        </div>
        <div className="divide-y divide-border/60">
          {rows.map((item, index) => {
            const cacheTokens = item.cacheReadTokens + item.cacheWriteTokens;
            return (
              <div
                key={item.timestamp + "-" + item.sourceId + "-" + item.sessionId + "-" + index}
                className="grid gap-x-4 gap-y-2 px-4 py-3 text-xs transition-colors hover:bg-muted/20 sm:grid-cols-[120px_minmax(0,1fr)_170px_90px]"
              >
                <div className="flex min-w-0 items-center gap-1.5 font-medium">
                  <span className="h-1.5 w-1.5 shrink-0 rounded-full bg-primary/70" />
                  <span className="truncate">{item.sourceName}</span>
                </div>
                <div className="min-w-0">
                  <p className="truncate font-mono text-[11px] text-foreground" title={item.model}>
                    {item.model || "—"}
                  </p>
                  <p className="mt-0.5 truncate font-mono text-[9px] text-muted-foreground" title={item.sessionId}>
                    {t("accounts.sessionShort")} · {item.sessionId || "—"}
                  </p>
                </div>
                <div className="text-[10px] text-muted-foreground sm:text-xs">
                  {new Date(item.timestamp * 1000).toLocaleString()}
                </div>
                <div className="text-right font-mono font-semibold tabular-nums text-foreground">
                  {formatTokens(item.totalTokens)}
                </div>
                <div className="col-span-full flex flex-wrap gap-x-4 gap-y-1 rounded-lg bg-muted/25 px-3 py-1.5 text-[10px] text-muted-foreground">
                  <span>
                    {t("accounts.inputShort")} <strong className="font-mono font-medium text-sky-600 dark:text-sky-400">{formatTokens(item.inputTokens)}</strong>
                  </span>
                  <span>
                    {t("accounts.outputShort")} <strong className="font-mono font-medium text-amber-600 dark:text-amber-400">{formatTokens(item.outputTokens)}</strong>
                  </span>
                  <span>
                    {t("accounts.reasoningShort")} <strong className="font-mono font-medium text-primary">{formatTokens(item.reasoningTokens)}</strong>
                  </span>
                  <span>
                    {t("accounts.cacheShort")} <strong className="font-mono font-medium text-emerald-600 dark:text-emerald-400">{formatTokens(cacheTokens)}</strong>
                  </span>
                </div>
              </div>
            );
          })}
        </div>
      </div>
    </details>
  );
}
