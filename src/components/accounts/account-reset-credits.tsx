import { useEffect, useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { CalendarClock, Loader2, RefreshCw, Ticket, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Popover, PopoverClose, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import type { AccountResetCredit, CodexAccountSummary } from "@/types";

export function AccountResetCreditsBadge({ account, refreshing, onRefresh }: {
  account: CodexAccountSummary; refreshing: boolean; onRefresh: () => void;
}) {
  const { t, i18n } = useTranslation();
  const titleId = useId();
  const [open, setOpen] = useState(false);
  const pinned = useRef(false);
  const timer = useRef<ReturnType<typeof setTimeout>>();
  const content = useRef<HTMLDivElement>(null);
  const info = account.resetCredits;
  const cancelClose = () => clearTimeout(timer.current);
  const close = () => { cancelClose(); setOpen(false); pinned.current = false; };
  const leave = () => {
    cancelClose();
    if (!pinned.current) timer.current = setTimeout(() => setOpen(false), 180);
  };
  useEffect(() => () => clearTimeout(timer.current), []);
  const now = Date.now() / 1000;
  const available = info?.credits?.filter(credit => credit.status === "available")
    .sort((a, b) => (a.expiresAt ?? Infinity) - (b.expiresAt ?? Infinity)) ?? [];
  const recent = info?.credits?.filter(credit => credit.grantedAt != null && credit.grantedAt <= now && credit.grantedAt >= now - 30 * 86400)
    .sort((a, b) => (b.grantedAt ?? 0) - (a.grantedAt ?? 0)) ?? [];
  const date = (time: number) => new Date(time * 1000).toLocaleString(i18n.language, { year: "numeric", month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit", hour12: false });
  const expiry = (credit: AccountResetCredit) => credit.expiresAt == null ? t("accounts.resetNoExpiry")
    : credit.expiresAt <= now ? t("accounts.resetExpired")
    : t("accounts.resetDaysLeft", { count: Math.ceil((credit.expiresAt - now) / 86400) });
  return (
    <Popover open={open} onOpenChange={value => { if (value) setOpen(true); else close(); }}>
      <PopoverTrigger asChild>
        <button type="button" className="inline-flex items-center gap-1 rounded-full bg-primary/10 px-2 py-1 text-[11px] font-semibold text-primary hover:bg-primary/20 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          aria-label={info ? t("accounts.resetAvailable", { count: info.availableCount }) : t("accounts.resetUnknown")}
          onPointerEnter={event => { if (event.pointerType === "mouse") { cancelClose(); setOpen(true); } }} onPointerLeave={leave}
          onClick={event => {
            event.preventDefault();
            if (pinned.current) { close(); return; }
            cancelClose(); pinned.current = true; setOpen(true);
            if (event.detail === 0) requestAnimationFrame(() => content.current?.querySelector<HTMLButtonElement>("button")?.focus());
          }}>
          <Ticket className="h-3 w-3" /><span className="tabular-nums">{info?.availableCount ?? "—"}</span>
        </button>
      </PopoverTrigger>
      <PopoverContent ref={content} aria-labelledby={titleId} onOpenAutoFocus={event => event.preventDefault()}
        onPointerEnter={cancelClose} onPointerLeave={leave} onFocusCapture={() => { pinned.current = true; cancelClose(); }}>
        <div className="mb-3 flex items-center gap-2">
          <Ticket className="h-4 w-4 text-primary" /><h4 id={titleId} className="flex-1 text-sm font-semibold">{t("accounts.resetCredits")} {" "}
            <span className="ml-2 font-normal tabular-nums text-muted-foreground">{info?.availableCount ?? "—"}</span></h4>
          <PopoverClose asChild><Button variant="ghost" size="icon-sm" aria-label={t("common.close")} className="h-7 w-7"><X className="h-3.5 w-3.5" /></Button></PopoverClose>
        </div>
        {!info ? <p className="text-xs leading-relaxed text-muted-foreground">{t("accounts.resetNotReported")}</p>
          : info.credits == null ? <p className="text-xs leading-relaxed text-muted-foreground">{t(info.availableCount ? "accounts.resetDetailsUnavailable" : "accounts.resetNone")}</p>
          : <>
            {available.length === 0 && <p className="text-xs text-muted-foreground">{t(info.availableCount ? "accounts.resetDetailsIncomplete" : "accounts.resetNone")}</p>}
            <ul className="divide-y divide-border">{available.map((credit, index) => <li key={index} className="flex gap-2 py-2.5">
              <CalendarClock className="mt-0.5 h-4 w-4 shrink-0 text-muted-foreground" />
              <div className="min-w-0 flex-1 text-xs"><div className="flex flex-wrap justify-between gap-1"><span className="font-medium">{t("accounts.resetCardNumber", { number: index + 1 })}</span><span className="text-muted-foreground">{expiry(credit)}</span></div>
                <p className="mt-1 text-muted-foreground">{credit.expiresAt == null ? t("accounts.resetNoExpiry") : t("accounts.resetExpiresAt", { time: date(credit.expiresAt) })}</p>
              </div>
            </li>)}</ul>
            <details className="mt-2 border-t border-border pt-2 text-xs">
              <summary className="cursor-pointer rounded py-1 text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring">{t("accounts.resetGrantHistory")} · {t("accounts.resetLast30Days")}</summary>
              <ul className="mt-1 space-y-2">{recent.map((credit, index) => <li key={index} className="flex flex-wrap justify-between gap-1">
                <time className="text-muted-foreground">{date(credit.grantedAt!)}</time>
                <span>{t([`accounts.resetStatus.${credit.status}`, "accounts.resetStatus.unknown"])}</span>
              </li>)}</ul>
              {recent.length === 0 && <p className="py-2 text-muted-foreground">{t("accounts.resetNoHistory")}</p>}
            </details>
          </>}
        <div className="mt-3 flex flex-wrap items-center justify-between gap-2 border-t border-border pt-2">
          <span className="text-[10px] text-muted-foreground">{account.lastUsageAt ? date(account.lastUsageAt) : t("accounts.unknown")}</span>
          <Button variant="ghost" size="sm" className="h-7 gap-1 text-xs" disabled={refreshing} onClick={onRefresh}>
            {refreshing ? <Loader2 className="h-3 w-3 animate-spin" /> : <RefreshCw className="h-3 w-3" />}{t("common.refresh")}
          </Button>
        </div>
      </PopoverContent>
    </Popover>
  );
}
