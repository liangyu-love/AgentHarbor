import { useEffect, useId, useRef, useState, type ReactNode } from "react";
import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import {
  ArrowRight, ArrowUpRight, Check, ChevronDown, CircleDashed, Copy, Cpu, FileCode2, FolderOpen, Image as ImageIcon,
  MessagesSquare, RadioTower, RotateCw, Server, Settings, ShieldCheck, Sparkles, UsersRound, Waypoints, Wrench, Zap,
} from "lucide-react";
import { api } from "@/lib/api";
import { Button } from "@/components/ui/button";
import { CountUp } from "@/components/ui/count-up";
import { toast } from "@/hooks/use-toast";
import { cn } from "@/lib/utils";
import type { Route } from "@/types/navigation";
import type { CodexAccountSummary } from "@/types";
import type { ClientFiles, ClientKeyFile } from "@/types/clients";
import { queryKeys } from "@/lib/query-keys";
import { errorMessage, formatInvokeError } from "@/lib/invoke-error";
import { formatRelative } from "@/lib/format-time";
import { storageKeys } from "@/lib/storage-keys";
import { ClientGlyph } from "@/components/newapi/configuration-shortcuts";
import { CLIENTS, useRoutingLinks, type Client, type Link } from "@/components/newapi/use-routing-links";
import { formatTokenCount } from "@/components/threads/thread-utils";
import { useFold } from "@/hooks/use-fold";

/** A client's key files as small status tags; hovering shows the full path. */
function KeyFiles({ group, name }: { group?: ClientFiles; name: string }) {
  const { t } = useTranslation();
  if (!group?.files.length) return null;
  // Desktop's two configs share a file name, so they are labelled by role.
  const label = (file: ClientKeyFile) => group.client === "claudeDesktop"
    ? t(`workspace.desktopFile.${file.id}`, { defaultValue: file.name }) : file.name;
  return (
    <ul className="data-files" aria-label={t("workspace.keyFiles", { name })}>
      {group.files.map((file) => (
        <li key={file.id} data-file={file.id} data-exists={file.exists || undefined} title={file.path}>
          {file.exists ? <Check aria-hidden /> : <CircleDashed aria-hidden />}
          <span className="data-file-name">{label(file)}</span>
          <span className="sr-only">{t(file.exists ? "workspace.filePresent" : "workspace.fileMissing")}</span>
        </li>
      ))}
    </ul>
  );
}

const ORBIT = 132;
// Clients sit on a ring around the hub, starting at the top.
const orbitPoint = (index: number) => {
  const angle = (-90 + index * (360 / CLIENTS.length)) * Math.PI / 180;
  return { x: 160 + Math.cos(angle) * ORBIT, y: 160 + Math.sin(angle) * ORBIT };
};

/** The live routing picture: every client around AgentHarbor, lit when it routes through it. */
function OrbitHub({ links, running, onOpen }: { links: Record<Client, Link>; running: boolean; onOpen: () => void }) {
  const { t } = useTranslation();
  const id = useId().replace(/:/g, "");
  const reduced = typeof window !== "undefined" && (window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false);
  const routed = CLIENTS.filter((client) => links[client] === "routed").length;
  const live = running && routed > 0;
  return (
    <button type="button" onClick={onOpen} data-dash-orbit data-live={live || undefined}
      aria-label={t("workspace.orbitLabel")} title={t("workspace.manageConnection")}
      className="dash-orbit group relative mx-auto aspect-square w-full max-w-[320px] rounded-full focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring">
      <svg viewBox="0 0 320 320" className="absolute inset-0 h-full w-full overflow-visible" aria-hidden>
        <defs>
          {/* User space: an object bounding box is empty for vertical lines. */}
          <linearGradient id={`${id}-line`} gradientUnits="userSpaceOnUse" x1="0" y1="0" x2="320" y2="320">
            <stop offset="0%" stopColor="hsl(var(--primary))" />
            <stop offset="100%" stopColor="hsl(var(--primary) / 0.55)" />
          </linearGradient>
          <radialGradient id={`${id}-glow`}>
            <stop offset="0%" stopColor="hsl(var(--primary))" stopOpacity="0.35" />
            <stop offset="100%" stopColor="hsl(var(--primary))" stopOpacity="0" />
          </radialGradient>
          <path id={`${id}-ring`} d="M160,160 m-157,0 a157,157 0 1,1 314,0 a157,157 0 1,1 -314,0" />
        </defs>
        <g className="orbit-text-spin">
          <text className="orbit-text"><textPath href={`#${id}-ring`}>AGENTHARBOR · LOCAL ROUTER · CODEX · CLAUDE · OPENCODE · PI · HARNESS · CURSOR ·</textPath></text>
        </g>
        <circle cx="160" cy="160" r={ORBIT} fill="none" className="dash-orbit-ring stroke-muted-foreground/30" strokeWidth="1.2" strokeDasharray="2 7" />
        <circle cx="160" cy="160" r={ORBIT - 46} fill="none" className="stroke-muted-foreground/20" strokeDasharray="1 6" />
        {live && <circle cx="160" cy="160" r="110" fill={`url(#${id}-glow)`} />}
        {CLIENTS.map((client, index) => {
          const point = orbitPoint(index);
          const link = links[client];
          const d = `M160,160 L${point.x},${point.y}`;
          return (
            <g key={client} data-orbit-link={client} data-state={link}>
              <path d={d} fill="none" className="stroke-muted-foreground/30" strokeWidth="1" strokeDasharray={link === "idle" ? "2 5" : undefined} />
              {link !== "idle" && (running || link === "direct") && <>
                <path d={d} fill="none" stroke={link === "routed" ? `url(#${id}-line)` : "rgb(16 185 129)"} strokeWidth="2" strokeLinecap="round"
                  className={link === "routed" ? "route-flow" : "route-flow-slow"} />
                {!reduced && <circle r="3" fill={link === "routed" ? "hsl(var(--primary))" : "rgb(16 185 129)"}>
                  <animateMotion dur={`${1.6 + index * 0.17}s`} repeatCount="indefinite" path={`M${point.x},${point.y} L160,160`} />
                </circle>}
              </>}
            </g>
          );
        })}
      </svg>
      <span className={cn("absolute left-1/2 top-1/2 flex h-[92px] w-[92px] -translate-x-1/2 -translate-y-1/2 flex-col items-center justify-center rounded-full border bg-card/95 shadow-lg backdrop-blur",
        live ? "border-primary/40 shadow-primary/25" : "border-border")}>
        {live && <span aria-hidden className="route-hub-ring absolute -inset-1.5 rounded-full" />}
        <Waypoints className={cn("relative h-4 w-4", live ? "text-primary" : "text-muted-foreground")} />
        <span className="poster-number relative mt-0.5 text-[26px] font-bold leading-none">
          {routed}<small className="text-[13px] font-semibold text-muted-foreground">/{CLIENTS.length}</small>
        </span>
        <span className="relative mt-1 text-[10.5px] font-semibold text-muted-foreground">{t(live ? "newapi.topology.hubRouting" : running ? "newapi.topology.hubListening" : "newapi.topology.hubStopped")}</span>
      </span>
      {CLIENTS.map((client, index) => {
        const point = orbitPoint(index);
        const link = links[client];
        return (
          <span key={client} title={`${t(`newapi.topology.client.${client}`)} · ${t(`newapi.topology.link.${link}`)}`}
            className="absolute flex flex-col items-center gap-1"
            style={{ left: `${point.x / 3.2}%`, top: `${point.y / 3.2}%`, transform: "translate(-50%, -22px)" }}>
            <span className={cn("relative flex h-11 w-11 items-center justify-center rounded-2xl border bg-card shadow-sm transition-transform duration-300 group-hover:scale-105",
              link === "routed" ? "border-primary/50" : link === "direct" ? "border-emerald-500/50" : "border-border opacity-75")}>
              <ClientGlyph client={client} className="h-5 w-5" />
              <span aria-hidden className={cn("absolute -right-0.5 -top-0.5 h-2.5 w-2.5 rounded-full border-2 border-card",
                link === "routed" ? "bg-primary" : link === "direct" ? "bg-emerald-500" : "bg-muted-foreground/40")} />
            </span>
            <span className={cn("whitespace-nowrap rounded-full bg-card/90 px-1.5 font-display text-[10px] font-semibold leading-4",
              link === "idle" ? "text-muted-foreground" : "text-foreground")}>{t(`newapi.topology.client.${client}`)}</span>
          </span>
        );
      })}
    </button>
  );
}

/** A soft area chart of the last days of token use. */
function Sparkline({ values }: { values: number[] }) {
  const id = useId().replace(/:/g, "");
  if (values.length < 2) return null;
  const max = Math.max(...values, 1);
  const points = values.map((value, index) => [index / (values.length - 1) * 100, 36 - value / max * 32] as const);
  const line = points.map(([x, y], index) => `${index ? "L" : "M"}${x.toFixed(2)},${y.toFixed(2)}`).join(" ");
  return (
    <svg viewBox="0 0 100 40" preserveAspectRatio="none" className="dash-spark h-12 w-full" aria-hidden>
      <defs>
        <linearGradient id={`${id}-fill`} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%" stopColor="hsl(var(--primary))" stopOpacity="0.32" />
          <stop offset="100%" stopColor="hsl(var(--primary))" stopOpacity="0" />
        </linearGradient>
      </defs>
      <path d={`${line} L100,40 L0,40 Z`} fill={`url(#${id}-fill)`} />
      <path d={line} fill="none" stroke="hsl(var(--primary))" strokeWidth="1.6" vectorEffect="non-scaling-stroke" className="dash-spark-line" pathLength={1} />
    </svg>
  );
}

function Tile({ icon, label, onClick, children, caption, className }: {
  icon: ReactNode; label: string; onClick: () => void; children: ReactNode; caption: ReactNode; className?: string;
}) {
  return (
    <button type="button" onClick={onClick} data-spotlight className={cn("dash-tile group", className)}>
      <span className="flex items-center justify-between gap-2 text-[12px] font-medium text-muted-foreground">
        <span className="flex items-center gap-2"><span className="dash-tile-icon">{icon}</span>{label}</span>
        <ArrowUpRight aria-hidden className="h-3.5 w-3.5 opacity-0 transition-all group-hover:-translate-y-0.5 group-hover:translate-x-0.5 group-hover:opacity-100" />
      </span>
      <span className="mt-3 block min-w-0">{children}</span>
      <span className="mt-auto block truncate pt-2 text-[11.5px] text-muted-foreground">{caption}</span>
    </button>
  );
}

const maskEmail = (email: string) => {
  const [local, domain] = email.split("@");
  return domain ? `${local.slice(0, 1)}${"•".repeat(Math.max(3, local.length - 2))}${local.length > 1 ? local.slice(-1) : ""}@${domain}` : email;
};

function QuotaRing({ account }: { account: CodexAccountSummary }) {
  const used = account.primaryWindow?.usedPercent ?? null;
  const tone = used === null ? "stroke-muted-foreground/40" : used >= 85 ? "stroke-red-500" : used >= 60 ? "stroke-amber-500" : "stroke-emerald-500";
  return (
    <svg viewBox="0 0 40 40" className="h-12 w-12 shrink-0 -rotate-90" aria-hidden>
      <circle cx="20" cy="20" r="16" fill="none" className="stroke-muted" strokeWidth="4" />
      <circle cx="20" cy="20" r="16" fill="none" className={cn(tone, "transition-[stroke-dashoffset] duration-700")} strokeWidth="4"
        strokeLinecap="round" pathLength={100} strokeDasharray="100" strokeDashoffset={100 - (used ?? 0)} />
    </svg>
  );
}

type RecentSource = "all" | "codex" | "claudeCode";

export function OverviewPage({ onNavigate, onOpenProxySettings }: {
  onNavigate: (route: Route) => void;
  onOpenProxySettings: () => void;
}) {
  const { t, i18n } = useTranslation();
  const [copiedPath, setCopiedPath] = useState<string | null>(null);
  // Collapsed until the user opens it; the choice is remembered.
  const dataMap = useFold("overview-data-map", false);
  const dataMapRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    dataMapRef.current?.toggleAttribute("inert", !dataMap.open);
  }, [dataMap.open]);

  const snapshot = useQuery({ queryKey: queryKeys.runtimeStateDisplay, queryFn: () => api.loadSnapshot(true) });
  const newapi = useQuery({ queryKey: queryKeys.providers, queryFn: api.providers.loadState });
  const mcp = useQuery({ queryKey: [...queryKeys.mcpServers, "codex"], queryFn: () => api.loadMcpServers("codex") });
  const skills = useQuery({ queryKey: [...queryKeys.installedSkills, "codex"], queryFn: () => api.loadInstalledSkills("codex") });
  const skillBackups = useQuery({ queryKey: [...queryKeys.skillBackups, "codex"], queryFn: () => api.loadSkillBackups("codex") });
  const claudeSkills = useQuery({ queryKey: [...queryKeys.installedSkills, "claudeCode"], queryFn: () => api.loadInstalledSkills("claudeCode") });
  const claudeBackups = useQuery({ queryKey: [...queryKeys.skillBackups, "claudeCode"], queryFn: () => api.loadSkillBackups("claudeCode") });
  const instructions = useQuery({ queryKey: [...queryKeys.customInstructionState, "codex"], queryFn: () => api.loadCustomInstructionState("codex") });
  const systemInfo = useQuery({ queryKey: queryKeys.systemInfo, queryFn: api.getSystemInfo, staleTime: Infinity });
  // Stored results only: refreshing would query every account remotely.
  const accounts = useQuery({ queryKey: queryKeys.codexAccounts, queryFn: api.accounts.list, staleTime: 60_000, refetchOnWindowFocus: false });
  const usage = useQuery({ queryKey: queryKeys.tokenUsage, queryFn: api.usage.overview, staleTime: 5 * 60_000, refetchOnWindowFocus: false, retry: false });
  const threads = useQuery({ queryKey: queryKeys.conversations("codex"), queryFn: () => api.conversations.list("codex"), staleTime: 30_000 });
  const claudeThreads = useQuery({ queryKey: queryKeys.conversations("claudeCode"), queryFn: () => api.conversations.list("claudeCode"), staleTime: 30_000 });
  const [recentSource, setRecentSource] = useState<RecentSource>("all");
  const clientConfigs = useQuery({ queryKey: queryKeys.clientConfigs, queryFn: api.targets.configs });
  const clientFiles = useQuery({ queryKey: queryKeys.clientFiles, queryFn: api.targets.files, refetchOnWindowFocus: false });
  const desktop = useQuery({ queryKey: queryKeys.claudeDesktopStatus, queryFn: api.claudeDesktop.status, refetchOnWindowFocus: false });
  const cursor = useQuery({ queryKey: queryKeys.cursorStatus, queryFn: api.cursor.status, refetchOnWindowFocus: false });
  const { links } = useRoutingLinks(newapi.data?.router);

  const status = snapshot.data?.data.status;
  const errors = [snapshot.error, newapi.error, mcp.error, skills.error, skillBackups.error, claudeSkills.error, claudeBackups.error, instructions.error].filter(Boolean);
  const busy = [snapshot, newapi, mcp, skills, claudeSkills, instructions, accounts, usage, threads, claudeThreads, clientFiles, cursor].some((query) => query.isFetching);

  const refresh = () => {
    void Promise.all([
      snapshot.refetch(), newapi.refetch(), mcp.refetch(), skills.refetch(), skillBackups.refetch(), claudeSkills.refetch(),
      claudeBackups.refetch(), instructions.refetch(), accounts.refetch(), usage.refetch(), threads.refetch(), claudeThreads.refetch(), clientFiles.refetch(), cursor.refetch(),
    ]);
  };

  const handleCopyPath = async (path: string, key: string) => {
    if (!path) return;
    try {
      await navigator.clipboard.writeText(path);
      setCopiedPath(key);
      window.setTimeout(() => setCopiedPath((current) => current === key ? null : current), 2000);
      toast({ title: t("workspace.pathCopied") });
    } catch (error) {
      toast({ title: t("common.error"), description: formatInvokeError(error), variant: "destructive" });
    }
  };
  const handleOpenFolder = async (path: string) => {
    if (!path) return;
    try {
      await api.openPath(path);
    } catch (error) {
      toast({ title: t("common.error"), description: formatInvokeError(error), variant: "destructive" });
    }
  };
  const handleRestartCodex = async () => {
    try {
      await api.targets.restart("codex");
      toast({ title: t("workspace.restartSent") });
    } catch (error) {
      toast({ title: t("common.error"), description: formatInvokeError(error), variant: "destructive" });
    }
  };

  const now = new Date();
  const hour = now.getHours();
  const dateLine = now.toLocaleDateString(i18n.language, { weekday: "long", month: "long", day: "numeric" });
  const greeting = hour >= 5 && hour < 12 ? t("workspace.goodMorning")
    : hour >= 12 && hour < 18 ? t("workspace.goodAfternoon") : t("workspace.goodEvening");

  const proxyRunning = newapi.data?.proxyRunning ?? false;
  const router = newapi.data?.router;
  const routingEnabled = router?.enabled ?? false;
  const apiProxy = status?.api.proxy;
  const apiProxyMode = !status
    ? t(snapshot.isError ? "workspace.checkFailed" : "workspace.checking")
    : t(apiProxy?.mode === "manual" ? "settings.apiProxyModeManual"
      : apiProxy?.mode === "direct" ? "settings.apiProxyModeDirect" : "settings.apiProxyModeSystem");
  const providers = newapi.data?.providers ?? [];
  const enabledProviders = providers.filter((provider) => provider.enabled).length;
  const routedCount = CLIENTS.filter((client) => links[client] === "routed").length;
  const mcpServers = mcp.data?.data.items ?? [];
  const mcpEnabledCount = mcpServers.filter((server) => server.enabled).length;
  const skillsCount = (skills.data?.data.total ?? 0) + (claudeSkills.data?.data.total ?? 0);
  const backupsCount = (skillBackups.data?.data.total ?? 0) + (claudeBackups.data?.data.total ?? 0);
  const skillsAvailable = !!skills.data && !!claudeSkills.data && !skills.isError && !claudeSkills.isError;
  const backupsAvailable = !!skillBackups.data && !!claudeBackups.data && !skillBackups.isError && !claudeBackups.isError;
  const instructionManaged = instructions.data?.data.current.managedBlockPresent ?? false;
  const lastTemplateTitle = instructions.data?.data.current.lastTemplateTitle;
  // Every client's data on this machine, in one list.
  const parentDir = (file: string) => file.replace(/[\\/][^\\/]*$/, "");
  const configDir = (target: string) => parentDir(clientConfigs.data?.find((client) => client.targetId === target)?.path ?? "");
  const filesOf = (key: string) => clientFiles.data?.find((group) => group.client === key);
  const harness = clientConfigs.data?.find((client) => client.targetId === "harness");
  // A client whose key files exist is located even when no other source names its directory.
  const locatedDir = (key: string) => {
    const files = filesOf(key)?.files ?? [];
    return files.some((file) => file.exists) ? parentDir(files[0].path) : "";
  };
  const dataLocations: { key: string; name: string; mark: ReactNode; path: string; ok: boolean; detail: string }[] = [
    { key: "codex", name: "Codex", mark: <ClientGlyph client="codex" className="h-4 w-4" />, path: status?.paths.codexHome ?? "",
      ok: !!status?.paths.sessionsExists, detail: t("workspace.dataCodex", { count: threads.data?.data.items.length ?? 0 }) },
    { key: "claudeCode", name: "Claude Code", mark: <ClientGlyph client="claudeCode" className="h-4 w-4" />, path: status?.paths.claudeHome ?? "",
      ok: !!status?.paths.claudeProjectsExists, detail: t(status?.paths.claudeProjectsExists ? "workspace.dataClaudeSessions" : "workspace.dataNoSessions") },
    { key: "claudeDesktop", name: "Claude Desktop", mark: <ClientGlyph client="claudeDesktop" className="h-4 w-4" />,
      path: desktop.data?.paths?.normalRoot ?? "", ok: !!desktop.data?.paths, detail: t(desktop.data?.isManaged ? "workspace.dataManaged" : "workspace.dataUnmanaged") },
    { key: "opencode", name: "OpenCode", mark: <ClientGlyph client="opencode" className="h-4 w-4" />, path: configDir("opencode"),
      ok: !!clientConfigs.data?.find((client) => client.targetId === "opencode")?.configured, detail: t("workspace.dataConfigFile") },
    { key: "pi", name: "Pi", mark: <ClientGlyph client="pi" className="h-4 w-4" />, path: configDir("pi"),
      ok: !!clientConfigs.data?.find((client) => client.targetId === "pi")?.configured, detail: t("workspace.dataConfigFile") },
    { key: "harness", name: "DeepSeek Harness", mark: <ClientGlyph client="harness" className="h-4 w-4" />,
      path: harness?.harness?.home ?? "", ok: !!harness?.configured,
      detail: harness?.configured && harness.request?.harnessProfile
        ? t("workspace.dataHarnessProfile", { profile: harness.request.harnessProfile }) : t("workspace.dataConfigFile") },
    { key: "cursor", name: "Cursor", mark: <ClientGlyph client="cursor" className="h-4 w-4" />,
      path: cursor.data?.settingsPath ? parentDir(cursor.data.settingsPath) : "",
      ok: !!cursor.data && (cursor.data.ideApplied || cursor.data.cliApplied),
      detail: t(cursor.data && (cursor.data.ideApplied || cursor.data.cliApplied) ? "workspace.dataCursorInjected" : "workspace.dataCursorIdle") },
    { key: "agentharbor", name: "AgentHarbor", mark: <Waypoints className="h-4 w-4 text-primary" />, path: status?.paths.appDataDir ?? "",
      ok: !!status?.paths.appDataDir, detail: t("workspace.dataHarbor") },
  ].map((location) => ({ ...location, path: location.path || locatedDir(location.key) }));

  const local = usage.data?.local;
  const daily = (local?.daily ?? []).slice(-14).map((day) => day.totals.totalTokens);
  const activeAccount = accounts.data?.items.find((account) => account.isActive) ?? null;
  // Codex and Claude Code sessions together, newest first; each keeps the client it came from.
  const threadItems = [
    ...(threads.data?.data.items ?? []).map((thread) => ({ ...thread, client: "codex" as const })),
    ...(claudeThreads.data?.data.items ?? []).map((thread) => ({ ...thread, client: "claudeCode" as const })),
  ];
  const startOfDay = new Date().setHours(0, 0, 0, 0) / 1000;
  const todayThreads = threadItems.filter((thread) => thread.updatedAt >= startOfDay).length;
  const recent = threadItems.filter((thread) => recentSource === "all" || thread.client === recentSource)
    .sort((a, b) => b.updatedAt - a.updatedAt).slice(0, 6);
  const recentLoading = threads.isLoading || claudeThreads.isLoading;
  const openThreads = (client: "codex" | "claudeCode") => {
    // The threads page opens on the source this session belongs to.
    try { localStorage.setItem(storageKeys.threadsSource, client); } catch { /* storage unavailable */ }
    onNavigate("threads");
  };
  const connectionLabel = newapi.isError ? t("workspace.checkFailed") : !newapi.data ? t("workspace.checking")
    : proxyRunning ? t("workspace.proxyOnline") : t("workspace.proxyOffline");

  return (
    <div className="overview-shell">
      <section className="dash-hero route-surface" aria-labelledby="overview-title">
        <div className="dash-hero-copy relative z-[1]">
          <div className="overview-eyebrow"><span aria-hidden />{t("workspace.eyebrow")}</div>
          <h1 id="overview-title" className="poster-title">{greeting}</h1>
          <p className="poster-kicker mt-2 text-[11px] text-muted-foreground">{dateLine}</p>
          <p className="dash-hero-lead">{t("workspace.heroLead")}</p>
          <div className="dash-hero-chips" aria-live="polite">
            <span className={cn("dash-chip", proxyRunning && "is-on")}>
              <span aria-hidden className={cn("overview-connection-dot", proxyRunning && "is-online")} />
              {connectionLabel}
            </span>
            <span className="dash-chip">
              {routingEnabled ? t("workspace.routedClients", { count: routedCount }) : t("workspace.directMode")}
            </span>
            <span className="dash-chip">{t("workspace.enabledRelays", { count: enabledProviders })}</span>
          </div>
          <div className="dash-hero-actions">
            <Button className="h-9 rounded-xl px-4" onClick={() => onNavigate("newapi")}>
              {t("workspace.manageConnection")} <ArrowRight aria-hidden className="h-4 w-4" />
            </Button>
            <Button variant="outline" className="h-9 rounded-xl bg-background/80" onClick={refresh} disabled={busy}>
              <RotateCw aria-hidden className={cn("h-4 w-4", busy && "animate-spin")} />
              {t("common.refresh")}
            </Button>
          </div>
          <div className="overview-proxy-row">
            <span>{t("settings.apiProxy")}</span>
            <strong title={apiProxy?.url ?? apiProxyMode}>{apiProxyMode}{apiProxy?.mode === "manual" && apiProxy.url ? ` · ${apiProxy.url}` : ""}</strong>
            <button type="button" onClick={onOpenProxySettings}>
              {t("workspace.proxySettings")} <ArrowRight aria-hidden className="h-3.5 w-3.5" />
            </button>
          </div>
        </div>
        <div className="relative z-[1]"><OrbitHub links={links} running={proxyRunning} onOpen={() => onNavigate("newapi")} /></div>
      </section>

      {errors.length > 0 && (
        <div role="alert" className="overview-error">
          <strong>{t("baseline.readFailed")}</strong>
          <details>
            <summary>{t("workspace.errorDetails")}</summary>
            <div className="overview-error-details">
              {errors.map((error, index) => <span key={index}>{errorMessage(error)}</span>)}
            </div>
          </details>
        </div>
      )}

      <section className="dash-bento" aria-label={t("workspace.pulse")}>
        <Tile icon={<Zap className="h-3.5 w-3.5" />} label={t("workspace.tokensToday")} onClick={() => onNavigate("usage")}
          caption={local ? t("workspace.tokensWeek", { value: formatTokenCount(local.week.totalTokens), turns: local.today.turnCount }) : t(usage.isError ? "workspace.checkFailed" : "workspace.checking")}>
          <span className="dash-value"><CountUp value={local?.today.totalTokens ?? 0} format={formatTokenCount} /></span>
          <Sparkline values={daily} />
        </Tile>
        <Tile icon={<UsersRound className="h-3.5 w-3.5" />} label={t("workspace.activeAccount")} onClick={() => onNavigate("accounts")}
          caption={activeAccount?.primaryWindow
            ? t("workspace.quotaUsed", { value: Math.round(activeAccount.primaryWindow.usedPercent) })
            : t(accounts.isError ? "workspace.checkFailed" : activeAccount ? "workspace.quotaUnknown" : "workspace.noAccount")}>
          <span className="flex min-w-0 items-center gap-3">
            {activeAccount && <QuotaRing account={activeAccount} />}
            <span className="min-w-0">
              <span className="block truncate text-[15px] font-semibold">{activeAccount ? maskEmail(activeAccount.email) || activeAccount.displayName : "—"}</span>
              {activeAccount && <span className="mt-1 inline-flex rounded-full bg-primary/10 px-2 py-0.5 text-[11px] font-semibold uppercase text-primary">{activeAccount.plan}</span>}
            </span>
          </span>
        </Tile>
        <Tile icon={<RadioTower className="h-3.5 w-3.5" />} label={t("workspace.relaysTile")} onClick={() => onNavigate("relays")}
          caption={routingEnabled ? t("workspace.smartRouting") : t("workspace.directMode")}>
          <span className="dash-value"><CountUp value={enabledProviders} /><small>/ {providers.length}</small></span>
          <span className="dash-meter" aria-hidden><span style={{ width: `${providers.length ? enabledProviders / providers.length * 100 : 0}%` }} /></span>
        </Tile>
        <Tile icon={<MessagesSquare className="h-3.5 w-3.5" />} label={t("nav.threads")} onClick={() => onNavigate("threads")}
          caption={t("workspace.threadsToday", { count: todayThreads })}>
          <span className="dash-value"><CountUp value={threadItems.length} /></span>
          <span className="dash-meter is-violet" aria-hidden><span style={{ width: `${threadItems.length ? Math.min(100, todayThreads / threadItems.length * 400) : 0}%` }} /></span>
        </Tile>
      </section>

      <div className="dash-split">
        <section className="dash-panel" aria-labelledby="overview-recent-title">
          <div className="dash-panel-head">
            <h2 id="overview-recent-title"><MessagesSquare aria-hidden />{t("workspace.recentThreads")}<span aria-hidden className="poster-kicker text-[9.5px] text-muted-foreground">Recent</span></h2>
            <button type="button" onClick={() => onNavigate("threads")}>{t("workspace.viewAll")}<ArrowRight aria-hidden className="h-3.5 w-3.5" /></button>
          </div>
          <div className="dash-recent-filter" role="radiogroup" aria-label={t("workspace.recentSource")}>
            {(["all", "codex", "claudeCode"] as const).map((source) => {
              const count = source === "all" ? threadItems.length : threadItems.filter((thread) => thread.client === source).length;
              return (
                <button key={source} type="button" role="radio" aria-checked={recentSource === source} data-recent-source={source}
                  onClick={() => setRecentSource(source)}>
                  {source !== "all" && <ClientGlyph client={source} className="h-3.5 w-3.5" />}
                  {source === "all" ? t("workspace.recentAll") : t(`newapi.topology.client.${source}`)}
                  <span className="tabular-nums">{count}</span>
                </button>
              );
            })}
          </div>
          {recent.length === 0 ? (
            <p className="dash-empty">{t(recentLoading ? "workspace.checking" : "workspace.noThreads")}</p>
          ) : (
            <ul className="dash-recent">
              {recent.map((thread, index) => (
                <li key={`${thread.client}:${thread.key}`} data-recent-client={thread.client} style={{ animationDelay: `${index * 45}ms` }}>
                  <button type="button" onClick={() => openThreads(thread.client)} title={`${t(`newapi.topology.client.${thread.client}`)} · ${thread.title}`}>
                    <span className="dash-recent-mark"><ClientGlyph client={thread.client} className="h-4 w-4" /></span>
                    <span className="min-w-0 flex-1">
                      <span className="block truncate text-[13px] font-medium text-foreground">{thread.title || t("threads.untitled")}</span>
                      <span className="block truncate text-[11px] text-muted-foreground">{thread.folderName || t("threads.directChats")} · {thread.model || t("threads.unknownModel")}</span>
                    </span>
                    <span className="shrink-0 text-[11px] text-muted-foreground">{formatRelative(thread.updatedAt)}</span>
                  </button>
                </li>
              ))}
            </ul>
          )}
        </section>

        <section className="dash-panel" aria-labelledby="overview-capabilities-title">
          <div className="dash-panel-head">
            <h2 id="overview-capabilities-title"><Sparkles aria-hidden />{t("workspace.capabilities")}<span aria-hidden className="poster-kicker text-[9.5px] text-muted-foreground">Toolkit</span></h2>
          </div>
          <div className="overview-capabilities">
            <button type="button" data-spotlight className="overview-capability" onClick={() => onNavigate("newapi")}>
              <span className="overview-capability-heading"><Waypoints aria-hidden />{t("newapi.title")}<ArrowRight aria-hidden className="overview-capability-arrow" /></span>
              <span className={cn("overview-capability-value", !newapi.data && "is-empty")}><strong>{newapi.isError || !newapi.data ? "—" : newapi.data.providers.length}</strong><small>{t("workspace.providersUnit")}</small></span>
              <span className="overview-capability-meta">{!newapi.data ? t(newapi.isError ? "workspace.checkFailed" : "workspace.checking") : routingEnabled ? t("workspace.smartRouting") : t("workspace.directMode")}</span>
            </button>
            <button type="button" data-spotlight className="overview-capability" onClick={() => onNavigate("mcp")}>
              <span className="overview-capability-heading"><Server aria-hidden />{t("nav.mcp")}<ArrowRight aria-hidden className="overview-capability-arrow" /></span>
              <span className={cn("overview-capability-value", !mcp.data && "is-empty")}><strong>{mcp.isError || !mcp.data ? "—" : mcpServers.length}</strong><small>{t("workspace.mcpUnit")}</small></span>
              <span className="overview-capability-meta">{mcp.data ? t("workspace.mcpSummary", { count: mcpEnabledCount }) : t(mcp.isError ? "workspace.checkFailed" : "workspace.checking")}</span>
            </button>
            <button type="button" data-spotlight className="overview-capability" onClick={() => onNavigate("skills")}>
              <span className="overview-capability-heading"><Sparkles aria-hidden />{t("nav.skills")}<ArrowRight aria-hidden className="overview-capability-arrow" /></span>
              <span className={cn("overview-capability-value", !skillsAvailable && "is-empty")}><strong>{skillsAvailable ? skillsCount : "—"}</strong><small>{t("workspace.skillsUnit")}</small></span>
              <span className="overview-capability-meta">{backupsAvailable ? t("workspace.skillsSummary", { count: backupsCount }) : t(skillBackups.isError || claudeBackups.isError ? "workspace.checkFailed" : "workspace.checking")}</span>
            </button>
            <button type="button" data-spotlight className="overview-capability" onClick={() => onNavigate("customInstructions")}>
              <span className="overview-capability-heading"><FileCode2 aria-hidden />{t("nav.customInstructions")}<ArrowRight aria-hidden className="overview-capability-arrow" /></span>
              <span className={cn("overview-capability-value", !instructions.data && "is-empty")}><strong>{instructions.isError || !instructions.data ? "—" : instructionManaged ? t("workspace.applied") : t("workspace.notApplied")}</strong></span>
              <span className="overview-capability-meta">{lastTemplateTitle ?? t("workspace.instructionsSummary")}</span>
            </button>
          </div>
        </section>
      </div>

      <section aria-labelledby="overview-modules-title">
        <h2 id="overview-modules-title" className="sr-only">{t("workspace.quickModules")}</h2>
        <div className="dash-modules">
          {([
            ["accounts", UsersRound, t("nav.accounts"), t("workspace.accountsModule"), "is-blue"],
            ["imageWorkbench", ImageIcon, t("image.title"), t("workspace.imageModule"), "is-amber"],
            ["threads", MessagesSquare, t("nav.threads"), t("workspace.threadsModule"), "is-violet"],
            ["maintenance", Wrench, t("nav.maintenance"), t("workspace.maintenanceModule"), "is-green"],
            ["settings", Settings, t("nav.settings"), t("workspace.settingsModule"), "is-slate"],
          ] as const).map(([route, Icon, title, description, tone]) => (
            <button key={route} type="button" data-spotlight className={cn("overview-quick-card group", tone)} onClick={() => onNavigate(route)}>
              <span className="overview-quick-icon"><Icon className="h-5 w-5" /></span>
              <span className="min-w-0 text-left">
                <span className="flex items-center gap-1 text-[13px] font-semibold text-foreground transition-colors group-hover:text-primary">
                  {title}
                  <ArrowRight className="h-3 w-3 opacity-0 transition-all group-hover:translate-x-0.5 group-hover:opacity-100" />
                </span>
                <span className="block truncate text-[11px] text-muted-foreground">{description}</span>
              </span>
            </button>
          ))}
        </div>
      </section>

      <section className="overview-system" aria-labelledby="overview-system-title" data-open={dataMap.open || undefined}>
        <div className="overview-system-header">
          <div className="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-1">
            <h2 id="overview-system-title" className="overview-system-title">
              <button type="button" className="overview-system-toggle" aria-expanded={dataMap.open}
                aria-controls="overview-data-map" onClick={dataMap.toggle}>
                <ShieldCheck aria-hidden />{t("baseline.localData")}
                <span aria-hidden className="poster-kicker text-[9.5px] text-muted-foreground">Data map</span>
                <ChevronDown aria-hidden className="overview-system-chevron" />
              </button>
            </h2>
            {!dataMap.open && (
              <span className="overview-system-summary">
                {t("workspace.dataMapSummary", {
                  found: dataLocations.filter((location) => location.path).length,
                  total: dataLocations.length,
                })}
              </span>
            )}
          </div>
          <div className="overview-system-tools">
            {systemInfo.data && <span className="overview-system-platform"><Cpu aria-hidden />{systemInfo.data.os} ({systemInfo.data.arch})</span>}
            <Button variant="outline" size="sm" onClick={() => void handleRestartCodex()} className="overview-restart">
              <RotateCw aria-hidden className="h-3.5 w-3.5" />{t("workspace.restartCodex")}
            </Button>
          </div>
        </div>
        <div id="overview-data-map" ref={dataMapRef} className="overview-fold">
        <div className="overview-fold-inner">
        <ul className="data-map" aria-label={t("workspace.dataLocations")}>
          {dataLocations.map((location) => (
            <li key={location.key} className="data-row" data-location={location.key}>
              <span className="data-row-mark">{location.mark}</span>
              <span className="min-w-0 flex-1">
                <span className="flex items-center gap-2 text-[13px] font-semibold">
                  {location.name}
                  <span aria-hidden className={cn("h-1.5 w-1.5 rounded-full", location.path ? location.ok ? "bg-emerald-500" : "bg-amber-500" : "bg-muted-foreground/40")} />
                  <span className="truncate text-[11px] font-normal text-muted-foreground">{location.detail}</span>
                </span>
                <span className="block truncate font-mono text-[11px] text-muted-foreground" title={location.path || undefined}>
                  {location.path || t("workspace.notDetected")}
                </span>
              </span>
              <span className="flex shrink-0 items-center gap-1">
                <Button variant="ghost" size="icon-sm" onClick={() => void handleCopyPath(location.path, location.key)} disabled={!location.path}
                  title={t("workspace.copyPath")} aria-label={`${t("workspace.copyPath")} · ${location.name}`}>
                  {copiedPath === location.key ? <Check className="h-4 w-4" /> : <Copy className="h-4 w-4" />}
                </Button>
                <Button variant="ghost" size="icon-sm" onClick={() => void handleOpenFolder(location.path)} disabled={!location.path}
                  title={t("baseline.openFolder")} aria-label={`${t("baseline.openFolder")} · ${location.name}`}>
                  <FolderOpen className="h-4 w-4" />
                </Button>
              </span>
              <KeyFiles group={filesOf(location.key)} name={location.name} />
            </li>
          ))}
        </ul>
        </div>
        </div>
      </section>
    </div>
  );
}
