import { useTranslation } from "react-i18next";
import { CalendarCheck, CalendarClock, CalendarX, Loader2 } from "lucide-react";

import type { useAccountSources } from "@/components/accounts/use-account-sources";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { cn } from "@/lib/utils";
import type { AccountSource, AccountSourceAccount, AccountSourceKind, AccountSourceListPayload } from "@/types/account-sources";

type Sources = ReturnType<typeof useAccountSources>;

function formatCredit(value: number) {
  return new Intl.NumberFormat(undefined, { maximumFractionDigits: 2 }).format(value);
}

/** How today's check-in stands for a source's taking-part accounts. */
export function checkinTally(data: AccountSourceListPayload, kind: AccountSourceKind) {
  const tally = { claimed: 0, done: 0, missed: 0 };
  const accounts = data.sources.find(source => source.kind === kind)?.accounts ?? [];
  for (const account of accounts) {
    if (account.lapsed || account.checkinOff) continue;
    const outcome = account.checkin?.day === data.today ? account.checkin.outcome : null;
    if (outcome === "claimed") tally.claimed += 1;
    else if (outcome === "done") tally.done += 1;
    else tally.missed += 1;
  }
  return tally;
}

/** A source's daily check-in: on each day by itself, or pressed now. */
export function CheckinControls({ source, sources }: { source: AccountSource; sources: Sources }) {
  const { t } = useTranslation();
  if (!source.checksIn) return null;
  const switching = sources.setCheckin.isPending && sources.setCheckin.variables?.kind === source.kind;
  const checking = sources.checkIn.isPending && sources.checkIn.variables === source.kind;
  const usable = source.accounts.some(account => !account.lapsed && !account.checkinOff);
  return (
    <div className="flex flex-wrap items-center gap-x-3 gap-y-2 md:justify-end" data-checkin-controls={source.kind}>
      <label className="flex items-center gap-2 text-sm font-medium" title={t("accountSources.checkin.autoHint")}>
        <Switch checked={source.autoCheckin} disabled={switching}
          onCheckedChange={on => sources.setCheckin.mutate({ kind: source.kind, on })} />
        {t("accountSources.checkin.auto")}
      </label>
      {/* A disabled button gets no pointer events; the wrapper carries the reason. */}
      <span className="inline-flex" title={usable ? undefined : t("accountSources.checkin.noneUsable")}>
        <Button variant="outline" size="xs" className="h-7 gap-1.5" disabled={checking || !usable}
          onClick={() => sources.checkIn.mutate(source.kind)}>
          {checking ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <CalendarCheck className="h-3.5 w-3.5" />}
          {t("accountSources.checkin.now")}
        </Button>
      </span>
    </div>
  );
}

/** How an account's check-in stands today, and whether it takes part. */
export function CheckinStatus({ source, account, sources }: { source: AccountSource; account: AccountSourceAccount; sources: Sources }) {
  const { t } = useTranslation();
  if (!source.checksIn) return null;
  const today = account.checkin && account.checkin.day === sources.today ? account.checkin : null;
  const toggling = sources.setCheckinOff.isPending && sources.setCheckinOff.variables?.id === account.id;
  let tone: "good" | "warn" | "idle" = "idle";
  let text: string;
  if (account.checkinOff) {
    text = t("accountSources.checkin.left");
  } else if (!today) {
    text = t(source.autoCheckin ? "accountSources.checkin.pending" : "accountSources.checkin.notYet");
  } else {
    const credit = today.credit > 0 ? t("accountSources.checkin.credit", { value: formatCredit(today.credit) }) : "";
    const streak = today.streak > 0 ? t("accountSources.checkin.streak", { count: today.streak }) : "";
    const extras = [credit, streak].filter(Boolean).join(" · ");
    switch (today.outcome) {
      case "claimed":
      case "done":
        tone = "good";
        text = [t(`accountSources.checkin.outcome.${today.outcome}`), extras].filter(Boolean).join(" · ");
        break;
      case "failed":
        tone = "warn";
        text = t("accountSources.checkin.outcome.failed", { message: today.message ?? "" });
        break;
      default:
        tone = today.outcome === "inactive" || today.outcome === "ineligible" ? "idle" : "warn";
        text = t(`accountSources.checkin.outcome.${today.outcome}`, { name: source.name });
    }
  }
  const Icon = tone === "good" ? CalendarCheck : tone === "warn" ? CalendarX : CalendarClock;
  return (
    <div className="mt-2 flex items-center gap-2 border-t border-border/40 pt-2 text-xs" data-account-checkin={account.checkinOff ? "off" : today?.outcome ?? "none"}>
      <Icon aria-hidden className={cn("h-3.5 w-3.5 shrink-0", tone === "good" ? "text-emerald-600 dark:text-emerald-400" : tone === "warn" ? "text-amber-600 dark:text-amber-400" : "text-muted-foreground")} />
      <span className={cn("min-w-0 flex-1 truncate", tone === "warn" ? "text-amber-700 dark:text-amber-300" : "text-muted-foreground")}
        title={today?.message ?? text}>{text}</span>
      <Switch checked={!account.checkinOff} disabled={toggling} className="scale-75"
        aria-label={t("accountSources.checkin.takesPart", { name: account.name })}
        onCheckedChange={on => sources.setCheckinOff.mutate({ id: account.id, off: !on })} />
    </div>
  );
}
