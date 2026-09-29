import { useTranslation } from "react-i18next";
import { Wallet, ChevronLeft, ChevronRight, Layers, List, Loader2, Network, PlugZap, RadioTower, RefreshCw, SquarePen, Trash2 } from "lucide-react";
import { balanceSourceName } from "@/lib/provider-balance";
import { api } from "@/lib/api";
import { cn } from "@/lib/utils";
import { Badge } from "@/components/ui/badge";
import { Switch } from "@/components/ui/switch";
import { AuroraCard } from "@/components/ui/aurora-card";
import { toast } from "@/hooks/use-toast";
import {
  DropdownMenu, DropdownMenuContent, DropdownMenuRadioGroup, DropdownMenuRadioItem, DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { includedModels, providerModelProtocol } from "@/lib/provider-models";
import {
  cardButton, deepSeekPeakNow, hostOf, iconButton, protocolBadgeKey, providerColor, softBadge, type RelayGroup,
} from "@/components/newapi/relay-groups";
import { StatusDot } from "@/components/newapi/status-dot";
import type { ProviderState } from "@/components/newapi/use-provider-state";
import type { RelayView } from "@/components/newapi/use-relay-view";
import type { NewApiProvider } from "@/types/newapi";

// One relay: the current member's card face, stacked peek layers for the other
// members, and the expanded member list.
export function RelayCard({ group, store, view, onEdit, onTest, onRemove, onBalance }: {
  group: RelayGroup;
  store: ProviderState;
  view: RelayView;
  onEdit: (provider: NewApiProvider) => void;
  onTest: (provider: NewApiProvider) => void;
  onRemove: (provider: NewApiProvider) => void;
  onBalance: (provider: NewApiProvider) => void;
}) {
  const { t, i18n } = useTranslation();
  const { mutationLocked, run, applyChange, balanceBusy, refreshBalance } = store;
  const { filter, groupLabel, renameGroup, memberSelection, memberDirection, expandedGroups, selectMember, setExpanded, toggleExpanded, finishFlip } = view;
  const routingEnabled = !!store.state.data?.router?.enabled;
  const activeId = store.state.data?.activeProviderId ?? null;

  const formatMoney = (value: number) => new Intl.NumberFormat(i18n.language, { maximumFractionDigits: 2 }).format(value);
  const balanceValue = (provider: NewApiProvider) => {
    const balance = provider.balance;
    if (!balance) return null;
    if (!balance.conversionAvailable) return `${formatMoney(balance.rawQuota)} ${t("newapi.balanceQuota")}`;
    const symbol = balance.unit === "USD" ? "$" : balance.unit === "CNY" ? "¥" : "";
    return `${symbol}${formatMoney(balance.remaining)}${symbol ? "" : ` ${balance.unit}`}`.trim();
  };
  const balanceLabel = (provider: NewApiProvider) =>
    provider.balance && provider.balanceSource !== "deepseek" ? t("newapi.balanceRemaining") : t("newapi.balance");
  const balanceCaption = (provider: NewApiProvider) => {
    if (!provider.balance) return t("newapi.balanceNotQueried");
    return balanceSourceName(provider.balanceSource) || t("newapi.balanceUndetected");
  };

  const activeMemberId = memberSelection[group.key] ?? (group.providers.find(item => item.id === activeId)?.id ?? group.providers[0].id);
  const provider = group.providers.find(item => item.id === activeMemberId) ?? group.providers[0];
  const isGroupActive = group.providers.some(item => item.id === activeId) && routingEnabled;
  const isCurrentActive = provider.id === activeId && routingEnabled;
  const selected = filter === group.key;
  const dsPeak = hostOf(provider) === "api.deepseek.com" ? deepSeekPeakNow() : null;
  const value = balanceValue(provider);
  const valuePositive = !!provider.balance && provider.balance.remaining > 0;
  const hasMultiple = group.providers.length > 1;
  const isExpanded = !!expandedGroups[group.key];
  const cardColor = isGroupActive ? "gold" : providerColor(group.key);
  const memberIndex = group.providers.findIndex(item => item.id === provider.id);
  const switchMember = (offset: number) => {
    const nextIndex = (memberIndex + offset + group.providers.length) % group.providers.length;
    selectMember(group, group.providers[nextIndex].id, offset < 0 ? "prev" : "next");
  };

  return (
    <div className="group relative flex min-w-0 flex-col">
      {/* 3D Physical Card Deck Peek Layers for Same-Relay Groups */}
      {hasMultiple && !isExpanded && (
        <>
          <div
            aria-hidden
            className={cn(
              "pointer-events-none absolute -inset-x-1.5 -bottom-2 top-3 rounded-2xl border transition-all duration-300",
              cardColor === "gold"
                ? "border-[#d4af37]/25 bg-amber-950/15 dark:bg-[#14120e]/80 shadow-[0_8px_20px_-8px_rgba(212,175,55,0.15)]"
                : "border-border/60 bg-card/75 dark:bg-card/60 shadow-sm",
              "group-hover:-bottom-2.5 group-hover:-inset-x-2",
            )}
          />
          {group.providers.length > 2 && (
            <div
              aria-hidden
              className={cn(
                "pointer-events-none absolute -inset-x-3 -bottom-3.5 top-6 rounded-2xl border transition-all duration-300",
                cardColor === "gold"
                  ? "border-[#d4af37]/15 bg-amber-950/10 dark:bg-[#0f0e0b]/60"
                  : "border-border/40 bg-card/45 dark:bg-card/35 shadow-xs",
                "group-hover:-bottom-4 group-hover:-inset-x-3",
              )}
            />
          )}
        </>
      )}

      <AuroraCard
        key={provider.id}
        onAnimationEnd={event => { if (event.target === event.currentTarget) finishFlip(group); }}
        data-relay-card-face
        color={cardColor}
        shine={isGroupActive || selected}
        className={cn(
          "flex flex-col",
          selected && "rounded-2xl ring-2 ring-primary/40",
          hasMultiple && !isExpanded && memberDirection[group.key] &&
            (memberDirection[group.key] === "prev" ? "relay-card-swap-prev" : "relay-card-swap-next"),
        )}
      >
        <div className="flex flex-col justify-between p-3.5 sm:p-4">
          <div className="flex flex-col">
            {/* Header: Title + Domain + Badges + Switch */}
            <div className="flex items-center justify-between gap-2">
              <div className="flex min-w-0 flex-1 items-center gap-1.5">
                <StatusDot provider={provider} />
                <h3 className="truncate text-[15px] font-semibold tracking-tight" title={groupLabel(group, provider)}>
                  {groupLabel(group, provider)}
                </h3>
                {hasMultiple && (
                  <button
                    type="button"
                    className="inline-flex shrink-0 items-center gap-1 rounded-full bg-primary/10 px-1.5 py-0.5 text-[10px] font-medium text-primary hover:bg-primary/20"
                    onClick={() => toggleExpanded(group)}
                    title={isExpanded ? t("newapi.collapseStack") : t("newapi.expandAll")}
                    aria-label={isExpanded ? t("newapi.collapseStack") : t("newapi.expandAll")}
                    aria-expanded={isExpanded}
                  >
                    <Layers className="h-2.5 w-2.5" />
                    {group.providers.length}
                  </button>
                )}
              </div>
              <div className="flex shrink-0 items-center gap-1.5">
                {hasMultiple && (
                  <div className="relay-member-nav" role="group" aria-label={t("newapi.groupMembers")}>
                    <button type="button" onClick={() => switchMember(-1)} aria-label={t("newapi.previousGroupMember")} title={t("newapi.previousGroupMember")}>
                      <ChevronLeft className="h-3.5 w-3.5" />
                    </button>
                    <span className="relay-member-name" title={provider.name} aria-live="polite">
                      {provider.name} <small>{memberIndex + 1}/{group.providers.length}</small>
                    </span>
                    <button type="button" onClick={() => switchMember(1)} aria-label={t("newapi.nextGroupMember")} title={t("newapi.nextGroupMember")}>
                      <ChevronRight className="h-3.5 w-3.5" />
                    </button>
                  </div>
                )}
                <Switch
                  checked={provider.enabled}
                  className={isCurrentActive ? "data-[state=checked]:bg-amber-500" : undefined}
                  disabled={mutationLocked}
                  onCheckedChange={value =>
                    void run(`enable:${provider.id}`, async () => {
                      const change = await api.providers.setEnabled(provider.id, value);
                      applyChange(change);
                    }, t("newapi.enableFailed"))
                  }
                />
              </div>
            </div>

            {/* Dual Info Cards: Balance (Left) & Model/Status (Right) */}
            <div className="mt-3 grid grid-cols-2 gap-2">
              {/* Left: Balance Box */}
              <div className={cn(
                "flex flex-col justify-between rounded-xl p-2.5 ring-1 transition-colors min-w-0 min-h-[92px]",
                isCurrentActive
                  ? "bg-amber-500/[0.08] ring-amber-500/30"
                  : "bg-black/[0.025] ring-border/50 dark:bg-white/[0.03]",
              )}>
                <div className="flex items-center justify-between gap-1">
                  <span className="text-[11px] font-medium text-muted-foreground truncate">{balanceLabel(provider)}</span>
                  <button
                    type="button"
                    className={iconButton}
                    title={t("newapi.refreshBalance")}
                    aria-label={t("newapi.refreshBalance")}
                    disabled={balanceBusy.has(provider.id) || !(provider.hasApiKey || provider.hasAccessToken)}
                    onClick={() => void refreshBalance(provider.id)}
                  >
                    {balanceBusy.has(provider.id) ? <Loader2 className="h-3 w-3 animate-spin" /> : <RefreshCw className="h-3 w-3" />}
                  </button>
                </div>
                <div className="my-auto py-0.5">
                  <span className={cn(
                    "truncate text-base font-semibold tabular-nums tracking-tight block",
                    isCurrentActive && valuePositive && "text-emerald-600 dark:text-emerald-400",
                    cardColor === "gold" && "dark:text-[#fef08a]"
                  )}>
                    {value ?? "— —"}
                  </span>
                </div>
                <p className="truncate text-[10px] text-muted-foreground/80" title={provider.balance ? `${balanceCaption(provider)} · ${new Date(provider.balance.checkedAt * 1000).toLocaleString()}` : balanceCaption(provider)}>
                  {store.balanceErrors[provider.id] ? t("newapi.balanceFailed") : balanceCaption(provider)}
                </p>
              </div>

              {/* Right: Model & Connection Box */}
              <div className={cn(
                "flex flex-col justify-between rounded-xl p-2.5 ring-1 transition-colors min-w-0 min-h-[92px]",
                isCurrentActive
                  ? "bg-amber-500/[0.08] ring-amber-500/30"
                  : "bg-black/[0.025] ring-border/50 dark:bg-white/[0.03]",
              )}>
                <div className="flex items-center justify-between gap-1">
                  <span className="text-[11px] font-medium text-muted-foreground truncate">
                    {t("newapi.model")}
                  </span>
                  <div className="flex items-center gap-1 shrink-0">
                    {provider.testStatus === "untested" && <Badge className={softBadge("gray")}>{t("newapi.badgeUntested")}</Badge>}
                    {provider.testStatus === "ok" && <Badge className={softBadge("green")}>{t("newapi.badgeOk")}</Badge>}
                    {provider.testStatus === "failed" && <Badge className={softBadge("red")}>{t("newapi.badgeFailed")}</Badge>}
                    {dsPeak !== null && (
                      <Badge className={softBadge(dsPeak ? "amber" : "green")} title={t("newapi.deepSeekPeakTip")}>
                        {t(dsPeak ? "newapi.peakNow" : "newapi.offPeakNow")}
                      </Badge>
                    )}
                  </div>
                </div>
                <div className="my-auto flex min-w-0 items-center gap-1.5 py-0.5">
                  <List className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                  <span className="truncate font-mono text-xs font-medium" title={provider.model || t("newapi.noModel")}>
                    {provider.model || t("newapi.noModel")}
                  </span>
                </div>
                <div className="flex items-center justify-between gap-1 text-[10px] text-muted-foreground">
                  <span className="truncate">
                    {t("newapi.modelCount", { count: includedModels(provider.models).length })}
                  </span>
                  <span className="shrink-0 rounded bg-muted px-1 py-0.2 font-mono text-[9px] text-muted-foreground">
                    {t(protocolBadgeKey[providerModelProtocol(provider)])}
                  </span>
                </div>
              </div>
            </div>

            {/* Expanded Mode: Cascaded clean cards for all providers in this relay */}
            {hasMultiple && isExpanded && (
              <div className="mt-3 space-y-2 rounded-xl bg-black/[0.025] p-2.5 ring-1 ring-border/50 dark:bg-white/[0.03]">
                <div className="flex items-center justify-between gap-2 px-1 text-[11px] font-medium text-muted-foreground">
                  <div className="flex items-center gap-1.5">
                    <Layers className="h-3.5 w-3.5 text-primary" />
                    <span>{t("newapi.groupMembers")} ({group.providers.length})</span>
                  </div>
                  <div className="flex items-center gap-1">
                    <button
                      type="button"
                      className="rounded px-1.5 py-0.5 text-xs text-primary hover:bg-primary/10"
                      onClick={() => renameGroup(group)}
                    >
                      {t("newapi.renameGroup")}
                    </button>
                    <button
                      type="button"
                      className="rounded px-1.5 py-0.5 text-xs text-muted-foreground hover:bg-muted"
                      onClick={() => setExpanded(group, false)}
                    >
                      {t("newapi.collapseStack")}
                    </button>
                  </div>
                </div>
                <div className="space-y-1.5">
                  {group.providers.map(item => (
                    <div
                      key={item.id}
                      className={cn(
                        "flex flex-wrap items-center justify-between gap-2 rounded-lg border bg-card/80 p-2 text-xs transition-colors hover:bg-muted/50",
                        item.id === provider.id && "ring-1 ring-primary/40",
                      )}
                    >
                      <div className="flex min-w-0 flex-1 items-center gap-2">
                        <StatusDot provider={item} />
                        <button
                          type="button"
                          className="max-w-[120px] truncate rounded font-semibold text-foreground hover:text-primary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary/35"
                          title={item.name}
                          onClick={() => {
                            if (item.id !== provider.id) selectMember(group, item.id, group.providers.indexOf(item) < memberIndex ? "prev" : "next");
                            setExpanded(group, false);
                          }}
                        >{item.name}</button>
                        <span className="font-mono text-[11px] text-muted-foreground truncate max-w-[110px]" title={item.model}>{item.model || t("newapi.noModel")}</span>
                        {item.id === activeId && routingEnabled && (
                          <span className="inline-flex items-center gap-0.5 rounded bg-amber-500/10 px-1 py-0.2 text-[10px] font-medium text-amber-600 dark:text-amber-400">
                            <PlugZap className="h-2.5 w-2.5" />
                            {t("newapi.isCurrentRoute")}
                          </span>
                        )}
                      </div>
                      <div className="flex shrink-0 items-center gap-1.5">
                        <button type="button" className={cardButton} aria-label={`${t("newapi.queryBalance")} ${item.name}`} onClick={() => onBalance(item)}><Wallet className="h-3 w-3" /></button>
                        <Switch
                          checked={item.enabled}
                          disabled={mutationLocked}
                          onCheckedChange={value =>
                            void run(`enable:${item.id}`, async () => applyChange(await api.providers.setEnabled(item.id, value)), t("newapi.enableFailed"))
                          }
                        />
                        <button
                          type="button"
                          className={cardButton}
                          title={t("newapi.test")}
                          aria-label={t("newapi.test")}
                          disabled={!item.hasApiKey}
                          onClick={() => onTest(item)}
                        >
                          <RadioTower className="h-3 w-3" />
                        </button>
                        <button
                          type="button"
                          className={cardButton}
                          title={t("newapi.edit")}
                          aria-label={t("newapi.edit")}
                          onClick={() => onEdit(item)}
                        >
                          <SquarePen className="h-3 w-3" />
                        </button>
                        <button
                          type="button"
                          className={cn(cardButton, "hover:bg-red-500/10 hover:text-red-600 dark:hover:text-red-400")}
                          title={t("newapi.remove")}
                          aria-label={t("newapi.remove")}
                          onClick={() => onRemove(item)}
                        >
                          <Trash2 className="h-3 w-3" />
                        </button>
                      </div>
                    </div>
                  ))}
                </div>
              </div>
            )}
          </div>

          {/* Footer: Status / Hint & Action Buttons */}
          <div className="mt-3 flex items-center justify-between gap-2 border-t border-border/40 pt-2.5">
            <div className="flex min-w-0 items-center gap-1.5">
              {isCurrentActive ? (
                <span className="inline-flex items-center gap-1 rounded-full bg-amber-500/15 px-2 py-0.5 text-[11px] font-medium text-amber-700 dark:text-amber-300">
                  <PlugZap className="h-3 w-3" />
                  {t("newapi.isCurrentRoute")}
                </span>
              ) : (
                <span className="truncate font-mono text-[11px] text-muted-foreground/80">
                  http://{hostOf(provider)}
                </span>
              )}
            </div>
            <div className="flex shrink-0 items-center gap-1">
              <button type="button" className={cn(cardButton, "w-auto gap-1.5 whitespace-nowrap px-2")} onClick={() => onBalance(provider)} aria-label={`${t("newapi.queryBalance")} ${provider.name}`}>
                <Wallet className="h-3.5 w-3.5" /><span className="text-[11px]">{t("newapi.queryBalance")}</span>
              </button>
              <DropdownMenu>
                <DropdownMenuTrigger asChild>
                  <button
                    type="button"
                    className={cardButton}
                    title={t("newapi.networkMode")}
                    aria-label={t("newapi.networkMode")}
                    disabled={mutationLocked}
                  >
                    <Network className={cn("h-3.5 w-3.5", provider.networkMode === "direct" ? "text-emerald-600 dark:text-emerald-400" : "text-blue-600 dark:text-blue-400")} />
                  </button>
                </DropdownMenuTrigger>
                <DropdownMenuContent align="end" className="w-52">
                  <DropdownMenuRadioGroup
                    value={provider.networkMode}
                    onValueChange={mode => {
                      if (mode !== "proxy" && mode !== "direct") return;
                      void run(`network:${provider.id}`, async () => {
                        applyChange(await api.providers.setNetwork(provider.id, mode));
                        toast({
                          title: t("newapi.networkUpdated"),
                          description: t(mode === "direct" ? "newapi.networkDirectHint" : "newapi.networkProxyHint"),
                          variant: "success",
                        });
                      }, t("newapi.networkFailed"));
                    }}
                  >
                    <DropdownMenuRadioItem value="proxy">
                      <PlugZap className="mr-2 h-4 w-4" />
                      <span>
                        <span className="block">{t("newapi.networkProxy")}</span>
                        <span className="block text-[10px] text-muted-foreground">{t("newapi.networkProxyShort")}</span>
                      </span>
                    </DropdownMenuRadioItem>
                    <DropdownMenuRadioItem value="direct">
                      <Network className="mr-2 h-4 w-4" />
                      <span>
                        <span className="block">{t("newapi.networkDirect")}</span>
                        <span className="block text-[10px] text-muted-foreground">{t("newapi.networkDirectShort")}</span>
                      </span>
                    </DropdownMenuRadioItem>
                  </DropdownMenuRadioGroup>
                </DropdownMenuContent>
              </DropdownMenu>
              <button
                type="button"
                className={cardButton}
                title={t("newapi.renameGroup")}
                aria-label={t("newapi.renameGroup")}
                onClick={() => renameGroup(group)}
              >
                <SquarePen className="h-3.5 w-3.5" />
              </button>
              <button
                type="button"
                className={cardButton}
                title={t("newapi.test")}
                aria-label={t("newapi.test")}
                disabled={!provider.hasApiKey}
                onClick={() => onTest(provider)}
              >
                <RadioTower className="h-3.5 w-3.5" />
              </button>
              <button
                type="button"
                className={cardButton}
                title={t("newapi.edit")}
                aria-label={t("newapi.edit")}
                disabled={mutationLocked}
                onClick={() => onEdit(provider)}
              >
                <SquarePen className="h-3.5 w-3.5" />
              </button>
              <button
                type="button"
                className={cn(cardButton, "hover:bg-red-500/10 hover:text-red-600 dark:hover:text-red-400")}
                title={t("newapi.remove")}
                aria-label={t("newapi.remove")}
                disabled={mutationLocked}
                onClick={() => onRemove(provider)}
              >
                <Trash2 className="h-3.5 w-3.5" />
              </button>
            </div>
          </div>
        </div>
      </AuroraCard>
    </div>
  );
}
