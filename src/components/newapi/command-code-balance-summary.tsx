import { useTranslation } from "react-i18next";
import type { NewApiBalance } from "@/types/newapi";
import { BalanceWindowRow } from "./balance-window-row";

export function CommandCodeBalanceSummary({ balance, providerName }: {
  balance: NewApiBalance; providerName: string;
}) {
  const { t, i18n } = useTranslation();
  const number = (value: number) => new Intl.NumberFormat(i18n.language, { maximumFractionDigits: 4 }).format(value);
  const tokens = (value: number | null | undefined) => value == null ? "—" : value >= 1_000_000
    ? `${new Intl.NumberFormat(i18n.language, { maximumFractionDigits: 1 }).format(value / 1_000_000)}M`
    : value >= 1_000 ? `${new Intl.NumberFormat(i18n.language, { maximumFractionDigits: 1 }).format(value / 1_000)}K` : number(value);
  const amount = (value: number | null | undefined) => value == null ? "—" : `$${number(value)}`;
  const date = (value: string | null | undefined) => value && Number.isFinite(Date.parse(value))
    ? new Date(value).toLocaleString(i18n.language) : null;
  const entries: NonNullable<NewApiBalance["entries"]> = balance.entries?.length ? balance.entries : [{
    remaining: balance.conversionAvailable ? balance.remaining : balance.rawQuota,
    used: balance.used, total: null, unit: balance.unit, valid: balance.remaining > 0,
  }];
  const monthly = entries.find(entry => entry.label === "monthly" || entry.label?.startsWith("Command Code"))
    ?? (balance.entries?.length ? undefined : entries[0]);
  const purchased = entries.find(entry => entry.label === "purchased");
  const free = entries.find(entry => entry.label === "free");
  const windows = entries.filter(entry => entry.label === "fiveHour" || entry.label === "weekly");
  const details = balance.commandCode;
  const planLabel = details?.plan?.name ?? (monthly?.label && monthly.label !== "monthly" ? monthly.label : null);
  const billingEnd = date(details?.plan?.currentPeriodEnd);
  const monthlyReset = !billingEnd ? date(monthly?.resetsAt) : null;
  const usage = details?.usage;
  const totalTokens = usage?.totalTokensIn != null && usage.totalTokensOut != null
    ? usage.totalTokensIn + usage.totalTokensOut : null;
  const hasUsage = usage && [usage.totalCount, usage.completedCount, usage.failedCount, usage.successRate,
    usage.totalCost, usage.totalCredits, usage.totalTokensIn, usage.totalTokensOut].some(value => value != null);

  return (
    <section data-command-code-balance className="min-w-0 space-y-4 rounded-xl border border-border/70 bg-card p-4">
      <div className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 border-b border-border/60 pb-3">
        <span className="min-w-0 break-words text-sm font-semibold">{details?.account?.name ?? providerName}</span>
        {details?.account?.userName && <span className="min-w-0 break-all text-xs text-muted-foreground">{details.account.userName}</span>}
        {planLabel && <span className="rounded-md border border-border/70 px-1.5 py-0.5 text-[11px] font-medium text-muted-foreground">{planLabel}</span>}
      </div>

      {hasUsage && (
        <div className="space-y-2">
          {usage.periodBasis === "billing-period" && <p className="text-[11px] text-muted-foreground">{t("newapi.commandCode.billingPeriod")}</p>}
          <div className="grid grid-cols-2 gap-2 sm:grid-cols-4" data-command-code-usage>
            <div className="min-w-0 rounded-lg bg-muted/45 p-3" data-command-code-stat="requests">
              <p className="text-[11px] text-muted-foreground">{t("newapi.commandCode.requests")}</p>
              <strong className="mt-1 block text-lg font-semibold tabular-nums">{usage.totalCount == null ? "—" : number(usage.totalCount)}</strong>
              {usage.failedCount != null && <p className="mt-0.5 text-[11px] text-muted-foreground">{t("newapi.commandCode.failed", { count: usage.failedCount })}</p>}
            </div>
            <div className="min-w-0 rounded-lg bg-muted/45 p-3" data-command-code-stat="successRate">
              <p className="text-[11px] text-muted-foreground">{t("newapi.commandCode.successRate")}</p>
              <strong className="mt-1 block text-lg font-semibold tabular-nums">{usage.successRate == null ? "—" : `${number(usage.successRate)}%`}</strong>
            </div>
            <div className="min-w-0 rounded-lg bg-muted/45 p-3" data-command-code-stat="cost">
              <p className="text-[11px] text-muted-foreground">{t("newapi.commandCode.cost")}</p>
              <strong className="mt-1 block break-words text-lg font-semibold tabular-nums">{usage.totalCost == null ? "—" : `$${number(usage.totalCost)}`}</strong>
              {usage.totalCredits != null && <p className="mt-0.5 break-words text-[11px] text-muted-foreground">{amount(usage.totalCredits)}</p>}
            </div>
            <div className="min-w-0 rounded-lg bg-muted/45 p-3" data-command-code-stat="tokens">
              <p className="text-[11px] text-muted-foreground">Token</p>
              <strong className="mt-1 block text-lg font-semibold tabular-nums">{tokens(totalTokens)}</strong>
              {(usage.totalTokensIn != null || usage.totalTokensOut != null) && <p className="mt-0.5 break-words text-[11px] text-muted-foreground">{t("newapi.commandCode.tokensDetail", { input: tokens(usage.totalTokensIn), output: tokens(usage.totalTokensOut) })}</p>}
            </div>
          </div>
        </div>
      )}

      {(monthly || purchased || free) && (
        <div className="grid grid-cols-3 gap-2" data-command-code-credits>
          {([
            ["monthlyRemaining", monthly], ["purchased", purchased], ["free", free],
          ] as const).map(([key, entry]) => (
            <div key={key} className="min-w-0 rounded-lg bg-muted/45 p-3">
              <p className="text-[11px] text-muted-foreground">{t(`newapi.commandCode.${key}`)}</p>
              <strong className="mt-1 block break-words text-sm font-semibold tabular-nums">{amount(entry?.remaining)}</strong>
            </div>
          ))}
        </div>
      )}

      {windows.map(entry => (
        <div key={entry.label} data-command-code-window={entry.label}>
          <BalanceWindowRow entry={entry} label={t(`newapi.commandCode.${entry.label}`)} amount={amount}
            unavailableText={t("newapi.commandCode.windowExhausted")} />
        </div>
      ))}

      <div className="flex flex-wrap justify-between gap-x-3 gap-y-1 text-[11px] text-muted-foreground">
        {billingEnd && <span>{t("newapi.commandCode.billingEnds", { time: billingEnd })}</span>}
        {monthlyReset && <span>{t("newapi.balanceResets", { time: monthlyReset })}</span>}
        <span>{t("newapi.balanceUpdatedAt", { time: new Date(balance.checkedAt * 1000).toLocaleTimeString(i18n.language) })}</span>
      </div>
    </section>
  );
}
