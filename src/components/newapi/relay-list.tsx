import { useTranslation } from "react-i18next";
import { Activity, FileDown, FileUp, Plus, RadioTower, Search, Waypoints, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { CountUp } from "@/components/ui/count-up";
import { RainbowButton } from "@/registry/magicui/rainbow-button";
import {
  DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { cn } from "@/lib/utils";
import { RelayCard } from "@/components/newapi/relay-card";
import { relayHue } from "@/components/newapi/relay-groups";
import { ClientGlyph } from "@/components/newapi/configuration-shortcuts";
import { useRelayConsumers, type RelayConsumer } from "@/components/newapi/use-relay-consumers";
import type { ProviderState } from "@/components/newapi/use-provider-state";
import type { RelayView } from "@/components/newapi/use-relay-view";
import type { NewApiProvider } from "@/types/newapi";

const CONSUMER_ORDER: RelayConsumer[] = ["codex", "claudeCode", "claudeDesktop", "opencode", "pi"];
const GRID = "grid items-start gap-x-4 gap-y-3 md:grid-cols-2 2xl:grid-cols-3";

function Stat({ label, children, caption, className }: { label: string; children: React.ReactNode; caption: React.ReactNode; className?: string }) {
  return (
    <div className={cn("relative min-w-0 overflow-hidden rounded-2xl bg-background/75 p-3.5 ring-1 ring-border/70 backdrop-blur-sm", className)}>
      <p className="truncate text-[11px] font-medium text-muted-foreground">{label}</p>
      <div className="poster-number mt-1 flex min-w-0 items-baseline gap-1.5 text-[1.75rem] font-bold leading-tight tracking-tight">{children}</div>
      <div className="mt-1 truncate text-[11px] text-muted-foreground">{caption}</div>
    </div>
  );
}

// Relay library: an overview of every relay site, a search and site filter,
// and the card grid.
export function RelayList({ store, view, providers, onEdit, onTest, onRemove, onBalance, onCapabilities }: {
  store: ProviderState;
  view: RelayView;
  providers: NewApiProvider[];
  onEdit: (provider: NewApiProvider | null) => void;
  onTest: (provider: NewApiProvider) => void;
  onRemove: (provider: NewApiProvider) => void;
  onBalance: (provider: NewApiProvider) => void;
  onCapabilities: (provider: NewApiProvider) => void;
}) {
  const { t } = useTranslation();
  const { state, mutationLocked, exportBusy, importProviders, exportProviders } = store;
  const { filter, setFilter, search, setSearch, groups, shown, groupLabel } = view;
  const consumers = useRelayConsumers(state.data);

  const router = state.data?.router;
  const enabled = providers.filter(provider => provider.enabled).length;
  const injected = [...consumers.values()].filter(use => use.codex === "injected").length;
  const direct = providers.find(provider => consumers.get(provider.id)?.codex === "direct");
  const tests = { ok: 0, failed: 0, untested: 0 };
  for (const provider of providers) tests[provider.testStatus]++;
  const users = CONSUMER_ORDER.filter(consumer => [...consumers.values()].some(use => use.consumers.includes(consumer)));
  const ratio = providers.length ? enabled / providers.length : 0;
  const codexCaption = !router?.enabled ? t("newapi.relayCodexOff")
    : router.accessMode === "native" ? t("newapi.relayCodexDirect")
    : router.aggregateModelId ? t("newapi.relayCodexAggregate") : t("newapi.relayCodexInjected");

  return (
    <section className="space-y-4" aria-labelledby="newapi-providers-title">
      <div className="route-surface route-rise relative overflow-hidden rounded-3xl border border-border/80 bg-card p-5 shadow-sm sm:p-6">
        <div className="relative flex flex-wrap items-start justify-between gap-4">
          <div className="min-w-0">
            <div className="flex items-center gap-2.5">
              <span className="flex h-10 w-10 items-center justify-center rounded-2xl bg-primary/10 text-primary ring-1 ring-primary/20">
                <RadioTower className="h-[18px] w-[18px]" />
              </span>
              <div className="min-w-0">
                <h2 id="newapi-providers-title" className="poster-title text-2xl">{t("newapi.providersSection")}</h2>
                <p className="text-[13px] text-muted-foreground">{t("newapi.relayLibraryDesc")}</p>
              </div>
            </div>
          </div>
          <div className="flex items-center gap-2">
            <RainbowButton className="h-9 rounded-xl px-3.5" onClick={() => onEdit(null)} disabled={mutationLocked}>
              <Plus className="h-4 w-4" />
              {t("newapi.addProvider")}
            </RainbowButton>
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button variant="outline" size="icon" className="h-9 w-9 rounded-xl bg-background/80" aria-label={t("newapi.importExport")}>
                  <FileUp className="h-4 w-4" />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                <DropdownMenuItem onClick={() => void importProviders()} disabled={mutationLocked}>
                  <FileUp className="h-4 w-4" />
                  {t("newapi.importProviders")}
                </DropdownMenuItem>
                <DropdownMenuItem onClick={() => void exportProviders()} disabled={providers.length === 0 || exportBusy}>
                  <FileDown className="h-4 w-4" />
                  {t("newapi.exportProviders")}
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          </div>
        </div>

        <div className="relative mt-5 grid gap-3 sm:grid-cols-2 xl:grid-cols-4" data-relay-overview>
          <Stat label={t("newapi.relayStatSites")}
            caption={t("newapi.relayStatConfigs", { count: providers.length })}>
            <CountUp value={groups.length} />
            <span className="text-xs font-medium text-muted-foreground">{t("newapi.relayStatSitesUnit")}</span>
          </Stat>
          <Stat label={t("newapi.relayStatEnabled")}
            caption={t("newapi.relayStatEnabledOf", { total: providers.length })}>
            <CountUp value={enabled} />
            <svg aria-hidden viewBox="0 0 36 36" className="ml-auto h-9 w-9 -rotate-90 self-center">
              <circle cx="18" cy="18" r="15" fill="none" className="stroke-muted" strokeWidth="4" />
              <circle cx="18" cy="18" r="15" fill="none" className="stroke-primary transition-[stroke-dashoffset] duration-700"
                strokeWidth="4" strokeLinecap="round" pathLength={100} strokeDasharray="100" strokeDashoffset={100 - ratio * 100} />
            </svg>
          </Stat>
          <Stat label={t("newapi.relayStatCodex")} caption={codexCaption}
            className={cn(injected > 0 && "ring-primary/35")}>
            {router?.enabled && router.accessMode === "native"
              ? <span className="truncate text-lg">{direct?.name ?? "—"}</span>
              : router?.enabled && router.aggregateModelId
                ? <span className="flex items-center gap-1.5 text-lg"><Waypoints className="h-4 w-4 text-primary" />{t("newapi.topology.dest.aggregation")}</span>
                : <>
                  <CountUp value={router?.enabled ? injected : 0} />
                  {injected > 0 && <span aria-hidden className="relay-live-dot ml-1 self-center" />}
                </>}
          </Stat>
          <Stat label={t("newapi.relayStatHealth")}
            caption={<span className="flex items-center gap-2">
              <span className="text-emerald-600 dark:text-emerald-400">{t("newapi.badgeOk")} {tests.ok}</span>
              <span className="text-red-600 dark:text-red-400">{t("newapi.badgeFailed")} {tests.failed}</span>
              <span>{t("newapi.badgeUntested")} {tests.untested}</span>
            </span>}>
            <Activity className="h-5 w-5 self-center text-primary" />
            <span className="ml-1 flex h-2 min-w-0 flex-1 self-center overflow-hidden rounded-full bg-muted">
              {providers.length > 0 && <>
                <span className="h-full bg-emerald-500 transition-[width] duration-700" style={{ width: `${tests.ok / providers.length * 100}%` }} />
                <span className="h-full bg-red-500 transition-[width] duration-700" style={{ width: `${tests.failed / providers.length * 100}%` }} />
              </>}
            </span>
          </Stat>
        </div>

        {users.length > 0 && (
          <div className="relative mt-4 flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
            <span>{t("newapi.relayUsedBy")}</span>
            {users.map(consumer => (
              <span key={consumer} className="inline-flex items-center gap-1.5 rounded-full bg-background/80 py-0.5 pl-1 pr-2.5 text-foreground ring-1 ring-border/70">
                <span className="flex h-5 w-5 items-center justify-center rounded-full bg-muted/60">
                  <ClientGlyph client={consumer} className="h-3 w-3" />
                </span>
                {t(`newapi.topology.client.${consumer}`)}
              </span>
            ))}
          </div>
        )}
      </div>

      <div className="flex flex-wrap items-center gap-2">
        <label className="relative flex h-9 min-w-[12rem] flex-1 items-center sm:max-w-xs">
          <Search aria-hidden className="pointer-events-none absolute left-3 h-4 w-4 text-muted-foreground" />
          <input value={search} onChange={event => setSearch(event.target.value)} placeholder={t("newapi.relaySearch")}
            aria-label={t("newapi.relaySearch")}
            className="h-9 w-full rounded-xl border border-border bg-card pl-9 pr-8 text-sm shadow-sm outline-none transition-colors placeholder:text-muted-foreground/70 focus:border-primary/50 focus:ring-2 focus:ring-primary/15" />
          {search && (
            <button type="button" onClick={() => setSearch("")} aria-label={t("newapi.relaySearchClear")}
              className="absolute right-2 rounded-md p-0.5 text-muted-foreground hover:bg-muted hover:text-foreground">
              <X className="h-3.5 w-3.5" />
            </button>
          )}
        </label>
        <div className="flex min-w-0 flex-1 items-center gap-1.5 overflow-x-auto pb-0.5" role="toolbar" aria-label={t("newapi.relaySites")}>
          <button type="button" onClick={() => setFilter("all")} aria-pressed={filter === "all"}
            className={cn("relay-chip", filter === "all" && "relay-chip-on")}>
            {t("newapi.allProviders")}
          </button>
          {groups.map(group => (
            <button key={group.key} type="button" onClick={() => setFilter(group.key)} aria-pressed={filter === group.key}
              className={cn("relay-chip", filter === group.key && "relay-chip-on")}
              style={{ "--relay-hue": relayHue(group.key) } as React.CSSProperties}>
              <span aria-hidden className="relay-chip-dot" />
              {groupLabel(group)}
              <span className="text-[10px] text-muted-foreground">{group.providers.length}</span>
            </button>
          ))}
        </div>
      </div>

      {state.isLoading ? (
        <div className={GRID}>
          {[0, 1, 2].map(i => (
            <div key={i} className="pt-3">
              <div className="space-y-3 rounded-[20px] border border-border bg-card p-4">
                <div className="flex items-center gap-3">
                  <Skeleton className="h-10 w-10 rounded-2xl" />
                  <div className="flex-1 space-y-1.5"><Skeleton className="h-4 w-28" /><Skeleton className="h-3 w-20" /></div>
                  <Skeleton className="h-5 w-9 rounded-full" />
                </div>
                <Skeleton className="h-20 w-full rounded-2xl" />
                <Skeleton className="h-6 w-full rounded-md" />
              </div>
            </div>
          ))}
        </div>
      ) : shown.length === 0 ? (
        <div className="route-surface rounded-3xl border border-dashed border-border/80 bg-card/60 px-6 py-14 text-center">
          <div className="mx-auto flex h-14 w-14 items-center justify-center rounded-2xl bg-primary/10 ring-1 ring-primary/20">
            <RadioTower className="h-6 w-6 text-primary" />
          </div>
          <p className="mt-4 text-sm text-muted-foreground">{t(search ? "newapi.relayNoMatch" : "newapi.empty")}</p>
          {search ? (
            <Button variant="outline" size="sm" className="mt-5" onClick={() => setSearch("")}>
              <X className="h-4 w-4" />{t("newapi.relaySearchClear")}
            </Button>
          ) : (
            <Button variant="outline" size="sm" className="mt-5" onClick={() => onEdit(null)}>
              <Plus className="h-4 w-4" />{t("newapi.addProvider")}
            </Button>
          )}
        </div>
      ) : (
        <div className={GRID}>
          {shown.map((group, index) => (
            <div key={group.key} className="route-rise min-w-0" style={{ animationDelay: `${Math.min(index, 8) * 40}ms` }}>
              <RelayCard group={group} store={store} view={view} consumers={consumers}
                onEdit={onEdit} onTest={onTest} onRemove={onRemove} onBalance={onBalance} onCapabilities={onCapabilities} />
            </div>
          ))}
        </div>
      )}
    </section>
  );
}
