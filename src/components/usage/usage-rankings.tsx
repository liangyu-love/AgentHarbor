import { useEffect, useRef, useState, type CSSProperties, type ReactNode } from "react";
import type { TFunction } from "i18next";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { ArrowRight, ArrowUpRight, ChevronDown, CircleHelp, KeyRound, Network, Palette, RefreshCw, Waypoints } from "lucide-react";

import { ClientGlyph } from "@/components/newapi/configuration-shortcuts";
import { toast } from "@/hooks/use-toast";
import { useProviderBalanceUpdates } from "@/hooks/use-provider-balance-updates";
import { api } from "@/lib/api";
import { cacheHitRate, formatCacheRate } from "@/lib/cache-usage";
import { formatInvokeError } from "@/lib/invoke-error";
import { queryKeys } from "@/lib/query-keys";
import { cn } from "@/lib/utils";
import type { NewApiState } from "@/types/newapi";
import type { UsageGroup, UsagePayload } from "@/types";
import { clientLabel, coverageNotes, formatMetric, formatTokens, groupChannelLabel, metricValue, totalsCost, type UsageMetric } from "./usage-format";
import { UsageSegments } from "./usage-segments";

/** Rows a ranking shows before the rest fold away. */
const FOLD = 5;

const GLYPH_CLIENTS = ["codex", "claudeCode", "claudeDesktop", "opencode", "pi", "harness", "cursor"] as const;
type GlyphClient = typeof GLYPH_CLIENTS[number];

export function ClientIcon({ client }: { client: string }) {
  if ((GLYPH_CLIENTS as readonly string[]).includes(client)) return <ClientGlyph client={client as GlyphClient} />;
  if (client === "aggregate") return <Network aria-hidden />;
  if (client === "imageWorkbench") return <Palette aria-hidden />;
  if (client.startsWith("gateway:")) return <KeyRound aria-hidden />;
  return <CircleHelp aria-hidden />;
}

function sorted(groups: UsageGroup[], metric: UsageMetric) {
  return [...groups].sort((a, b) => metricValue(b.totals, metric) - metricValue(a.totals, metric)
    || b.totals.newTokens - a.totals.newTokens);
}

function RankRow({ group, metric, max, name, detail, icon, extra }: {
  group: UsageGroup; metric: UsageMetric; max: number; name: string; detail?: string | null; icon?: ReactNode; extra?: ReactNode;
}) {
  const { t } = useTranslation();
  const value = metricValue(group.totals, metric);
  const unpriced = metric === "cost" && group.totals.pricedCount === 0;
  return <div className="usage-rank-row" data-usage-rank={group.key}>
    <div className="usage-rank-name">
      {icon && <span className="usage-rank-icon">{icon}</span>}
      <span><b title={name}>{name}</b>{detail && <small title={detail}>{detail}</small>}</span>
    </div>
    <div className="usage-rank-value">
      {unpriced ? "—" : formatMetric(value, metric)}
      <small className="usage-cost-line">
        {metric === "cost"
          ? [`${formatTokens(group.totals.newTokens)} Token`, ...coverageNotes(group.totals, t)].join(" · ")
          : totalsCost(group.totals, t)}
      </small>
    </div>
    <div className="usage-rank-bar" aria-hidden>
      <i style={{ "--share": `${max > 0 ? Math.max(value > 0 ? 1.5 : 0, value / max * 100) : 0}%` } as CSSProperties} />
    </div>
    {extra && <div className="usage-rank-extra">{extra}</div>}
  </div>;
}

function RankCard({ kicker, title, subtitle, actions, children }: {
  kicker: string; title: string; subtitle?: string; actions?: ReactNode; children: ReactNode;
}) {
  return <section className="harbor-card route-rise flex min-w-0 flex-col" aria-label={title}>
    <div className="usage-card-head">
      <div className="usage-card-title">
        <span className="poster-kicker text-[9.5px] text-primary">{kicker}</span>
        <h3 className="font-display">{title}</h3>
        {subtitle && <p>{subtitle}</p>}
      </div>
      {actions}
    </div>
    {children}
  </section>;
}

/** The first rows, then the rest folded behind a fade that opens in place without a jump. */
function RankList({ items }: { items: ReactNode[] }) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const more = useRef<HTMLDivElement>(null);
  const folded = items.length > FOLD;
  // Folded rows stay mounted for the animation but out of reach of focus and readers.
  useEffect(() => { more.current?.toggleAttribute("inert", !open); }, [open, folded]);
  return <div className="usage-rank-list" data-open={open || undefined}>
    <div className="usage-rank" data-folded={(folded && !open) || undefined}>{items.slice(0, FOLD)}</div>
    {folded && <>
      <div ref={more} className="usage-rank-more" aria-hidden={!open}>
        <div><div className="usage-rank">{items.slice(FOLD)}</div></div>
      </div>
      <button type="button" className="usage-rank-toggle" aria-expanded={open} onClick={() => setOpen(!open)}>
        {open ? t("usage.rank.collapse") : t("usage.rank.showAll", { count: items.length })}
        <ChevronDown aria-hidden />
      </button>
    </>}
  </div>;
}

function rows(groups: UsageGroup[], metric: UsageMetric, render: (group: UsageGroup, max: number) => ReactNode) {
  const ordered = sorted(groups, metric);
  const max = Math.max(0, ...ordered.map(group => metricValue(group.totals, metric)));
  return ordered.length ? <RankList items={ordered.map(group => render(group, max))} /> : null;
}

function Empty() {
  const { t } = useTranslation();
  return <p className="usage-rank-empty">{t("usage.rank.empty")}</p>;
}

function cacheLine(group: UsageGroup, t: TFunction) {
  const rate = cacheHitRate(group.totals.cacheUsage);
  return rate == null ? null : <span>{t("accounts.cacheHitRate")} <strong>{formatCacheRate(rate)}</strong></span>;
}

function ChannelRanking({ data, metric, onManageProviders }: { data: UsagePayload; metric: UsageMetric; onManageProviders: () => void }) {
  useProviderBalanceUpdates();
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [refreshing, setRefreshing] = useState<string | null>(null);
  const providers = useQuery({ queryKey: queryKeys.providers, queryFn: api.providers.loadState, staleTime: 30_000 });
  const byId = new Map(providers.data?.providers.map(provider => [provider.id, provider]));
  const refresh = async (providerId: string) => {
    if (refreshing) return;
    setRefreshing(providerId);
    try {
      const next = await api.providers.queryBalance(providerId);
      queryClient.setQueryData<NewApiState>(queryKeys.providers, current => current ? {
        ...current, providers: current.providers.map(provider => provider.id === providerId
          ? { ...provider, balance: next.balance, balanceSource: next.balanceSource } : provider),
      } : current);
    } catch (error) {
      toast({ title: t("newapi.balanceFailed"), description: formatInvokeError(error), variant: "destructive" });
    } finally {
      setRefreshing(null);
    }
  };
  const balanceText = (providerId: string) => {
    const balance = byId.get(providerId)?.balance;
    if (!balance) return null;
    const amount = new Intl.NumberFormat(undefined, { maximumFractionDigits: 2 })
      .format(balance.conversionAvailable ? balance.remaining : balance.rawQuota);
    return balance.conversionAvailable ? `${amount} ${balance.unit}` : `${amount} ${t("newapi.balanceQuota")}`;
  };
  return <RankCard kicker="Channels" title={t("usage.rank.channels")}
    actions={<button type="button" className="inline-flex items-center gap-1 rounded-lg px-2 py-1 text-xs font-medium text-primary hover:bg-primary/10"
      onClick={onManageProviders}>{t("usage.rank.manageProviders")}<ArrowRight className="h-3.5 w-3.5" /></button>}>
    {rows(data.byChannel, metric, (group, max) => {
      const provider = group.providerId ? byId.get(group.providerId) : undefined;
      const name = groupChannelLabel(group, t);
      const balance = group.providerId ? balanceText(group.providerId) : null;
      return <RankRow key={group.key} group={group} metric={metric} max={max} name={name}
        icon={group.channel === "official" ? <Waypoints aria-hidden /> : group.channel === "aggregate" ? <Network aria-hidden /> : <span className="text-[11px] font-bold">{name.charAt(0).toUpperCase()}</span>}
        extra={<>
          {cacheLine(group, t)}
          {group.providerId && <span title={provider?.balance ? new Date(provider.balance.checkedAt * 1000).toLocaleString() : undefined}>
            {t("accounts.balanceRemaining")} <strong>{balance ?? "—"}</strong>
          </span>}
          {group.providerId && <button type="button" className="usage-icon-button"
            disabled={!provider || !(provider.hasApiKey || provider.hasAccessToken) || refreshing !== null}
            aria-label={`${t("accounts.refreshBalance")} ${name}`} title={t("accounts.refreshBalance")}
            onClick={() => void refresh(group.providerId!)}>
            <RefreshCw className={cn("h-3.5 w-3.5", refreshing === group.providerId && "animate-spin")} />
          </button>}
        </>} />;
    }) ?? <Empty />}
  </RankCard>;
}

function WorkRanking({ data, metric, onOpenThreads }: { data: UsagePayload; metric: UsageMetric; onOpenThreads: (client: string) => void }) {
  const { t } = useTranslation();
  const [view, setView] = useState<"projects" | "sessions">("projects");
  return <RankCard kicker="Work" title={t("usage.rank.work")} subtitle={t("usage.rank.workHint")}
    actions={<UsageSegments label={t("usage.rank.work")} value={view} onChange={setView}
      items={(["projects", "sessions"] as const).map(value => ({ value, label: t(`usage.rank.${value}`) }))} />}>
    {view === "projects"
      ? rows(data.byProject, metric, (group, max) => <RankRow key={group.key} group={group} metric={metric} max={max}
          name={group.label} detail={group.detail} />) ?? <Empty />
      : rows(data.bySession, metric, (group, max) => {
          const client = group.client ?? "unknown";
          return <RankRow key={group.key} group={group} metric={metric} max={max} icon={<ClientIcon client={client} />}
            name={group.label.length > 18 ? `${group.label.slice(0, 8)}…${group.label.slice(-6)}` : group.label}
            detail={[group.detail, clientLabel(client, t)].filter(Boolean).join(" · ")}
            extra={(client === "codex" || client === "claudeCode") && <button type="button"
              className="inline-flex items-center gap-1 rounded-md text-primary hover:underline"
              onClick={() => onOpenThreads(client)}>{t("usage.rank.openThreads")}<ArrowUpRight className="h-3 w-3" /></button>} />;
        }) ?? <Empty />}
  </RankCard>;
}

export function UsageRankings({ data, metric, onManageProviders, onOpenThreads }: {
  data: UsagePayload; metric: UsageMetric; onManageProviders: () => void; onOpenThreads: (client: string) => void;
}) {
  const { t } = useTranslation();
  const official = t("usage.channel.official");
  // Rows of a pair share one height; folding keeps both near the same length.
  return <div className="grid gap-5 lg:grid-cols-2 min-[1800px]:grid-cols-4">
    <RankCard kicker="Clients" title={t("usage.rank.clients")}>
      {rows(data.byClient, metric, (group, max) => <RankRow key={group.key} group={group} metric={metric} max={max}
        name={clientLabel(group.key, t)} icon={<ClientIcon client={group.key} />} extra={cacheLine(group, t)} />) ?? <Empty />}
    </RankCard>
    <ChannelRanking data={data} metric={metric} onManageProviders={onManageProviders} />
    <RankCard kicker="Models" title={t("usage.rank.models")}>
      {rows(data.byModel, metric, (group, max) => <RankRow key={group.key} group={group} metric={metric} max={max}
        name={group.label} detail={group.detail?.split(" · ").map(part => part === "official" ? official : part).join(" · ")} />) ?? <Empty />}
    </RankCard>
    <WorkRanking data={data} metric={metric} onOpenThreads={onOpenThreads} />
  </div>;
}
