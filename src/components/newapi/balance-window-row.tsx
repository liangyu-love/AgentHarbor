import { useTranslation } from "react-i18next";
import type { NewApiBalance } from "@/types/newapi";

export function BalanceWindowRow({ entry, label, amount, unavailableText }: {
  entry: NonNullable<NewApiBalance["entries"]>[number];
  label: string;
  amount: (value: number | null | undefined) => string;
  unavailableText: string;
}) {
  const { t, i18n } = useTranslation();
  const percent = entry.used != null && entry.total != null && entry.total > 0
    ? Math.max(0, Math.min(100, entry.used / entry.total * 100)) : null;
  const reset = entry.resetsAt && Number.isFinite(Date.parse(entry.resetsAt))
    ? new Date(entry.resetsAt).toLocaleString(i18n.language) : null;

  return (
    <div className="space-y-1.5">
      <div className="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1 text-xs">
        <span className="text-muted-foreground">{label}</span>
        <span className="tabular-nums">{entry.used != null || entry.total != null
          ? `${amount(entry.used)} / ${amount(entry.total)}` : amount(entry.remaining)}</span>
      </div>
      {!entry.valid && <p className="text-[11px] text-destructive">{unavailableText}</p>}
      {percent != null && (
        <div role="progressbar" aria-label={label} aria-valuemin={0} aria-valuemax={100} aria-valuenow={percent} className="h-1.5 overflow-hidden rounded-full bg-muted/70">
          <div className="h-full rounded-full bg-foreground/80" style={{ width: `${percent}%` }} />
        </div>
      )}
      {reset && <p className="text-[11px] text-muted-foreground">{t("newapi.balanceResets", { time: reset })}</p>}
    </div>
  );
}
