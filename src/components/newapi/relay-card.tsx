import type { CSSProperties } from "react";
import { useTranslation } from "react-i18next";
import {
  ChevronLeft, ChevronRight, Layers, Loader2, Network, PlugZap, RadioTower, RefreshCw, SquarePen, Tag, Trash2, Wallet,
} from "lucide-react";
import { balanceSourceName, isMimoBalanceSite } from "@/lib/provider-balance";
import { api } from "@/lib/api";
import { cn } from "@/lib/utils";
import { Switch } from "@/components/ui/switch";
import { toast } from "@/hooks/use-toast";
import {
  DropdownMenu, DropdownMenuContent, DropdownMenuRadioGroup, DropdownMenuRadioItem, DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { includedModels, providerModelProtocol } from "@/lib/provider-models";
import { cardButton, deepSeekPeakNow, hostOf, protocolBadgeKey, relayHue, type RelayGroup } from "@/components/newapi/relay-groups";
import { StatusDot } from "@/components/newapi/status-dot";
import { RelayDeck } from "@/components/newapi/relay-deck";
import { useRelayDeck } from "@/components/newapi/use-relay-deck";
import { ClientGlyph } from "@/components/newapi/configuration-shortcuts";
import type { ProviderState } from "@/components/newapi/use-provider-state";
import type { RelayView } from "@/components/newapi/use-relay-view";
import type { RelayConsumers } from "@/components/newapi/use-relay-consumers";
import type { NewApiProvider } from "@/types/newapi";

const pill = "inline-flex h-5 shrink-0 items-center gap-1 rounded-full px-2 text-[10.5px] font-medium";
const testTone = {
  ok: "bg-emerald-500/15 text-emerald-700 dark:text-emerald-400",
  failed: "bg-red-500/10 text-red-600 dark:text-red-400",
  untested: "bg-muted text-muted-foreground",
} as const;
const testLabel = { ok: "newapi.badgeOk", failed: "newapi.badgeFailed", untested: "newapi.badgeUntested" } as const;

// One relay site: the displayed member's face, a stack of peeking layers for
// the other members, and the expanded member list.
export function RelayCard({ group, store, view, consumers, onEdit, onTest, onRemove, onBalance }: {
  group: RelayGroup;
  store: ProviderState;
  view: RelayView;
  consumers: RelayConsumers;
  onEdit: (provider: NewApiProvider) => void;
  onTest: (provider: NewApiProvider) => void;
  onRemove: (provider: NewApiProvider) => void;
  onBalance: (provider: NewApiProvider) => void;
}) {
  const { t, i18n } = useTranslation();
  const { mutationLocked, run, applyChange, balanceBusy, refreshBalance } = store;
  const { filter, groupLabel, renameGroup, memberSelection, memberDirection, expandedGroups, selectMember, setExpanded, toggleExpanded, finishFlip } = view;

  const formatMoney = (value: number) => new Intl.NumberFormat(i18n.language, { maximumFractionDigits: 2 }).format(value);
  const balanceValue = (provider: NewApiProvider) => {
    const balance = provider.balance;
    if (!balance) return null;
    if (!balance.conversionAvailable) return `${formatMoney(balance.rawQuota)} ${t("newapi.balanceQuota")}`;
    const symbol = balance.unit === "USD" || (provider.balanceSource === "commandCode" && balance.unit === "credits") ? "$" : balance.unit === "CNY" ? "¥" : "";
    return `${symbol}${formatMoney(balance.remaining)}${symbol ? "" : ` ${balance.unit}`}`.trim();
  };
  const balanceLabel = (provider: NewApiProvider) => {
    const entries = provider.balance?.entries;
    if (provider.balanceSource === "commandCode" && entries?.length && entries.every(entry => entry.label === "fiveHour" || entry.label === "weekly")) {
      return t("newapi.balanceWindowRemaining");
    }
    return provider.balance && provider.balanceSource !== "deepseek" ? t("newapi.balanceRemaining") : t("newapi.balance");
  };
  const balanceCaption = (provider: NewApiProvider) => {
    if (isMimoBalanceSite(provider.baseUrl) && !provider.balanceScript && !provider.balance) return t("newapi.balanceMimoShort");
    if (!provider.balance) return t("newapi.balanceNotQueried");
    if (provider.balanceSource === "commandCode") {
      const plan = provider.balance.entries?.find(entry => entry.label?.startsWith("Command Code"));
      if (plan?.label) return plan.label;
      const window = provider.balance.entries?.[0];
      if (window?.label === "fiveHour" || window?.label === "weekly") return t(`newapi.balanceWindow.${window.label}`);
    }
    return balanceSourceName(provider.balanceSource) || t("newapi.balanceUndetected");
  };
  const toggleEnabled = (item: NewApiProvider, value: boolean) =>
    void run(`enable:${item.id}`, async () => applyChange(await api.providers.setEnabled(item.id, value)), t("newapi.enableFailed"));

  // Prefer a member Codex currently uses, so the face shows the live one first.
  const preferred = group.providers.find(item => consumers.get(item.id)?.codex) ?? group.providers[0];
  const activeMemberId = memberSelection[group.key] ?? preferred.id;
  const provider = group.providers.find(item => item.id === activeMemberId) ?? group.providers[0];
  const use = consumers.get(provider.id);
  const balanceOwner = provider.balanceSharedFrom
    ? store.state.data?.providers.find(item => item.id === provider.balanceSharedFrom) : provider;
  const balanceReady = !!balanceOwner && (!isMimoBalanceSite(balanceOwner.baseUrl) || !!balanceOwner.balanceScript) && (balanceOwner.hasApiKey || balanceOwner.hasAccessToken || !!balanceOwner.balanceScript);
  const live = !!use?.codex;
  const filtered = filter === group.key;
  const dsPeak = hostOf(provider) === "api.deepseek.com" ? deepSeekPeakNow() : null;
  const value = balanceValue(provider);
  const hasMultiple = group.providers.length > 1;
  const isExpanded = !!expandedGroups[group.key];
  const memberIndex = group.providers.findIndex(item => item.id === provider.id);
  const stacked = hasMultiple && !isExpanded;
  const label = groupLabel(group, provider);
  const others = (use?.consumers ?? []).filter(consumer => consumer !== "codex");
  const { turn, exitDirection } = useRelayDeck({
    activeId: provider.id, memberIds: group.providers.map(item => item.id), stacked,
    onSelect: (id, direction) => selectMember(group, id, direction),
  });
  const switchMember = (offset: number) => {
    const nextIndex = (memberIndex + offset + group.providers.length) % group.providers.length;
    turn(group.providers[nextIndex].id, offset < 0 ? "prev" : "next");
  };

  return (
    <RelayDeck count={group.providers.length} stacked={stacked} hue={relayHue(group.key)}
      label={`${groupLabel(group)} · ${t("newapi.nextGroupMember")}`} onNext={switchMember}>
      <article
        onAnimationEnd={event => {
          // A queued exit animationend may arrive after the timer selected the
          // next member. Only the current entrance can finish its flip intent.
          if (event.target === event.currentTarget && !exitDirection
            && event.animationName === `relay-card-flip-${memberDirection[group.key]}`) finishFlip(group);
        }}
        data-relay-card-face
        data-spotlight
        data-provider-id={provider.id}
        data-codex={use?.codex ?? undefined}
        data-deck-exiting={exitDirection || undefined}
        style={{ "--relay-hue": relayHue(group.key) } as CSSProperties}
        className={cn(
          "relay-face relative flex flex-col overflow-hidden rounded-[20px] border bg-card shadow-sm transition-[border-color,box-shadow] duration-300 hover:shadow-lg motion-reduce:transition-none",
          live ? "relay-face-live border-primary/35" : "border-border",
          !provider.enabled && "relay-face-off",
          filtered && "ring-2 ring-primary/35",
          stacked && exitDirection && `relay-card-exit-${exitDirection}`,
          stacked && !exitDirection && memberDirection[group.key] &&
            (memberDirection[group.key] === "prev" ? "relay-card-swap-prev" : "relay-card-swap-next"),
        )}
      >
        <span aria-hidden className="relay-face-strip" />
        <div className="flex flex-col gap-3 p-4">
          {/* Identity: monogram, site and member, power switch */}
          <div className="relay-card-header flex items-start gap-3" data-has-members={hasMultiple || undefined}>
            <span aria-hidden className="relay-monogram">{label.replace(/^www\./, "").charAt(0).toUpperCase()}</span>
            <div className="relay-card-heading min-w-0 flex-1">
              <div className="flex min-w-0 items-center gap-1.5">
                <h3 className="truncate font-display text-[16px] font-semibold leading-6 tracking-tight" title={label}>{label}</h3>
                {hasMultiple && (
                  <button type="button" onClick={() => toggleExpanded(group)}
                    className="inline-flex h-5 shrink-0 items-center gap-1 rounded-full bg-primary/10 px-1.5 text-[10px] font-semibold text-primary transition-colors hover:bg-primary/20"
                    title={isExpanded ? t("newapi.collapseStack") : t("newapi.expandAll")}
                    aria-label={isExpanded ? t("newapi.collapseStack") : t("newapi.expandAll")} aria-expanded={isExpanded}>
                    <Layers className="h-2.5 w-2.5" />{group.providers.length}
                  </button>
                )}
              </div>
              {hasMultiple ? (
                <div className="relay-member-nav" role="group" data-relay-no-turn aria-label={t("newapi.groupMembers")}>
                  <button type="button" aria-disabled={!!exitDirection} onClick={() => switchMember(-1)} aria-label={t("newapi.previousGroupMember")} title={t("newapi.previousGroupMember")}>
                    <ChevronLeft className="h-3.5 w-3.5" />
                  </button>
                  <span className="relay-member-name" title={provider.name} aria-live="polite">
                    {provider.name} <small>{memberIndex + 1}/{group.providers.length}</small>
                  </span>
                  <button type="button" aria-disabled={!!exitDirection} onClick={() => switchMember(1)} aria-label={t("newapi.nextGroupMember")} title={t("newapi.nextGroupMember")}>
                    <ChevronRight className="h-3.5 w-3.5" />
                  </button>
                  <span aria-hidden className="relay-member-dots">
                    {group.providers.slice(0, 6).map((item, index) => (
                      <i key={item.id} data-current={index === memberIndex || undefined} />
                    ))}
                  </span>
                </div>
              ) : (
                <p className="flex h-6 min-w-0 items-center gap-1.5 text-xs text-muted-foreground">
                  <StatusDot provider={provider} />
                  <span className="truncate" title={provider.name}>{provider.name}</span>
                </p>
              )}
            </div>
            <Switch checked={provider.enabled} disabled={mutationLocked} aria-label={`${t("newapi.relayEnable")} ${provider.name}`}
              className="mt-0.5" onCheckedChange={value => toggleEnabled(provider, value)} />
          </div>

          {/* Who uses this relay right now */}
          <div className="relay-chips flex h-5 min-w-0 items-center gap-1.5 overflow-hidden">
            {use?.codex === "injected" && (
              <span className={cn(pill, "bg-primary/10 text-primary")} data-relay-injected>
                <span aria-hidden className="relay-live-dot" />{t("newapi.relayInjected")}
              </span>
            )}
            {use?.codex === "direct" && (
              <span className={cn(pill, "bg-emerald-500/15 text-emerald-700 dark:text-emerald-400")} data-relay-direct>
                <PlugZap className="h-3 w-3" />{t("newapi.relayCodexDirect")}
              </span>
            )}
            {!provider.enabled && <span className={cn(pill, "bg-muted text-muted-foreground")}>{t("newapi.relayDisabled")}</span>}
            {others.length > 0 && (
              <span className="flex shrink-0 items-center -space-x-1" title={others.map(consumer => t(`newapi.topology.client.${consumer}`)).join(" · ")}>
                {others.map(consumer => (
                  <span key={consumer} className="flex h-5 w-5 items-center justify-center rounded-full bg-background ring-1 ring-border">
                    <ClientGlyph client={consumer} className="h-3 w-3" />
                  </span>
                ))}
              </span>
            )}
            <span className={cn(pill, testTone[provider.testStatus])}>{t(testLabel[provider.testStatus])}</span>
            {dsPeak !== null && (
              <span className={cn(pill, dsPeak ? "bg-amber-500/15 text-amber-700 dark:text-amber-400" : testTone.ok)} title={t("newapi.deepSeekPeakTip")}>
                {t(dsPeak ? "newapi.peakNow" : "newapi.offPeakNow")}
              </span>
            )}
          </div>

          {/* Balance and model */}
          <div className="relay-metrics grid grid-cols-2 overflow-hidden rounded-2xl bg-muted/45 ring-1 ring-inset ring-border/60">
            <div className="flex min-w-0 flex-col gap-1 p-3">
              <div className="flex items-center justify-between gap-1">
                <span className="truncate text-[11px] font-medium text-muted-foreground">{balanceLabel(provider)}</span>
                <button type="button" className="inline-flex h-6 w-6 shrink-0 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-background hover:text-foreground disabled:pointer-events-none disabled:opacity-40"
                  title={t("newapi.refreshBalance")} aria-label={t("newapi.refreshBalance")}
                  disabled={balanceBusy.has(provider.id) || !balanceReady} onClick={() => void refreshBalance(provider.id)}>
                  {balanceBusy.has(provider.id) ? <Loader2 className="h-3 w-3 animate-spin" /> : <RefreshCw className="h-3 w-3" />}
                </button>
              </div>
              <span className={cn("poster-number block truncate text-[1.4rem] font-bold leading-7 tracking-tight",
                !value && "text-muted-foreground/60")}>{value ?? "— —"}</span>
              <span className="truncate text-[10.5px] text-muted-foreground/85"
                title={provider.balance ? `${balanceCaption(provider)} · ${new Date(provider.balance.checkedAt * 1000).toLocaleString()}` : balanceCaption(provider)}>
                {store.balanceErrors[provider.id] ? t("newapi.balanceFailed") : balanceCaption(provider)}
              </span>
            </div>
            <div className="flex min-w-0 flex-col gap-1 border-l border-border/60 p-3">
              <span className="flex h-6 items-center text-[11px] font-medium text-muted-foreground">{t("newapi.model")}</span>
              <span className="block truncate font-mono text-[13px] font-medium leading-7" title={provider.model || t("newapi.noModel")}>
                {provider.model || t("newapi.noModel")}
              </span>
              <span className="flex min-w-0 items-center gap-1.5 text-[10.5px] text-muted-foreground">
                <span className="truncate">{t("newapi.modelCount", { count: includedModels(provider.models).length })}</span>
                <span className="shrink-0 rounded bg-background px-1 font-mono text-[9.5px] ring-1 ring-border/70">
                  {t(protocolBadgeKey[providerModelProtocol(provider)])}
                </span>
              </span>
            </div>
          </div>

          {/* Expanded mode: every member of this site, directly selectable */}
          {hasMultiple && isExpanded && (
            <div className="space-y-1.5 rounded-2xl bg-muted/35 p-2 ring-1 ring-inset ring-border/60">
              <div className="flex items-center justify-between gap-2 px-1 text-[11px] font-medium text-muted-foreground">
                <span className="flex items-center gap-1.5"><Layers className="h-3.5 w-3.5 text-primary" />{t("newapi.groupMembers")} ({group.providers.length})</span>
                <span className="flex items-center gap-1">
                  <button type="button" className="rounded px-1.5 py-0.5 text-xs text-primary hover:bg-primary/10" onClick={() => renameGroup(group)}>{t("newapi.renameGroup")}</button>
                  <button type="button" className="rounded px-1.5 py-0.5 text-xs text-muted-foreground hover:bg-muted" onClick={() => setExpanded(group, false)}>{t("newapi.collapseStack")}</button>
                </span>
              </div>
              {group.providers.map(item => (
                <div key={item.id} className={cn("flex flex-wrap items-center justify-between gap-2 rounded-xl border bg-card/90 p-2 text-xs transition-colors hover:bg-muted/50",
                  item.id === provider.id && "ring-1 ring-primary/40")}>
                  <div className="flex min-w-0 flex-1 items-center gap-2">
                    <StatusDot provider={item} />
                    <button type="button" title={item.name}
                      className="max-w-[120px] truncate rounded font-semibold text-foreground hover:text-primary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary/35"
                      onClick={() => {
                        if (item.id !== provider.id) selectMember(group, item.id, group.providers.indexOf(item) < memberIndex ? "prev" : "next");
                        setExpanded(group, false);
                      }}>{item.name}</button>
                    <span className="max-w-[110px] truncate font-mono text-[11px] text-muted-foreground" title={item.model}>{item.model || t("newapi.noModel")}</span>
                    {consumers.get(item.id)?.codex === "injected" && <span aria-hidden className="relay-live-dot" title={t("newapi.relayInjected")} />}
                  </div>
                  <div className="flex shrink-0 items-center gap-1.5">
                    <button type="button" className={cardButton} aria-label={`${t("newapi.queryBalance")} ${item.name}`} onClick={() => onBalance(item)}><Wallet className="h-3 w-3" /></button>
                    <Switch checked={item.enabled} disabled={mutationLocked} aria-label={`${t("newapi.relayEnable")} ${item.name}`} onCheckedChange={value => toggleEnabled(item, value)} />
                    <button type="button" className={cardButton} title={t("newapi.test")} aria-label={t("newapi.test")} disabled={!item.hasApiKey} onClick={() => onTest(item)}>
                      <RadioTower className="h-3 w-3" />
                    </button>
                    <button type="button" className={cardButton} title={t("newapi.edit")} aria-label={t("newapi.edit")} onClick={() => onEdit(item)}>
                      <SquarePen className="h-3 w-3" />
                    </button>
                    <button type="button" className={cn(cardButton, "hover:bg-red-500/10 hover:text-red-600 dark:hover:text-red-400")} title={t("newapi.remove")} aria-label={t("newapi.remove")} onClick={() => onRemove(item)}>
                      <Trash2 className="h-3 w-3" />
                    </button>
                  </div>
                </div>
              ))}
            </div>
          )}

          {/* Endpoint and actions */}
          <div className="relay-card-footer flex items-center justify-between gap-2">
            <span className="min-w-0 truncate font-mono text-[11px] text-muted-foreground/80" title={provider.baseUrl}>{hostOf(provider)}</span>
            <div className="flex shrink-0 items-center gap-1">
              <button type="button" className={cn(cardButton, "w-auto gap-1.5 whitespace-nowrap px-2")} onClick={() => onBalance(provider)}
                aria-label={`${t("newapi.queryBalance")} ${provider.name}`} title={t("newapi.queryBalance")}>
                <Wallet className="h-3.5 w-3.5" /><span className="relay-card-balance-label text-[11px]">{t("newapi.queryBalance")}</span>
              </button>
              <DropdownMenu>
                <DropdownMenuTrigger asChild>
                  <button type="button" className={cardButton} title={t("newapi.networkMode")} aria-label={t("newapi.networkMode")} disabled={mutationLocked}>
                    <Network className={cn("h-3.5 w-3.5", provider.networkMode === "direct" ? "text-emerald-600 dark:text-emerald-400" : "text-blue-600 dark:text-blue-400")} />
                  </button>
                </DropdownMenuTrigger>
                <DropdownMenuContent align="end" className="w-52">
                  <DropdownMenuRadioGroup value={provider.networkMode} onValueChange={mode => {
                    if (mode !== "proxy" && mode !== "direct") return;
                    void run(`network:${provider.id}`, async () => {
                      applyChange(await api.providers.setNetwork(provider.id, mode));
                      toast({
                        title: t("newapi.networkUpdated"),
                        description: t(mode === "direct" ? "newapi.networkDirectHint" : "newapi.networkProxyHint"),
                        variant: "success",
                      });
                    }, t("newapi.networkFailed"));
                  }}>
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
              <button type="button" className={cardButton} title={t("newapi.renameGroup")} aria-label={t("newapi.renameGroup")} onClick={() => renameGroup(group)}>
                <Tag className="h-3.5 w-3.5" />
              </button>
              <button type="button" className={cardButton} title={t("newapi.test")} aria-label={t("newapi.test")} disabled={!provider.hasApiKey} onClick={() => onTest(provider)}>
                <RadioTower className="h-3.5 w-3.5" />
              </button>
              <button type="button" className={cardButton} title={t("newapi.edit")} aria-label={t("newapi.edit")} disabled={mutationLocked} onClick={() => onEdit(provider)}>
                <SquarePen className="h-3.5 w-3.5" />
              </button>
              <button type="button" className={cn(cardButton, "hover:bg-red-500/10 hover:text-red-600 dark:hover:text-red-400")}
                title={t("newapi.remove")} aria-label={t("newapi.remove")} disabled={mutationLocked} onClick={() => onRemove(provider)}>
                <Trash2 className="h-3.5 w-3.5" />
              </button>
            </div>
          </div>
        </div>
      </article>
    </RelayDeck>
  );
}
