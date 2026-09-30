import { useEffect, useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  CalendarClock,
  Clock,
  History,
  Loader2,
  RefreshCw,
  Ticket,
  X,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Popover,
  PopoverClose,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import { cn } from "@/lib/utils";
import type { AccountResetCredit, CodexAccountSummary } from "@/types";

export function AccountResetCreditsBadge({
  account,
  refreshing,
  onRefresh,
}: {
  account: CodexAccountSummary;
  refreshing: boolean;
  onRefresh: () => void;
}) {
  const { t, i18n } = useTranslation();
  const titleId = useId();
  const [open, setOpen] = useState(false);
  const pinned = useRef(false);
  const timer = useRef<ReturnType<typeof setTimeout>>();
  const content = useRef<HTMLDivElement>(null);
  const info = account.resetCredits;

  const cancelClose = () => clearTimeout(timer.current);
  const close = () => {
    cancelClose();
    setOpen(false);
    pinned.current = false;
  };
  const leave = () => {
    cancelClose();
    if (!pinned.current) timer.current = setTimeout(() => setOpen(false), 180);
  };

  useEffect(() => () => clearTimeout(timer.current), []);

  const now = Date.now() / 1000;
  const available =
    info?.credits
      ?.filter((credit) => credit.status === "available")
      .sort((a, b) => (a.expiresAt ?? Infinity) - (b.expiresAt ?? Infinity)) ?? [];

  const recent =
    info?.credits
      ?.filter(
        (credit) =>
          credit.grantedAt != null &&
          credit.grantedAt <= now &&
          credit.grantedAt >= now - 30 * 86400,
      )
      .sort((a, b) => (b.grantedAt ?? 0) - (a.grantedAt ?? 0)) ?? [];

  const date = (time: number) =>
    new Date(time * 1000).toLocaleString(i18n.language, {
      year: "numeric",
      month: "2-digit",
      day: "2-digit",
      hour: "2-digit",
      minute: "2-digit",
      hour12: false,
    });

  const expiry = (credit: AccountResetCredit) =>
    credit.expiresAt == null
      ? t("accounts.resetNoExpiry")
      : credit.expiresAt <= now
        ? t("accounts.resetExpired")
        : t("accounts.resetDaysLeft", {
            count: Math.ceil((credit.expiresAt - now) / 86400),
          });

  const hasCredits = info && info.availableCount > 0;

  return (
    <Popover
      open={open}
      onOpenChange={(value) => {
        if (value) setOpen(true);
        else close();
      }}
    >
      <PopoverTrigger asChild>
        <button
          type="button"
          className={cn(
            "group relative inline-flex items-center gap-1.5 rounded-full px-2.5 py-0.5 text-[11px] font-semibold tracking-wide transition-all duration-200 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring active:scale-95",
            hasCredits
              ? "border border-amber-500/35 bg-gradient-to-r from-amber-500/15 via-orange-500/10 to-amber-500/15 text-amber-600 shadow-[0_2px_8px_-2px_rgba(245,158,11,0.25)] hover:border-amber-500/50 hover:bg-amber-500/20 dark:text-amber-400"
              : info?.availableCount === 0
                ? "border border-border/80 bg-muted/60 text-muted-foreground hover:bg-muted"
                : "border border-primary/25 bg-primary/10 text-primary hover:bg-primary/20",
          )}
          aria-label={
            info
              ? t("accounts.resetAvailable", { count: info.availableCount })
              : t("accounts.resetUnknown")
          }
          onPointerEnter={(event) => {
            if (event.pointerType === "mouse") {
              cancelClose();
              setOpen(true);
            }
          }}
          onPointerLeave={leave}
          onClick={(event) => {
            event.preventDefault();
            if (pinned.current) {
              close();
              return;
            }
            cancelClose();
            pinned.current = true;
            setOpen(true);
            if (event.detail === 0) {
              requestAnimationFrame(() =>
                content.current
                  ?.querySelector<HTMLButtonElement>("button")
                  ?.focus(),
              );
            }
          }}
        >
          <Ticket
            className={cn(
              "h-3 w-3 transition-transform duration-200 group-hover:rotate-12",
              hasCredits ? "text-amber-500" : "opacity-70",
            )}
          />
          <span className="tabular-nums font-mono font-bold">
            {info?.availableCount ?? "—"}
          </span>
          {hasCredits && (
            <span className="h-1.5 w-1.5 rounded-full bg-amber-500 animate-pulse" />
          )}
        </button>
      </PopoverTrigger>

      <PopoverContent
        ref={content}
        aria-labelledby={titleId}
        onOpenAutoFocus={(event) => event.preventDefault()}
        onPointerEnter={cancelClose}
        onPointerLeave={leave}
        onFocusCapture={() => {
          pinned.current = true;
          cancelClose();
        }}
        className="w-84 p-4 shadow-2xl"
      >
        {/* Header */}
        <div className="mb-3 flex items-center justify-between gap-2 border-b border-border/50 pb-2.5">
          <div className="flex items-center gap-2">
            <div className="flex h-7 w-7 items-center justify-center rounded-lg bg-amber-500/15 text-amber-600 dark:text-amber-400">
              <Ticket className="h-4 w-4" />
            </div>
            <h4 id={titleId} className="text-sm font-semibold tracking-tight text-foreground flex items-center gap-1.5">
              <span>{t("accounts.resetCredits")}</span>
              <span className="inline-flex items-center justify-center rounded-full bg-muted px-2 py-0.5 text-xs font-bold font-mono tabular-nums text-foreground">
                {info?.availableCount ?? "—"}
              </span>
            </h4>
          </div>
          <PopoverClose asChild>
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label={t("common.close")}
              className="h-7 w-7 rounded-lg text-muted-foreground hover:bg-muted hover:text-foreground"
            >
              <X className="h-3.5 w-3.5" />
            </Button>
          </PopoverClose>
        </div>

        {/* Body */}
        {!info ? (
          <div className="rounded-xl border border-dashed border-border/70 bg-muted/20 p-3 text-center">
            <p className="text-xs leading-relaxed text-muted-foreground">
              {t("accounts.resetNotReported")}
            </p>
          </div>
        ) : info.credits == null ? (
          <div className="rounded-xl border border-dashed border-border/70 bg-muted/20 p-3 text-center">
            <p className="text-xs leading-relaxed text-muted-foreground">
              {t(
                info.availableCount
                  ? "accounts.resetDetailsUnavailable"
                  : "accounts.resetNone",
              )}
            </p>
          </div>
        ) : (
          <div className="space-y-3">
            {available.length === 0 && (
              <div className="rounded-xl border border-dashed border-border/70 bg-muted/20 p-3 text-center">
                <p className="text-xs text-muted-foreground">
                  {t(
                    info.availableCount
                      ? "accounts.resetDetailsIncomplete"
                      : "accounts.resetNone",
                  )}
                </p>
              </div>
            )}

            {/* Voucher Cards */}
            {available.length > 0 && (
              <ul className="space-y-2">
                {available.map((credit, index) => {
                  const daysLeft =
                    credit.expiresAt != null
                      ? Math.ceil((credit.expiresAt - now) / 86400)
                      : null;
                  const isExpiringSoon = daysLeft != null && daysLeft <= 7;

                  return (
                    <li
                      key={index}
                      className="group relative overflow-hidden rounded-xl border border-amber-500/20 bg-gradient-to-br from-amber-500/[0.04] to-orange-500/[0.02] p-3 shadow-xs transition-all hover:border-amber-500/40"
                    >
                      <div className="flex items-start justify-between gap-2">
                        <div className="flex items-center gap-2">
                          <span className="flex h-5 w-5 items-center justify-center rounded-md bg-amber-500/15 font-mono text-[10px] font-bold text-amber-600 dark:text-amber-400">
                            #{index + 1}
                          </span>
                          <span className="text-xs font-semibold text-foreground">
                            {t("accounts.resetCardNumber", { number: index + 1 })}
                          </span>
                        </div>
                        <span
                          className={cn(
                            "rounded-full px-2 py-0.5 text-[10px] font-medium tracking-tight",
                            isExpiringSoon
                              ? "bg-rose-500/10 text-rose-600 dark:text-rose-400"
                              : "bg-emerald-500/10 text-emerald-600 dark:text-emerald-400",
                          )}
                        >
                          {expiry(credit)}
                        </span>
                      </div>

                      <div className="mt-2 flex items-center gap-1.5 text-[11px] text-muted-foreground">
                        <CalendarClock className="h-3 w-3 shrink-0 text-muted-foreground/70" />
                        <span className="truncate">
                          {credit.expiresAt == null
                            ? t("accounts.resetNoExpiry")
                            : t("accounts.resetExpiresAt", {
                                time: date(credit.expiresAt),
                              })}
                        </span>
                      </div>
                    </li>
                  );
                })}
              </ul>
            )}

            {/* History Details Accordion */}
            <details className="group rounded-xl border border-border/50 bg-muted/15 p-2 text-xs transition-all">
              <summary className="flex cursor-pointer items-center justify-between rounded-lg px-1.5 py-1 font-medium text-muted-foreground transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring">
                <span className="flex items-center gap-1.5">
                  <History className="h-3.5 w-3.5" />
                  {t("accounts.resetGrantHistory")} · {t("accounts.resetLast30Days")}
                </span>
                <span className="text-[10px] text-muted-foreground/70 group-open:rotate-180 transition-transform duration-200">
                  ▼
                </span>
              </summary>
              <div className="mt-2 max-h-36 overflow-y-auto space-y-1.5 border-t border-border/40 pt-2 px-1">
                {recent.map((credit, index) => (
                  <div
                    key={index}
                    className="flex items-center justify-between gap-2 text-[11px] text-muted-foreground py-0.5"
                  >
                    <time className="font-mono">{date(credit.grantedAt!)}</time>
                    <span
                      className={cn(
                        "rounded px-1.5 py-0.2 text-[10px] font-medium",
                        credit.status === "available"
                          ? "bg-emerald-500/10 text-emerald-600 dark:text-emerald-400"
                          : credit.status === "redeemed"
                            ? "bg-sky-500/10 text-sky-600 dark:text-sky-400"
                            : "bg-muted text-muted-foreground",
                      )}
                    >
                      {t([
                        `accounts.resetStatus.${credit.status}`,
                        "accounts.resetStatus.unknown",
                      ])}
                    </span>
                  </div>
                ))}
                {recent.length === 0 && (
                  <p className="py-2 text-center text-muted-foreground">
                    {t("accounts.resetNoHistory")}
                  </p>
                )}
              </div>
            </details>
          </div>
        )}

        {/* Footer */}
        <div className="mt-3 flex items-center justify-between gap-2 border-t border-border/50 pt-2.5">
          <div className="flex items-center gap-1 text-[10px] text-muted-foreground">
            <Clock className="h-3 w-3 opacity-60" />
            <span>
              {account.lastUsageAt ? date(account.lastUsageAt) : t("accounts.unknown")}
            </span>
          </div>
          <Button
            variant="ghost"
            size="sm"
            className="h-7 gap-1.5 rounded-lg px-2 text-xs font-medium text-foreground hover:bg-muted"
            disabled={refreshing}
            onClick={onRefresh}
          >
            {refreshing ? (
              <Loader2 className="h-3 w-3 animate-spin text-primary" />
            ) : (
              <RefreshCw className="h-3 w-3 text-muted-foreground" />
            )}
            {t("common.refresh")}
          </Button>
        </div>
      </PopoverContent>
    </Popover>
  );
}
