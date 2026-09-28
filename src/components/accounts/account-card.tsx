import { useState } from "react";
import { useTranslation } from "react-i18next";
import {
  Calendar,
  Check,
  Copy,
  Loader2,
  LogIn,
  RefreshCw,
  Shield,
  ShieldCheck,
  Sparkles,
  Trash2,
  User,
} from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { toast } from "@/hooks/use-toast";
import { cn } from "@/lib/utils";
import type { AccountRateLimitWindow, CodexAccountSummary } from "@/types";

export function planLabel(plan: string, unknown: string) {
  const labels: Record<string, string> = {
    free: "Free",
    plus: "Plus",
    pro5x: "Pro 5x",
    pro20x: "Pro 20x",
    team: "Team",
    business: "Business",
    enterprise: "Enterprise",
    edu: "Edu",
  };
  return labels[plan.toLowerCase()] ?? (plan && plan !== "unknown" ? plan : unknown);
}

function statusStyle(status: CodexAccountSummary["usageStatus"]) {
  if (status === "healthy") return "bg-emerald-500";
  if (status === "expired") return "bg-amber-500";
  if (status === "error") return "bg-destructive";
  return "bg-muted-foreground/40";
}

function QuotaRing({
  label,
  window,
}: {
  label: string;
  window: AccountRateLimitWindow | null;
}) {
  const value = window?.remainingPercent ?? null;
  const radius = 30;
  const circumference = Math.PI * 2 * radius;
  const offset = value == null ? circumference : circumference * (1 - value / 100);
  const tone = value == null
    ? "stroke-muted-foreground/20 text-muted-foreground"
    : value <= 15
      ? "stroke-rose-500 text-rose-500"
      : value <= 35
        ? "stroke-amber-500 text-amber-500"
        : "stroke-emerald-500 text-emerald-500";

  // Parse reset time
  let resetTimeFormatted = "—";
  if (window?.resetsAt) {
    const d = new Date(window.resetsAt * 1000);
    const dateStr = `${d.getMonth() + 1}/${d.getDate()}`;
    const timeStr = d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", hour12: false });
    resetTimeFormatted = `${dateStr} ${timeStr} 重置`;
  }

  return (
    <div className="flex min-w-0 flex-1 flex-col items-center justify-center p-1">
      <div className="relative h-[70px] w-[70px]">
        <svg viewBox="0 0 72 72" className="h-full w-full -rotate-90" aria-hidden>
          <circle cx="36" cy="36" r={radius} fill="none" strokeWidth="5.5" className="stroke-muted/50 dark:stroke-muted/30" />
          <circle
            cx="36"
            cy="36"
            r={radius}
            fill="none"
            strokeWidth="5.5"
            strokeLinecap="round"
            strokeDasharray={circumference}
            strokeDashoffset={offset}
            className={cn("transition-all duration-500", tone)}
          />
        </svg>
        <div className="absolute inset-0 flex items-center justify-center">
          <span className={cn("font-mono text-sm font-bold tracking-tight", tone)}>
            {value == null ? "—" : `${value}%`}
          </span>
        </div>
      </div>
      <div className="mt-1.5 text-center">
        <p className="font-mono text-[11px] font-semibold tracking-tight text-foreground/90">
          <span className="font-bold">{label}</span>
          <span className="mx-1 text-muted-foreground/50">·</span>
          <span className="text-muted-foreground">{resetTimeFormatted}</span>
        </p>
      </div>
    </div>
  );
}

export function AccountCard({
  account,
  switching,
  refreshing,
  onSwitch,
  onDelete,
  onRefresh,
}: {
  account: CodexAccountSummary;
  switching: boolean;
  refreshing: boolean;
  onSwitch: () => void;
  onDelete: () => void;
  onRefresh: () => void;
}) {
  const { t } = useTranslation();
  const [copied, setCopied] = useState(false);
  const statusText = t(`accounts.${account.usageStatus}`);
  const title = account.email || account.displayName;

  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(account.email || account.displayName);
      setCopied(true);
      toast({ title: "已复制账号", description: account.email || account.displayName });
      setTimeout(() => setCopied(false), 2000);
    } catch {
      toast({ title: "复制失败", variant: "destructive" });
    }
  };

  const isPlus = /plus/i.test(account.plan);
  const isTeamOrPro = /pro|team|business|enterprise/i.test(account.plan);

  return (
    <article
      className={cn(
        "group relative flex flex-col justify-between overflow-hidden rounded-2xl border bg-card/90 backdrop-blur-md transition-all duration-300",
        account.isActive
          ? "border-emerald-500/40 ring-1 ring-emerald-500/20 shadow-[0_12px_32px_-12px_rgba(16,185,129,0.18)]"
          : "border-border/80 hover:border-primary/40 hover:shadow-lg hover:-translate-y-0.5",
      )}
    >
      {/* Top Header: Email Title + Status Badge */}
      <div className="px-4 pb-2.5 pt-3.5">
        <div className="flex items-center justify-between gap-2.5">
          <div className="flex min-w-0 flex-1 items-center gap-2">
            <span
              className={cn(
                "h-2.5 w-2.5 shrink-0 rounded-full",
                statusStyle(account.usageStatus),
                account.isActive && "shadow-[0_0_8px_hsl(142,76%,36%)]"
              )}
            />
            <h3
              className="truncate text-sm font-bold tracking-tight text-foreground/95"
              title={title}
            >
              {title}
            </h3>
          </div>
        </div>

        {/* Badges line: Plan Badge + Active Badge */}
        <div className="mt-2 flex flex-wrap items-center gap-1.5">
          <Badge
            variant="outline"
            className={cn(
              "gap-1 rounded-full px-2 py-0.5 text-[11px] font-semibold tracking-wide",
              isPlus && "border-emerald-500/30 bg-emerald-500/10 text-emerald-700 dark:text-emerald-400",
              isTeamOrPro && "border-primary/30 bg-primary/10 text-primary",
              !isPlus && !isTeamOrPro && "border-border bg-muted/60 text-muted-foreground",
            )}
          >
            <Sparkles className="h-3 w-3" />
            {planLabel(account.plan, t("accounts.planUnknown"))}
          </Badge>

          {account.isActive && (
            <Badge className="gap-1 rounded-full border border-emerald-500/25 bg-emerald-500/15 px-2 py-0.5 text-[11px] font-medium text-emerald-700 shadow-none dark:text-emerald-300">
              <span className="h-1.5 w-1.5 rounded-full bg-emerald-500" />
              {t("accounts.current")}
            </Badge>
          )}
        </div>
      </div>

      {/* Middle: 2x2 Info Grid with clean icons */}
      <div className="grid grid-cols-2 gap-2.5 border-y border-border/50 bg-black/[0.015] px-4 py-2.5 text-xs dark:bg-white/[0.015]">
        {/* User Nickname */}
        <div className="flex min-w-0 items-start gap-2">
          <div className="flex h-6 w-6 shrink-0 items-center justify-center rounded-md bg-muted/60 text-muted-foreground">
            <User className="h-3.5 w-3.5" />
          </div>
          <div className="min-w-0 flex-1">
            <p className="text-[11px] text-muted-foreground">用户昵称</p>
            <p className="truncate font-semibold text-foreground/90" title={account.displayName}>
              {account.displayName || "—"}
            </p>
          </div>
        </div>

        {/* Auth Mode */}
        <div className="flex min-w-0 items-start gap-2">
          <div className="flex h-6 w-6 shrink-0 items-center justify-center rounded-md bg-muted/60 text-muted-foreground">
            <Shield className="h-3.5 w-3.5" />
          </div>
          <div className="min-w-0 flex-1">
            <p className="text-[11px] text-muted-foreground">认证模式</p>
            <p className="truncate font-semibold text-foreground/90">
              {t("accounts.chatgptOauth")}
            </p>
          </div>
        </div>

        {/* Subscription Status */}
        <div className="flex min-w-0 items-start gap-2">
          <div className="flex h-6 w-6 shrink-0 items-center justify-center rounded-md bg-muted/60 text-muted-foreground">
            <ShieldCheck className="h-3.5 w-3.5" />
          </div>
          <div className="min-w-0 flex-1">
            <p className="text-[11px] text-muted-foreground">订阅状态</p>
            <p className="truncate font-semibold text-foreground/90">
              {account.usageStatus === "healthy" ? "有效订阅" : statusText}
            </p>
          </div>
        </div>

        {/* Expiry or Last checked */}
        <div className="flex min-w-0 items-start gap-2">
          <div className="flex h-6 w-6 shrink-0 items-center justify-center rounded-md bg-muted/60 text-muted-foreground">
            <Calendar className="h-3.5 w-3.5" />
          </div>
          <div className="min-w-0 flex-1">
            <p className="text-[11px] text-muted-foreground">最近同步</p>
            <p className="truncate font-semibold text-foreground/90" title={account.lastUsageAt ? new Date(account.lastUsageAt * 1000).toLocaleString() : undefined}>
              {account.lastUsageAt
                ? new Date(account.lastUsageAt * 1000).toLocaleString()
                : t("accounts.unknown")}
            </p>
          </div>
        </div>
      </div>

      {/* Quota Ring Dual Displays */}
      <div className="flex items-center justify-around divide-x divide-border/50 px-2 py-2.5">
        <QuotaRing label="5小时" window={account.primaryWindow} />
        <QuotaRing label="1周" window={account.secondaryWindow} />
      </div>

      {account.usageError && (
        <p className="mx-4 mb-3 rounded-xl bg-destructive/10 px-3 py-2 text-[11px] leading-relaxed text-destructive border border-destructive/20">
          {account.usageError}
        </p>
      )}

      {/* Footer Actions */}
      <div className="flex items-center justify-between border-t border-border/50 bg-black/[0.015] px-4 py-2.5 dark:bg-white/[0.015]">
        {/* Left: Active indicator or Set Active Button */}
        <div>
          {account.isActive ? (
            <div className="flex items-center gap-1.5 font-semibold text-emerald-600 dark:text-emerald-400 text-xs">
              <Check className="h-4 w-4" />
              <span>当前账号</span>
            </div>
          ) : (
            <Button
              variant="outline"
              size="xs"
              className="h-7 gap-1.5 rounded-lg border-border/80 px-2.5 text-[11px] font-semibold shadow-2xs transition-all hover:border-primary/40 hover:bg-primary/5 hover:text-primary"
              onClick={onSwitch}
              disabled={switching}
            >
              {switching ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <LogIn className="h-3.5 w-3.5" />}
              <span>{switching ? t("accounts.switching") : t("accounts.setCurrent")}</span>
            </Button>
          )}
        </div>

        {/* Right: Refresh Quota Button, Copy Button, Delete Button in soft pill capsule */}
        <div className="flex items-center gap-1 rounded-2xl bg-muted/50 p-1 border border-border/50">
          <Button
            variant="ghost"
            size="icon-sm"
            className="h-7 w-7 rounded-xl text-muted-foreground hover:bg-background hover:text-foreground hover:shadow-xs transition-all"
            onClick={onRefresh}
            disabled={refreshing}
            title="刷新该账号额度"
          >
            <RefreshCw className={cn("h-3.5 w-3.5", refreshing && "animate-spin text-primary")} />
          </Button>

          <Button
            variant="ghost"
            size="icon-sm"
            className="h-7 w-7 rounded-xl text-muted-foreground hover:bg-background hover:text-foreground hover:shadow-xs transition-all"
            onClick={handleCopy}
            title="复制账号名称"
          >
            {copied ? <Check className="h-3.5 w-3.5 text-emerald-500" /> : <Copy className="h-3.5 w-3.5" />}
          </Button>

          <Button
            variant="ghost"
            size="icon-sm"
            className="h-7 w-7 rounded-xl text-muted-foreground hover:bg-destructive/15 hover:text-destructive hover:shadow-xs transition-all"
            onClick={onDelete}
            disabled={account.isActive}
            title={account.isActive ? t("accounts.activeDeleteBlocked") : t("accounts.remove")}
          >
            <Trash2 className="h-3.5 w-3.5" />
          </Button>
        </div>
      </div>
    </article>
  );
}
