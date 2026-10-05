import { useTranslation } from "react-i18next";
import type { NewApiBalance } from "@/types/newapi";
import { BalanceWindowRow } from "./balance-window-row";

export function OpenCodeGoBalanceSummary({ balance, providerName }: {
  balance: NewApiBalance; providerName: string;
}) {
  const { t, i18n } = useTranslation();
  const number = (value: number) => new Intl.NumberFormat(i18n.language, { maximumFractionDigits: 2 }).format(value);
  const entries: NonNullable<NewApiBalance["entries"]> = balance.entries?.length ? balance.entries : [{
    remaining: balance.conversionAvailable ? balance.remaining : balance.rawQuota,
    used: balance.used, total: null, unit: balance.unit, valid: balance.remaining > 0,
  }];

  return (
    <section data-opencode-go-balance className="min-w-0 space-y-4 rounded-xl border border-border/70 bg-card p-4">
      <div className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 border-b border-border/60 pb-3">
        <span className="min-w-0 break-words text-sm font-semibold">{providerName}</span>
        <span className="rounded-md border border-border/70 px-1.5 py-0.5 text-[11px] font-medium text-muted-foreground">OpenCode Go</span>
      </div>
      {entries.map((entry, index) => {
        const label = entry.label ? t(`newapi.go.${entry.label}`, { defaultValue: entry.label }) : t("newapi.balanceRemaining");
        const amount = (value: number | null | undefined) => value == null ? "—"
          : entry.unit === "%" ? `${number(value)}%` : `${number(value)} ${entry.unit}`;
        return (
          <div key={`${entry.label}-${index}`} data-opencode-go-window={entry.label ?? "remaining"}>
            <BalanceWindowRow entry={entry} label={label} amount={amount} unavailableText={t("newapi.go.windowUnavailable")} />
            {(entry.used != null || entry.total != null) && <p className="mt-1.5 text-[11px] text-muted-foreground">{t("newapi.balanceRemaining")}: {amount(entry.remaining)}</p>}
          </div>
        );
      })}
      <p className="text-[11px] text-muted-foreground">{t("newapi.balanceUpdatedAt", {
        time: new Date(balance.checkedAt * 1000).toLocaleString(i18n.language),
      })}</p>
    </section>
  );
}
