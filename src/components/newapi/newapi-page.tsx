import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  CircleHelp, FileDown, Layers, FileUp, List, Loader2, Network, Plus, PlugZap, RadioTower, RefreshCw, Settings2, Sparkles, SquarePen, Trash2, X,
} from "lucide-react";
import { api } from "@/lib/api";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import { AuroraCard, LIQUID_GOLD_SHINE } from "@/components/ui/aurora-card";
import { AnimatedGradientText } from "@/components/ui/animated-gradient-text";
import { RainbowButton } from "@/registry/magicui/rainbow-button";
import type { GlowColor } from "@/components/ui/glow-card";
import { toast } from "@/hooks/use-toast";
import {
  AlertDialog, AlertDialogAction, AlertDialogCancel, AlertDialogContent, AlertDialogDescription, AlertDialogFooter, AlertDialogHeader, AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import {
  DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuRadioGroup, DropdownMenuRadioItem, DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { ProviderDialog } from "@/components/newapi/provider-dialog";
import { ProviderTestDialog } from "@/components/newapi/provider-test-dialog";
import { ClientConfigs } from "@/components/newapi/client-configs";
import { includedModels, providerModelProtocol } from "@/lib/provider-models";
import type { CodexAccessMode, ModelProtocol, NewApiProvider, NewApiState, NewApiStateChange } from "@/types/newapi";

const STATE_KEY = ["newapi-state"] as const;

const protocolBadgeKey: Record<ModelProtocol, string> = {
  responses: "newapi.protocolShortResponses",
  chatCompletions: "newapi.protocolShortChat",
  anthropicMessages: "newapi.protocolShortAnthropic",
};

const cardColors: GlowColor[] = ["gold", "blue", "purple", "green", "red", "amber"];
const providerColor = (id: string): GlowColor => {
  let hash = 0;
  for (const ch of id) hash = (hash * 31 + ch.charCodeAt(0)) >>> 0;
  return cardColors[hash % cardColors.length];
};

const errorMessage = (e: unknown) => {
  const text = typeof e === "string" ? e : e instanceof Error ? e.message : String(e);
  return text.startsWith("newapi.") ? text : "newapi.errors.unexpected";
};

// DeepSeek peak/off-peak pricing windows in Beijing time: peak 9-12 & 14-18 on
// weekdays, off-peak (half price) everywhere else including whole weekends.
const deepSeekPeakNow = () => {
  const parts = new Intl.DateTimeFormat("en-US", { timeZone: "Asia/Shanghai", weekday: "short", hour: "numeric", hour12: false }).formatToParts(new Date());
  const weekday = parts.find(p => p.type === "weekday")?.value ?? "";
  const hour = Number(parts.find(p => p.type === "hour")?.value ?? "0") % 24;
  const weekend = weekday === "Sat" || weekday === "Sun";
  return !weekend && ((hour >= 9 && hour < 12) || (hour >= 14 && hour < 18));
};

function StatusDot({ provider }: { provider: NewApiProvider }) {
  return (
    <span
      className={cn(
        "h-2 w-2 shrink-0 rounded-full",
        provider.testStatus === "ok"
          ? "bg-emerald-500"
          : provider.testStatus === "failed"
            ? "bg-red-500"
            : "bg-zinc-400 dark:bg-zinc-600",
      )}
    />
  );
}

type RelayGroup = {
  key: string;
  url: string;
  providers: NewApiProvider[];
};

function relayGroupKey(baseUrl: string) {
  try {
    const url = new URL(baseUrl);
    url.hash = "";
    url.search = "";
    url.hostname = url.hostname.toLowerCase();
    if ((url.protocol === "https:" && url.port === "443") || (url.protocol === "http:" && url.port === "80")) url.port = "";
    url.pathname = url.pathname.replace(/\/+$/, "") || "/";
    return url.toString();
  } catch {
    return baseUrl.trim().replace(/\/+$/, "").toLowerCase();
  }
}

export function NewApiPage() {
  const { t, i18n } = useTranslation();
  const queryClient = useQueryClient();
  const state = useQuery({ queryKey: STATE_KEY, queryFn: api.providers.loadState, refetchOnWindowFocus: false });
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [filter, setFilter] = useState<string>("all");
  const [dialog, setDialog] = useState<{ open: boolean; provider: NewApiProvider | null }>({ open: false, provider: null });
  const [testDialog, setTestDialog] = useState<NewApiProvider | null>(null);
  const [removing, setRemoving] = useState<NewApiProvider | null>(null);
  const [appliedNote, setAppliedNote] = useState<NewApiStateChange["applied"]>(null);
  const [groupAliases, setGroupAliases] = useState<Record<string, string>>(() => {
    try {
      const saved = localStorage.getItem("newapi-relay-group-aliases");
      return saved ? JSON.parse(saved) as Record<string, string> : {};
    } catch {
      return {};
    }
  });
  const [aliasGroup, setAliasGroup] = useState<RelayGroup | null>(null);
  const [aliasDraft, setAliasDraft] = useState("");
  const [groupMemberSelect, setGroupMemberSelect] = useState<Record<string, string>>({});
  const [expandedGroups, setExpandedGroups] = useState<Record<string, boolean>>({});

  const providers = state.data?.providers ?? [];
  const router = state.data?.router;
  const routingEnabled = !!router?.enabled;
  const accessMode = router?.accessMode ?? "router";
  const nativeAccess = accessMode === "native";
  const configDrifted = !!router?.configDrifted;
  const activeId = state.data?.activeProviderId ?? null;
  const selectedProvider = activeId
    ? providers.find(provider => provider.id === activeId)
    : providers.find(provider => provider.enabled && provider.hasApiKey && provider.model);
  const nativeCompatible = !!selectedProvider && providerModelProtocol(selectedProvider) === "responses";
  const proxyRunning = !!state.data?.proxyRunning;
  const groups = useMemo<RelayGroup[]>(() => {
    const grouped = new Map<string, RelayGroup>();
    for (const provider of providers) {
      const key = relayGroupKey(provider.baseUrl);
      const existing = grouped.get(key);
      if (existing) existing.providers.push(provider);
      else grouped.set(key, { key, url: provider.baseUrl, providers: [provider] });
    }
    return [...grouped.values()].sort((a, b) => a.key.localeCompare(b.key));
  }, [providers]);
  useEffect(() => {
    if (state.isSuccess && filter !== "all" && !groups.some(group => group.key === filter)) {
      setFilter("all");
    }
  }, [filter, groups, state.isSuccess]);
  const shown = filter === "all" ? groups : groups.filter(group => group.key === filter);

  const replaceState = (next: NewApiState) => queryClient.setQueryData(STATE_KEY, next);
  const applyChange = (change: NewApiStateChange) => {
    replaceState(change.state);
    if (change.applied) setAppliedNote(change.applied);
  };
  const patchProvider = (provider: NewApiProvider) => {
    const current = queryClient.getQueryData<NewApiState>(STATE_KEY);
    if (current) replaceState({ ...current, providers: current.providers.map(item => (item.id === provider.id ? provider : item)) });
  };
  const renameGroup = (group: RelayGroup) => {
    const current = groupAliases[group.key] ?? hostOf(group.providers[0]);
    setAliasGroup(group);
    setAliasDraft(current);
  };
  const saveGroupAlias = () => {
    if (!aliasGroup) return;
    const next = aliasDraft.trim();
    if (!next) return;
    const updated = { ...groupAliases, [aliasGroup.key]: next };
    setGroupAliases(updated);
    localStorage.setItem("newapi-relay-group-aliases", JSON.stringify(updated));
    setAliasGroup(null);
  };
  const run = async (key: string, action: () => Promise<void>, failureTitle: string) => {
    if (busy) return;
    setBusy(key);
    setError(null);
    try {
      await action();
    } catch (e) {
      setError(errorMessage(e));
      toast({ title: failureTitle, description: t(errorMessage(e)), variant: "destructive" });
      // The backend may have persisted partial state (e.g. a failed test marker) before erroring.
      void state.refetch();
    } finally {
      setBusy(null);
    }
  };
  const lock = (key: string) => !!busy && busy !== key;
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
    switch (provider.balanceSource) {
      case "newapi": return t("newapi.sourceNewApi");
      case "sub2api": return t("newapi.sourceSub2Api");
      case "deepseek": return t("newapi.sourceDeepSeek");
      default: return t("newapi.balanceUndetected");
    }
  };
  const hostOf = (provider: NewApiProvider) => {
    try { return new URL(provider.baseUrl).host; } catch { return provider.baseUrl; }
  };
  const pickImportFile = async () => {
    const { open } = await import("@tauri-apps/plugin-dialog");
    return open({ multiple: false, directory: false, filters: [{ name: "JSON", extensions: ["json"] }] });
  };
  const importProviders = () =>
    run("import", async () => {
      const path = await pickImportFile();
      if (typeof path !== "string") return;
      const result = await api.providers.import(path);
      applyChange(result);
      toast({ title: t("newapi.importedResult", { imported: result.imported, skipped: result.skipped }), variant: "success" });
    }, t("newapi.importFailed"));
  const exportProviders = () =>
    run("export", async () => {
      const { save } = await import("@tauri-apps/plugin-dialog");
      const path = await save({ defaultPath: "agentharbor-relays.json", filters: [{ name: "JSON", extensions: ["json"] }] });
      if (typeof path !== "string") return;
      await api.providers.export(path);
      toast({ title: t("newapi.exportedDone", { path }), variant: "success" });
    }, t("newapi.exportFailed"));

  const iconButton =
    "inline-flex h-7 w-7 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-muted hover:text-foreground disabled:opacity-40 disabled:pointer-events-none";
  const cardButton =
    "flex h-7 w-7 items-center justify-center rounded-md bg-black/[0.05] text-foreground/70 transition-colors hover:bg-black/[0.09] hover:text-foreground disabled:opacity-40 disabled:pointer-events-none dark:bg-white/[0.07] dark:hover:bg-white/[0.12]";
  const chipClass = (selected: boolean) =>
    cn(
      "flex shrink-0 items-center gap-1.5 rounded-lg border bg-card px-3 py-1.5 text-[13px] font-medium shadow-sm transition-all",
      selected
        ? "border-primary/50 bg-primary/[0.06] text-primary ring-1 ring-primary/15"
        : "text-foreground/80 hover:-translate-y-px hover:bg-muted/60",
    );
  const softBadge = (tone: "green" | "amber" | "red" | "gray") => {
    const tones = {
      green: "border-transparent bg-emerald-500/15 text-emerald-700 hover:bg-emerald-500/15 dark:text-emerald-400",
      amber: "border-transparent bg-amber-500/15 text-amber-700 hover:bg-amber-500/15 dark:text-amber-400",
      red: "border-transparent bg-red-500/10 text-red-600 hover:bg-red-500/10 dark:text-red-400",
      gray: "border-input bg-muted/60 text-muted-foreground hover:bg-muted/60",
    };
    return cn("rounded-full px-2 py-0.5 text-[11px] font-medium shadow-none", tones[tone]);
  };

  return (
    <div className="space-y-6">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <p className="text-base text-foreground/90">{t("newapi.description")}</p>
        {nativeAccess ? <Badge variant="outline">{t("newapi.nativeNoRouter")}</Badge> : <div className="flex items-center gap-2.5 rounded-full border bg-card py-1.5 pl-4 pr-2 text-sm shadow-sm">
          <span className="relative flex h-2.5 w-2.5 items-center justify-center">
            {proxyRunning && (
              <span aria-hidden className="absolute inline-flex h-full w-full animate-ping rounded-full bg-emerald-400 opacity-60 motion-reduce:hidden" />
            )}
            <span className={cn("relative inline-flex h-2.5 w-2.5 rounded-full", proxyRunning ? "bg-emerald-500" : "bg-zinc-400 dark:bg-zinc-600")} />
          </span>
          <span className="font-medium">{t(proxyRunning ? "newapi.proxyRunning" : "newapi.proxyStopped")}</span>
          <span className="rounded-md bg-muted px-2 py-0.5 font-mono text-xs text-muted-foreground">
            http://127.0.0.1:{router?.port ?? 43127}
          </span>
        </div>}
      </div>

      {(error || state.error) && (
        <div
          role="alert"
          className="flex items-start justify-between gap-3 rounded-xl border border-destructive/30 bg-destructive/5 p-4 text-sm text-destructive"
        >
          <span>{error ? t(error) : String(state.error)}</span>
          <button
            type="button"
            onClick={() => { setError(null); void state.refetch(); }}
            aria-label={t("common.cancel")}
            className="shrink-0 rounded p-0.5 transition-colors hover:bg-destructive/10"
          >
            <X className="h-4 w-4" />
          </button>
        </div>
      )}

      <AuroraCard
        color="blue"
        shine={routingEnabled}
        shineColor={["#60a5fa", "#a78bfa", "#60a5fa"]}
        shineDuration={6}
        className={cn(
          "shadow-[0_2px_24px_rgba(59,130,246,0.07)] dark:shadow-none",
          routingEnabled && "drop-shadow-[0_4px_22px_rgba(139,92,246,0.18)]",
        )}
      >
        <div aria-labelledby="newapi-routing-title" className="p-5">
          <div className="flex items-start gap-3.5">
            <div className="flex h-11 w-11 shrink-0 items-center justify-center rounded-2xl bg-gradient-to-br from-blue-500/20 to-purple-500/10 ring-1 ring-inset ring-blue-500/15">
              <Sparkles className="h-5 w-5 text-blue-500 dark:text-blue-400" />
            </div>
            <div className="min-w-0 flex-1">
              <div className="flex flex-wrap items-center gap-2.5">
                <h2 id="newapi-routing-title" className="text-base font-semibold tracking-tight">
                  {routingEnabled ? (
                    <AnimatedGradientText speed={1.4} colorFrom="#60a5fa" colorTo="#a78bfa">
                      {t("newapi.codexAccess")}
                    </AnimatedGradientText>
                  ) : (
                    t("newapi.codexAccess")
                  )}
                </h2>
                {routingEnabled && !configDrifted && <Badge className={softBadge("green")}>{t("newapi.enabledBadge")}</Badge>}
                {configDrifted && <Badge className={softBadge("amber")}>{t("newapi.driftedBadge")}</Badge>}
              </div>
              <p className="mt-1 text-sm leading-relaxed text-muted-foreground">{t(nativeAccess ? "newapi.nativeAccessDesc" : "newapi.smartRoutingDesc")}</p>
            </div>
            <Switch
              className="mt-1"
              aria-label={t("newapi.codexAccess")}
              checked={routingEnabled}
              disabled={lock("routing") || (!routingEnabled && (providers.length === 0 || (nativeAccess && !nativeCompatible)))}
              onCheckedChange={value =>
                void run("routing", async () => {
                  const change = await api.routing.setEnabled(value);
                  applyChange(change);
                  toast({ title: t(value ? "newapi.routingApplied" : "newapi.routingRemoved"), description: change.applied?.backupPath ?? undefined, variant: "success" });
                }, t("newapi.routingFailed"))
              }
            />
          </div>
          <div className="mt-4 space-y-2">
            <Label htmlFor="codex-access-mode">{t("newapi.accessMode")}</Label>
            <Select value={accessMode} disabled={lock("access-mode")} onValueChange={value => void run("access-mode", async () => {
              applyChange(await api.routing.setAccessMode(value as CodexAccessMode));
              toast({ title: t("newapi.accessModeApplied"), description: t("newapi.accessRestartHint"), variant: "success" });
            }, t("newapi.routingFailed"))}>
              <SelectTrigger id="codex-access-mode" className="max-w-sm"><SelectValue /></SelectTrigger>
              <SelectContent>
                <SelectItem value="router">{t("newapi.accessRouter")}</SelectItem>
                <SelectItem value="native" disabled={!nativeCompatible}>{t("newapi.accessNative")}</SelectItem>
              </SelectContent>
            </Select>
            {selectedProvider && <p className="text-xs text-muted-foreground">{t("newapi.accessSelectedModel", { provider: selectedProvider.name, model: selectedProvider.model, protocol: t(protocolBadgeKey[providerModelProtocol(selectedProvider)]) })}</p>}
            {!nativeCompatible && <p className="text-xs text-amber-700 dark:text-amber-300">{t(selectedProvider ? "newapi.nativeProtocolHint" : "newapi.nativeProviderHint")}</p>}
            {nativeAccess && <p className="text-xs leading-relaxed text-muted-foreground">{t("newapi.nativeAuthHint")}</p>}
            <p className="text-xs leading-relaxed text-muted-foreground">{t(nativeAccess ? "newapi.nativeNetworkHint" : "newapi.routerNetworkHint")}</p>
            <p className="text-xs text-muted-foreground">{t("newapi.accessRestartHint")}</p>
          </div>
          <p className="mt-3 text-sm leading-relaxed text-muted-foreground">
            {t(routingEnabled ? (nativeAccess ? "newapi.nativeOnHint" : "newapi.routingOnHint") : "newapi.routingOffHint")}
          </p>
          {configDrifted && (
            <p role="alert" className="mt-2 rounded-lg border border-amber-500/30 bg-amber-500/[0.07] px-3 py-2 text-[13px] leading-relaxed text-amber-700 dark:text-amber-300">
              {t("newapi.configDriftedHint")}
            </p>
          )}
          {appliedNote && (
            <p className="mt-2 break-all font-mono text-[11px] text-muted-foreground/80">
              {appliedNote.path}
              {appliedNote.backupPath ? ` · ${t("newapi.backup")}: ${appliedNote.backupPath}` : ""}
            </p>
          )}
          {!nativeAccess && <div className="mt-4 flex items-center justify-between gap-4 rounded-xl bg-black/[0.03] p-3.5 ring-1 ring-border/60 dark:bg-white/[0.03]">
            <div className="min-w-0">
              <div className="flex items-center gap-1.5">
                <Settings2 className="h-3.5 w-3.5 text-muted-foreground" />
                <span className="text-sm font-semibold">{t("newapi.accountless")}</span>
                <span title={t("newapi.accountlessTip")}>
                  <CircleHelp className="h-4 w-4 text-muted-foreground" />
                </span>
              </div>
              <p className="mt-0.5 text-[13px] text-muted-foreground">{t("newapi.accountlessHint")}</p>
            </div>
            <Switch
              aria-label={t("newapi.accountless")}
              checked={!!router?.accountless}
              disabled={!routingEnabled || lock("accountless")}
              onCheckedChange={value =>
                void run("accountless", async () => {
                  const change = await api.routing.setAccountless(value);
                  applyChange(change);
                  toast({ title: t("newapi.accountlessApplied"), variant: "success" });
                }, t("newapi.routingFailed"))
              }
            />
          </div>}
        </div>
      </AuroraCard>

      <ClientConfigs providers={providers} />

      <section className="space-y-4" aria-labelledby="newapi-providers-title">
        <div className="flex flex-wrap items-center gap-3">
          <h2 id="newapi-providers-title" className="text-lg font-semibold tracking-tight">
            {t("newapi.providersSection")}
          </h2>
          <span className="rounded-full bg-muted px-2 py-0.5 text-xs font-medium tabular-nums text-muted-foreground">
            {groups.length}
          </span>
          <div className="ml-auto flex items-center gap-2">
            <RainbowButton className="h-9 rounded-lg px-3.5 dark:text-black" onClick={() => setDialog({ open: true, provider: null })} disabled={lock("open-dialog")}>
              <Plus className="h-4 w-4" />
              {t("newapi.addProvider")}
            </RainbowButton>
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button variant="outline" size="icon" className="h-9 w-9 rounded-lg" disabled={lock("import")} aria-label={t("newapi.importExport")}>
                  <FileUp className="h-4 w-4" />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                <DropdownMenuItem onClick={() => void importProviders()}>
                  <FileUp className="h-4 w-4" />
                  {t("newapi.importProviders")}
                </DropdownMenuItem>
                <DropdownMenuItem onClick={() => void exportProviders()} disabled={providers.length === 0}>
                  <FileDown className="h-4 w-4" />
                  {t("newapi.exportProviders")}
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          </div>
        </div>

        <div className="flex min-w-0 items-center gap-2 overflow-x-auto pb-0.5">
          <button type="button" onClick={() => setFilter("all")} className={chipClass(filter === "all")}>
            {t("newapi.allProviders")}
          </button>
          {groups.map(group => (
            <button key={group.key} type="button" onClick={() => setFilter(group.key)} className={chipClass(filter === group.key)}>
              <span className="h-2 w-2 shrink-0 rounded-full bg-primary/70" />
              {groupAliases[group.key] ?? hostOf(group.providers[0])}
              <span className="text-[10px] text-muted-foreground">{group.providers.length}</span>
            </button>
          ))}
        </div>

        {state.isLoading ? (
          <div className="grid gap-4 sm:grid-cols-1 md:grid-cols-2 xl:grid-cols-3">
            {[0, 1, 2].map(i => (
              <AuroraCard key={i} color="blue" className="pointer-events-none opacity-70">
                <div className="space-y-3 p-4">
                  <div className="flex items-center justify-between">
                    <Skeleton className="h-5 w-28" />
                    <Skeleton className="h-5 w-9 rounded-full" />
                  </div>
                  <div className="grid grid-cols-2 gap-2">
                    <Skeleton className="h-16 w-full rounded-xl" />
                    <Skeleton className="h-16 w-full rounded-xl" />
                  </div>
                  <div className="flex items-center justify-between pt-1">
                    <Skeleton className="h-4 w-24" />
                    <Skeleton className="h-6 w-28 rounded-md" />
                  </div>
                </div>
              </AuroraCard>
            ))}
          </div>
        ) : shown.length === 0 ? (
          <div className="rounded-2xl border border-dashed border-border/80 bg-card/50 px-6 py-14 text-center">
            <div className="mx-auto flex h-12 w-12 items-center justify-center rounded-2xl bg-gradient-to-br from-primary/15 to-primary/5 ring-1 ring-inset ring-primary/10">
              <RadioTower className="h-5 w-5 text-primary" />
            </div>
            <p className="mt-4 text-sm text-muted-foreground">{t("newapi.empty")}</p>
            <Button variant="outline" size="sm" className="mt-5" onClick={() => setDialog({ open: true, provider: null })}>
              <Plus className="h-4 w-4" />
              {t("newapi.addProvider")}
            </Button>
          </div>
        ) : (
          <div className="grid gap-4 sm:grid-cols-1 md:grid-cols-2 xl:grid-cols-3">
            {shown.map(group => {
              const activeMemberId = groupMemberSelect[group.key] ?? (group.providers.find(item => item.id === activeId)?.id ?? group.providers[0].id);
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

              return (
                <div key={group.key} className="group relative">
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
                    color={cardColor}
                    shine={isGroupActive || selected}
                    shineColor={isGroupActive ? LIQUID_GOLD_SHINE : ["#60a5fa", "#a78bfa", "#60a5fa"]}
                    className={cn(
                      "transition-transform duration-200 ease-out motion-reduce:transition-none hover:-translate-y-0.5",
                      selected && "rounded-2xl ring-2 ring-primary/40",
                      isGroupActive && "drop-shadow-[0_4px_22px_rgba(212,175,55,0.3)]",
                    )}
                  >
                    <div className="flex flex-1 flex-col p-3.5">
                      {/* Header: Title + Domain + Badges + Switch */}
                      <div className="flex items-center justify-between gap-2">
                        <div className="flex min-w-0 flex-1 items-center gap-1.5">
                          <StatusDot provider={provider} />
                          <h3 className="truncate text-[15px] font-semibold tracking-tight" title={groupAliases[group.key] ?? hostOf(provider)}>
                            {groupAliases[group.key] ?? hostOf(provider)}
                          </h3>
                          {hasMultiple && (
                            <span
                              className="inline-flex shrink-0 items-center gap-1 rounded-full bg-primary/10 px-1.5 py-0.5 text-[10px] font-medium text-primary cursor-pointer hover:bg-primary/20"
                              onClick={(e) => {
                                e.stopPropagation();
                                setExpandedGroups(prev => ({ ...prev, [group.key]: !prev[group.key] }));
                              }}
                              title={isExpanded ? t("newapi.collapseStack") : t("newapi.expandAll")}
                            >
                              <Layers className="h-2.5 w-2.5" />
                              {group.providers.length}
                            </span>
                          )}
                          <span className="shrink-0 max-w-[100px] truncate rounded bg-black/[0.04] px-1.5 py-0.5 font-mono text-[10px] text-muted-foreground dark:bg-white/[0.06]" title={hostOf(provider)}>
                            {hostOf(provider)}
                          </span>
                        </div>
                        <div className="flex shrink-0 items-center gap-1.5">
                          {provider.enabled && routingEnabled && (
                            <Badge className={softBadge("amber")}>{t("newapi.badgeLocked")}</Badge>
                          )}
                          {provider.enabled && !routingEnabled && (
                            <Badge className={softBadge("green")}>{t("newapi.badgeEnabled")}</Badge>
                          )}
                          <Switch
                            checked={provider.enabled}
                            className={isCurrentActive ? "data-[state=checked]:bg-amber-500" : undefined}
                            disabled={lock(`enable:${provider.id}`)}
                            onCheckedChange={value =>
                              void run(`enable:${provider.id}`, async () => {
                                const change = await api.providers.setEnabled(provider.id, value);
                                applyChange(change);
                              }, t("newapi.enableFailed"))
                            }
                          />
                        </div>
                      </div>

                      {/* Stack Deck Selector Tabs for Same-Relay Groups */}
                      {hasMultiple && !isExpanded && (
                        <div className="mt-2.5 flex items-center justify-between gap-1 rounded-xl bg-black/[0.03] p-1 ring-1 ring-border/50 dark:bg-white/[0.03]">
                          <div className="flex min-w-0 flex-1 items-center gap-1 overflow-x-auto scrollbar-hide py-0.5">
                            {group.providers.map(item => {
                              const isCur = item.id === provider.id;
                              const isItemActive = item.id === activeId && routingEnabled;
                              return (
                                <button
                                  key={item.id}
                                  type="button"
                                  onClick={(e) => {
                                    e.stopPropagation();
                                    setGroupMemberSelect(prev => ({ ...prev, [group.key]: item.id }));
                                  }}
                                  className={cn(
                                    "flex shrink-0 items-center gap-1.5 rounded-lg px-2 py-1 text-xs font-medium transition-all",
                                    isCur
                                      ? "bg-card text-foreground shadow-xs ring-1 ring-border"
                                      : "text-muted-foreground hover:bg-black/[0.04] hover:text-foreground dark:hover:bg-white/[0.06]",
                                  )}
                                  title={item.name}
                                >
                                  <StatusDot provider={item} />
                                  <span className="max-w-[80px] truncate">{item.name}</span>
                                  {isItemActive && <PlugZap className="h-3 w-3 text-amber-500 shrink-0" />}
                                </button>
                              );
                            })}
                          </div>
                          <button
                            type="button"
                            className={cn(cardButton, "h-6 w-6 shrink-0")}
                            title={t("newapi.expandAll")}
                            onClick={(e) => {
                              e.stopPropagation();
                              setExpandedGroups(prev => ({ ...prev, [group.key]: true }));
                            }}
                          >
                            <Layers className="h-3 w-3" />
                          </button>
                        </div>
                      )}

                      {/* Dual Info Cards: Balance (Left) & Model/Status (Right) */}
                      <div className="mt-2.5 grid grid-cols-2 gap-2">
                        {/* Left: Balance Box */}
                        <div className={cn(
                          "flex flex-col justify-between rounded-xl p-2.5 ring-1 transition-colors min-w-0",
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
                              disabled={lock(`balance:${provider.id}`) || !(provider.hasApiKey || provider.hasAccessToken)}
                              onClick={() =>
                                void run(`balance:${provider.id}`, async () => patchProvider(await api.providers.queryBalance(provider.id)), t("newapi.balanceFailed"))
                              }
                            >
                              {busy === `balance:${provider.id}` ? <Loader2 className="h-3 w-3 animate-spin" /> : <RefreshCw className="h-3 w-3" />}
                            </button>
                          </div>
                          <div className="mt-0.5">
                            <span className={cn(
                              "truncate text-base font-semibold tabular-nums tracking-tight block",
                              isCurrentActive && valuePositive && "text-emerald-600 dark:text-emerald-400",
                              cardColor === "gold" && "dark:text-[#fef08a]"
                            )}>
                              {value ?? "— —"}
                            </span>
                          </div>
                          <p className="mt-1 truncate text-[10px] text-muted-foreground/80" title={balanceCaption(provider)}>
                            {balanceCaption(provider)}
                          </p>
                        </div>

                        {/* Right: Model & Connection Box */}
                        <div className={cn(
                          "flex flex-col justify-between rounded-xl p-2.5 ring-1 transition-colors min-w-0",
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
                          <div className="mt-0.5 flex min-w-0 items-center gap-1.5">
                            <List className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                            <span className="truncate font-mono text-xs font-medium" title={provider.model || t("newapi.noModel")}>
                              {provider.model || t("newapi.noModel")}
                            </span>
                          </div>
                          <div className="mt-1 flex items-center justify-between gap-1 text-[10px] text-muted-foreground">
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
                                onClick={() => setExpandedGroups(prev => ({ ...prev, [group.key]: false }))}
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
                                  <span className="font-semibold truncate max-w-[120px]" title={item.name}>{item.name}</span>
                                  <span className="font-mono text-[11px] text-muted-foreground truncate max-w-[110px]" title={item.model}>{item.model || t("newapi.noModel")}</span>
                                  {item.id === activeId && routingEnabled && (
                                    <span className="inline-flex items-center gap-0.5 rounded bg-amber-500/10 px-1 py-0.2 text-[10px] font-medium text-amber-600 dark:text-amber-400">
                                      <PlugZap className="h-2.5 w-2.5" />
                                      {t("newapi.isCurrentRoute")}
                                    </span>
                                  )}
                                </div>
                                <div className="flex shrink-0 items-center gap-1.5">
                                  <Switch
                                    checked={item.enabled}
                                    disabled={lock(`enable:${item.id}`)}
                                    onCheckedChange={value =>
                                      void run(`enable:${item.id}`, async () => applyChange(await api.providers.setEnabled(item.id, value)), t("newapi.enableFailed"))
                                    }
                                  />
                                  <button
                                    type="button"
                                    className={cardButton}
                                    title={t("newapi.test")}
                                    aria-label={t("newapi.test")}
                                    disabled={lock(`test:${item.id}`) || !item.hasApiKey}
                                    onClick={() => setTestDialog(item)}
                                  >
                                    <RadioTower className="h-3 w-3" />
                                  </button>
                                  <button
                                    type="button"
                                    className={cardButton}
                                    title={t("newapi.edit")}
                                    aria-label={t("newapi.edit")}
                                    onClick={() => setDialog({ open: true, provider: item })}
                                  >
                                    <SquarePen className="h-3 w-3" />
                                  </button>
                                  <button
                                    type="button"
                                    className={cn(cardButton, "hover:bg-red-500/10 hover:text-red-600 dark:hover:text-red-400")}
                                    title={t("newapi.remove")}
                                    aria-label={t("newapi.remove")}
                                    onClick={() => setRemoving(item)}
                                  >
                                    <Trash2 className="h-3 w-3" />
                                  </button>
                                </div>
                              </div>
                            ))}
                          </div>
                        </div>
                      )}

                      {/* Footer: Status / Hint & Action Buttons */}
                      <div className="mt-2.5 flex items-center justify-between gap-2 border-t border-border/40 pt-2">
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
                          <DropdownMenu>
                            <DropdownMenuTrigger asChild>
                              <button
                                type="button"
                                className={cardButton}
                                title={t("newapi.networkMode")}
                                aria-label={t("newapi.networkMode")}
                                disabled={lock(`network:${provider.id}`)}
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
                            disabled={lock(`test:${provider.id}`) || !provider.hasApiKey}
                            onClick={() => setTestDialog(provider)}
                          >
                            <RadioTower className="h-3.5 w-3.5" />
                          </button>
                          <button
                            type="button"
                            className={cardButton}
                            title={t("newapi.edit")}
                            aria-label={t("newapi.edit")}
                            disabled={lock(`open-dialog`)}
                            onClick={() => setDialog({ open: true, provider })}
                          >
                            <SquarePen className="h-3.5 w-3.5" />
                          </button>
                          <button
                            type="button"
                            className={cn(cardButton, "hover:bg-red-500/10 hover:text-red-600 dark:hover:text-red-400")}
                            title={t("newapi.remove")}
                            aria-label={t("newapi.remove")}
                            disabled={lock(`delete:${provider.id}`)}
                            onClick={() => setRemoving(provider)}
                          >
                            <Trash2 className="h-3.5 w-3.5" />
                          </button>
                        </div>
                      </div>
                    </div>
                  </AuroraCard>
                </div>
              );
            })}          </div>
        )}
      </section>

      <ProviderDialog
        open={dialog.open}
        provider={dialog.provider}
        onOpenChange={open => setDialog(current => ({ ...current, open }))}
        onSaved={applyChange}
      />

      <Dialog open={aliasGroup !== null} onOpenChange={open => !open && setAliasGroup(null)}>
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>{t("newapi.groupAliasTitle")}</DialogTitle>
            <DialogDescription>{t("newapi.groupAliasDescription")}</DialogDescription>
          </DialogHeader>
          <Input autoFocus value={aliasDraft} placeholder={t("newapi.groupAliasPlaceholder")} onChange={event => setAliasDraft(event.target.value)} onKeyDown={event => { if (event.key === "Enter") saveGroupAlias(); }} />
          <DialogFooter>
            <Button type="button" variant="ghost" onClick={() => setAliasGroup(null)}>{t("common.cancel")}</Button>
            <Button type="button" onClick={saveGroupAlias} disabled={!aliasDraft.trim()}>{t("newapi.groupAliasSave")}</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <ProviderTestDialog
        open={testDialog !== null}
        provider={testDialog}
        onOpenChange={open => !open && setTestDialog(null)}
        onTested={patchProvider}
        onFailed={() => void state.refetch()}
      />

      <AlertDialog open={removing !== null} onOpenChange={open => !open && setRemoving(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("newapi.confirmDeleteTitle")}</AlertDialogTitle>
            <AlertDialogDescription>{t("newapi.confirmDeleteBody", { name: removing?.name ?? "" })}</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction
              onClick={() => {
                const provider = removing;
                setRemoving(null);
                if (provider) {
                  void run(`delete:${provider.id}`, async () => {
                    const change = await api.providers.delete(provider.id);
                    applyChange(change);
                  }, t("newapi.deleteFailed"));
                }
              }}
            >
              {t("common.confirm")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}
