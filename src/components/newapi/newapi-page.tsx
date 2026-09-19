import { useState } from "react";
import { useTranslation } from "react-i18next";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  CircleHelp, Crosshair, FileDown, FileUp, List, Loader2, Plus, RadioTower, RefreshCw, Sparkles, SquarePen, Trash2, X,
} from "lucide-react";
import { api } from "@/lib/api";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Switch } from "@/components/ui/switch";
import { toast } from "@/hooks/use-toast";
import {
  AlertDialog, AlertDialogAction, AlertDialogCancel, AlertDialogContent, AlertDialogDescription, AlertDialogFooter, AlertDialogHeader, AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import {
  DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { ProviderDialog } from "@/components/newapi/provider-dialog";
import type { NewApiProvider, NewApiState, NewApiStateChange } from "@/types/newapi";

const STATE_KEY = ["newapi-state"] as const;

const errorMessage = (e: unknown) => {
  const text = typeof e === "string" ? e : e instanceof Error ? e.message : String(e);
  return text.startsWith("newapi.") ? text : "newapi.errors.unexpected";
};

function StatusDot({ provider, active }: { provider: NewApiProvider; active: boolean }) {
  return (
    <span
      className={cn(
        "h-2 w-2 shrink-0 rounded-full",
        active ? "bg-amber-500" : provider.enabled ? "bg-emerald-500" : "bg-zinc-400 dark:bg-zinc-600",
      )}
    />
  );
}

export function NewApiPage() {
  const { t, i18n } = useTranslation();
  const queryClient = useQueryClient();
  const state = useQuery({ queryKey: STATE_KEY, queryFn: api.loadNewApiState, refetchOnWindowFocus: false });
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [filter, setFilter] = useState<string>("all");
  const [dialog, setDialog] = useState<{ open: boolean; provider: NewApiProvider | null }>({ open: false, provider: null });
  const [removing, setRemoving] = useState<NewApiProvider | null>(null);
  const [appliedNote, setAppliedNote] = useState<NewApiStateChange["applied"]>(null);

  const providers = state.data?.providers ?? [];
  const router = state.data?.router;
  const routingEnabled = !!router?.enabled;
  const activeId = state.data?.activeProviderId ?? null;
  const shown = filter === "all" ? providers : providers.filter(provider => provider.id === filter);

  const replaceState = (next: NewApiState) => queryClient.setQueryData(STATE_KEY, next);
  const applyChange = (change: NewApiStateChange) => {
    replaceState(change.state);
    if (change.applied) setAppliedNote(change.applied);
  };
  const patchProvider = (provider: NewApiProvider) => {
    const current = queryClient.getQueryData<NewApiState>(STATE_KEY);
    if (current) replaceState({ ...current, providers: current.providers.map(item => (item.id === provider.id ? provider : item)) });
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
      case "billing": return t("newapi.sourceBilling");
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
      const result = await api.importNewApiProviders(path);
      applyChange(result);
      toast({ title: t("newapi.importedResult", { imported: result.imported, skipped: result.skipped }), variant: "success" });
    }, t("newapi.importFailed"));
  const exportProviders = () =>
    run("export", async () => {
      const { save } = await import("@tauri-apps/plugin-dialog");
      const path = await save({ defaultPath: "aimami-relays.json", filters: [{ name: "JSON", extensions: ["json"] }] });
      if (typeof path !== "string") return;
      await api.exportNewApiProviders(path);
      toast({ title: t("newapi.exportedDone", { path }), variant: "success" });
    }, t("newapi.exportFailed"));

  const iconButton =
    "inline-flex h-7 w-7 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-muted hover:text-foreground disabled:opacity-40 disabled:pointer-events-none";
  const cardButton =
    "flex h-7 w-7 items-center justify-center rounded-md bg-muted/60 text-foreground/70 transition-colors hover:bg-muted hover:text-foreground disabled:opacity-40 disabled:pointer-events-none";
  const chipClass = (selected: boolean) =>
    cn(
      "flex shrink-0 items-center gap-1.5 rounded-lg border bg-card px-3 py-1.5 text-[13px] font-medium shadow-sm transition-colors",
      selected ? "border-primary/40 text-primary" : "text-foreground/80 hover:bg-muted/60",
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
        <div className="flex items-center gap-2.5 rounded-full border bg-card py-1.5 pl-4 pr-2 text-sm shadow-sm">
          <span className={cn("h-2.5 w-2.5 rounded-full", state.data?.proxyRunning ? "bg-emerald-500" : "bg-zinc-400 dark:bg-zinc-600")} />
          <span className="font-medium">{t(state.data?.proxyRunning ? "newapi.proxyRunning" : "newapi.proxyStopped")}</span>
          <span className="rounded-md bg-muted px-2 py-0.5 font-mono text-xs text-muted-foreground">
            http://127.0.0.1:{router?.port ?? 25817}
          </span>
        </div>
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

      <section className="rounded-2xl border bg-card p-5" aria-labelledby="newapi-routing-title">
        <div className="flex items-start gap-4">
          <div className="flex h-11 w-11 shrink-0 items-center justify-center rounded-xl bg-gradient-to-br from-primary/15 to-primary/5 ring-1 ring-primary/10">
            <Sparkles className="h-5 w-5 text-primary" />
          </div>
          <div className="min-w-0 flex-1">
            <div className="flex flex-wrap items-center gap-2.5">
              <h2 id="newapi-routing-title" className="text-lg font-semibold">{t("newapi.smartRouting")}</h2>
              {routingEnabled && <Badge className={softBadge("green")}>{t("newapi.enabledBadge")}</Badge>}
            </div>
            <p className="mt-1.5 text-sm leading-relaxed text-muted-foreground">{t("newapi.smartRoutingDesc")}</p>
          </div>
          <Switch
            className="mt-1.5"
            checked={routingEnabled}
            disabled={lock("routing") || providers.length === 0}
            onCheckedChange={value =>
              void run("routing", async () => {
                const change = await api.setNewApiSmartRouting(value);
                applyChange(change);
                toast({ title: t(value ? "newapi.routingApplied" : "newapi.routingRemoved"), description: change.applied?.backupPath ?? undefined, variant: "success" });
              }, t("newapi.routingFailed"))
            }
          />
        </div>
        <p className="mt-4 text-sm leading-relaxed text-muted-foreground">
          {t(routingEnabled ? "newapi.routingOnHint" : "newapi.routingOffHint")}
        </p>
        {appliedNote && (
          <p className="mt-2 break-all font-mono text-[11px] text-muted-foreground/80">
            {appliedNote.path}
            {appliedNote.backupPath ? ` · ${t("newapi.backup")}: ${appliedNote.backupPath}` : ""}
          </p>
        )}
        <div className="mt-5 flex items-center justify-between gap-3">
          <div className="min-w-0">
            <div className="flex items-center gap-1.5">
              <span className="text-[15px] font-semibold">{t("newapi.accountless")}</span>
              <span title={t("newapi.accountlessTip")}>
                <CircleHelp className="h-4 w-4 text-muted-foreground" />
              </span>
            </div>
            <p className="mt-1 text-sm text-muted-foreground">{t("newapi.accountlessHint")}</p>
          </div>
          <Switch
            checked={!!router?.accountless}
            disabled={!routingEnabled || lock("accountless")}
            onCheckedChange={value =>
              void run("accountless", async () => {
                const change = await api.setNewApiAccountless(value);
                applyChange(change);
                toast({ title: t("newapi.accountlessApplied"), variant: "success" });
              }, t("newapi.routingFailed"))
            }
          />
        </div>
      </section>

      <section className="rounded-2xl border bg-card p-4" aria-labelledby="newapi-providers-title">
        <h2 id="newapi-providers-title" className="sr-only">{t("newapi.providersSection")}</h2>
        <div className="flex flex-wrap items-center gap-3 px-1 py-1">
          <Button className="h-9 rounded-lg px-3.5" onClick={() => setDialog({ open: true, provider: null })} disabled={lock("open-dialog")}>
            <Plus className="h-4 w-4" />
            {t("newapi.addProvider")}
          </Button>
          <div className="flex min-w-0 flex-1 items-center gap-2 overflow-x-auto pb-0.5">
            <button type="button" onClick={() => setFilter("all")} className={chipClass(filter === "all")}>
              {t("newapi.allProviders")}
            </button>
            {providers.map(provider => (
              <button key={provider.id} type="button" onClick={() => setFilter(provider.id)} className={chipClass(filter === provider.id)}>
                <StatusDot provider={provider} active={provider.id === activeId && routingEnabled} />
                {provider.name}
              </button>
            ))}
          </div>
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

        <div className="mt-3 rounded-xl bg-muted/40 p-3">
          {state.isLoading ? (
            <div className="flex justify-center py-12">
              <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" />
            </div>
          ) : shown.length === 0 ? (
            <div className="rounded-lg border border-dashed p-10 text-center">
              <p className="text-sm text-muted-foreground">{t("newapi.empty")}</p>
              <Button variant="outline" size="sm" className="mt-4" onClick={() => setDialog({ open: true, provider: null })}>
                <Plus className="h-4 w-4" />
                {t("newapi.addProvider")}
              </Button>
            </div>
          ) : (
            <div className="grid gap-3.5 sm:grid-cols-2 xl:grid-cols-3">
              {shown.map(provider => {
                const active = provider.id === activeId && routingEnabled;
                const value = balanceValue(provider);
                const valuePositive = !!provider.balance && provider.balance.remaining > 0;
                return (
                  <div
                    key={provider.id}
                    className={cn(
                      "rounded-xl border bg-card p-4 transition-all hover:border-primary/30 hover:shadow-sm",
                      active && "border-amber-400/70 bg-amber-50/60 dark:border-amber-500/40 dark:bg-amber-950/20",
                    )}
                  >
                    <div className="flex items-start justify-between gap-2">
                      <div className="flex min-w-0 items-center gap-2">
                        <StatusDot provider={provider} active={active} />
                        <h3 className="truncate text-[15px] font-semibold">{provider.name}</h3>
                        <span className="shrink-0 text-[11px] text-muted-foreground">· {t("newapi.modelCount", { count: provider.models.length })}</span>
                      </div>
                      <Switch
                        checked={provider.enabled}
                        className={active ? "data-[state=checked]:bg-amber-500" : undefined}
                        disabled={lock(`enable:${provider.id}`)}
                        onCheckedChange={value =>
                          void run(`enable:${provider.id}`, async () => {
                            const change = await api.setNewApiProviderEnabled(provider.id, value);
                            applyChange(change);
                          }, t("newapi.enableFailed"))
                        }
                      />
                    </div>
                    <div className="mt-2.5 flex flex-wrap gap-1.5">
                      {provider.enabled && <Badge className={softBadge("green")}>{t("newapi.badgeEnabled")}</Badge>}
                      {provider.enabled && routingEnabled && <Badge className={softBadge("amber")}>{t("newapi.badgeLocked")}</Badge>}
                      {provider.testStatus === "untested" && <Badge className={softBadge("gray")}>{t("newapi.badgeUntested")}</Badge>}
                      {provider.testStatus === "ok" && <Badge className={softBadge("green")}>{t("newapi.badgeOk")}</Badge>}
                      {provider.testStatus === "failed" && <Badge className={softBadge("red")}>{t("newapi.badgeFailed")}</Badge>}
                    </div>
                    <div className={cn("mt-3 rounded-lg px-3.5 py-3", active ? "bg-amber-500/10" : "bg-muted/60")}>
                      <div className="flex items-center gap-2.5">
                        <span className="shrink-0 text-xs text-muted-foreground">{balanceLabel(provider)}</span>
                        <span className={cn("min-w-0 truncate text-lg font-semibold tabular-nums", active && valuePositive && "text-emerald-600 dark:text-emerald-400")}>
                          {value ?? "— —"}
                        </span>
                        <button
                          type="button"
                          className={iconButton}
                          title={t("newapi.refreshBalance")}
                          aria-label={t("newapi.refreshBalance")}
                          disabled={lock(`balance:${provider.id}`) || !provider.hasApiKey}
                          onClick={() =>
                            void run(`balance:${provider.id}`, async () => patchProvider(await api.queryNewApiProviderBalance(provider.id)), t("newapi.balanceFailed"))
                          }
                        >
                          {busy === `balance:${provider.id}` ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <RefreshCw className="h-3.5 w-3.5" />}
                        </button>
                      </div>
                      <p className="mt-0.5 truncate text-[11px] text-muted-foreground">{balanceCaption(provider)}</p>
                    </div>
                    <div className="mt-3 flex items-center justify-between gap-2">
                      <div className="flex min-w-0 items-center gap-1.5">
                        <List className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
                        <span className="truncate font-mono text-xs">{provider.model || t("newapi.noModel")}</span>
                        {provider.models.length > 1 && (
                          <span className="shrink-0 rounded bg-muted px-1 py-0.5 font-mono text-[10px] text-muted-foreground">
                            +{provider.models.length - 1}
                          </span>
                        )}
                      </div>
                      <div className="flex shrink-0 items-center gap-1">
                        <button
                          type="button"
                          className={cardButton}
                          title={t("newapi.test")}
                          aria-label={t("newapi.test")}
                          disabled={lock(`test:${provider.id}`) || !provider.hasApiKey}
                          onClick={() =>
                            void run(`test:${provider.id}`, async () => patchProvider(await api.testNewApiProvider(provider.id)), t("newapi.testFailed"))
                          }
                        >
                          {busy === `test:${provider.id}` ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <RadioTower className="h-3.5 w-3.5" />}
                        </button>
                        <button
                          type="button"
                          className={cardButton}
                          title={t(active ? "newapi.isCurrentRoute" : "newapi.setDefault")}
                          aria-label={t("newapi.setDefault")}
                          disabled={!provider.enabled || provider.id === activeId || lock(`active:${provider.id}`)}
                          onClick={() =>
                            void run(`active:${provider.id}`, async () => {
                              const change = await api.setNewApiActiveProvider(provider.id);
                              applyChange(change);
                            }, t("newapi.routeFailed"))
                          }
                        >
                          <Crosshair className={cn("h-3.5 w-3.5", active && "text-amber-500")} />
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
                    <p className="mt-1 truncate font-mono text-[11px] text-muted-foreground">{hostOf(provider)}</p>
                  </div>
                );
              })}
            </div>
          )}
        </div>
      </section>

      <ProviderDialog
        open={dialog.open}
        provider={dialog.provider}
        onOpenChange={open => setDialog(current => ({ ...current, open }))}
        onSaved={applyChange}
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
                    const change = await api.deleteNewApiProvider(provider.id);
                    applyChange(change);
                    if (filter === provider.id) setFilter("all");
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
