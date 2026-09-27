import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import {
  ArrowRight,
  Check,
  CheckCircle2,
  CircleDashed,
  Copy,
  Cpu,
  FileCode2,
  FolderOpen,
  RotateCw,
  Server,
  ShieldCheck,
  Sparkles,
  Wallet,
  XCircle,
} from "lucide-react";
import { api } from "@/lib/api";
import { Button } from "@/components/ui/button";
import { toast } from "@/hooks/use-toast";
import { cn } from "@/lib/utils";
import type { Route } from "@/types/navigation";

function SectionHeading({ index, title }: { index: string; title: string }) {
  return (
    <div className="overview-section-heading">
      <span className="overview-section-index">{index}</span>
      <div>
        <h2>{title}</h2>
      </div>
    </div>
  );
}

function HealthItem({ name, detail, healthy }: { name: string; detail: string; healthy?: boolean }) {
  return (
    <div className="overview-health-item">
      {healthy === undefined
        ? <CircleDashed aria-hidden className="overview-health-muted" />
        : healthy ? <CheckCircle2 aria-hidden className="overview-health-good" /> : <XCircle aria-hidden className="overview-health-muted" />}
      <div className="min-w-0">
        <strong>{name}</strong>
        <span>{detail}</span>
      </div>
    </div>
  );
}

export function OverviewPage({ onNavigate, onOpenProxySettings }: {
  onNavigate: (route: Route) => void;
  onOpenProxySettings: () => void;
}) {
  const { t } = useTranslation();
  const [copiedHome, setCopiedHome] = useState(false);

  const snapshot = useQuery({ queryKey: ["runtime-state", "display"], queryFn: () => api.loadSnapshot(true) });
  const newapi = useQuery({ queryKey: ["newapi-state"], queryFn: api.providers.loadState });
  const mcp = useQuery({ queryKey: ["mcp-servers"], queryFn: api.loadMcpServers });
  const skills = useQuery({ queryKey: ["installed-skills"], queryFn: api.loadInstalledSkills });
  const skillBackups = useQuery({ queryKey: ["skill-backups"], queryFn: api.loadSkillBackups });
  const instructions = useQuery({ queryKey: ["custom-instructions", "state"], queryFn: api.loadCustomInstructionState });
  const systemInfo = useQuery({ queryKey: ["system-info"], queryFn: api.getSystemInfo, staleTime: Infinity });

  const status = snapshot.data?.data.status;
  const errors = [snapshot.error, newapi.error, mcp.error, skills.error, skillBackups.error, instructions.error].filter(Boolean);
  const busy = [snapshot, newapi, mcp, skills, instructions].some((query) => query.isFetching);

  const refresh = () => {
    void Promise.all([
      snapshot.refetch(), newapi.refetch(), mcp.refetch(), skills.refetch(),
      skillBackups.refetch(), instructions.refetch(),
    ]);
  };

  const handleCopyCodexHome = async () => {
    if (!status?.paths.codexHome) return;
    try {
      await navigator.clipboard.writeText(status.paths.codexHome);
      setCopiedHome(true);
      window.setTimeout(() => setCopiedHome(false), 2000);
      toast({ title: t("workspace.pathCopied") });
    } catch (error) {
      toast({ title: t("common.error"), description: String(error), variant: "destructive" });
    }
  };

  const handleOpenFolder = async (path: string) => {
    if (!path) return;
    try {
      await api.openPath(path);
    } catch (error) {
      toast({ title: t("common.error"), description: String(error), variant: "destructive" });
    }
  };

  const handleRestartCodex = async () => {
    try {
      await api.targets.restart("codex");
      toast({ title: t("workspace.restartSent") });
    } catch (error) {
      toast({ title: t("common.error"), description: String(error), variant: "destructive" });
    }
  };

  const hour = new Date().getHours();
  const greeting = hour >= 5 && hour < 12
    ? t("workspace.goodMorning")
    : hour >= 12 && hour < 18 ? t("workspace.goodAfternoon") : t("workspace.goodEvening");

  const proxyRunning = newapi.data?.proxyRunning ?? false;
  const apiProxy = status?.api.proxy;
  const apiProxyMode = !status
    ? t(snapshot.isError ? "workspace.checkFailed" : "workspace.checking")
    : t(apiProxy?.mode === "manual" ? "settings.apiProxyModeManual"
      : apiProxy?.mode === "direct" ? "settings.apiProxyModeDirect" : "settings.apiProxyModeSystem");
  const routingEnabled = newapi.data?.router.enabled ?? false;
  const activeProvider = newapi.data?.providers.find((provider) => provider.id === newapi.data?.activeProviderId);
  const mcpServers = mcp.data?.data.items ?? [];
  const mcpEnabledCount = mcpServers.filter((server) => server.enabled).length;
  const skillsCount = skills.data?.data.total ?? 0;
  const backupsCount = skillBackups.data?.data.total ?? 0;
  const instructionManaged = instructions.data?.data.current.managedBlockPresent ?? false;
  const lastTemplateTitle = instructions.data?.data.current.lastTemplateTitle;

  return (
    <div className="overview-shell">
      <section className="overview-hero" aria-labelledby="overview-title">
        <div className="overview-hero-copy">
          <div className="overview-eyebrow"><span aria-hidden />{t("workspace.eyebrow")}</div>
          <h1 id="overview-title">{t("workspace.homeTitle")}</h1>
          <p className="overview-greeting">{greeting}</p>
          <Button variant="outline" size="sm" onClick={refresh} disabled={busy} className="overview-refresh">
            <RotateCw aria-hidden className={cn("h-4 w-4", busy && "animate-spin")} />
            {t("common.refresh")}
          </Button>
        </div>
        <div className="overview-connection" aria-live="polite">
          <div className="overview-connection-top">
            <span>{t("workspace.connection")}</span>
            <span className={cn("overview-connection-dot", proxyRunning && "is-online")} aria-hidden />
          </div>
          <strong>{newapi.isError ? t("workspace.checkFailed") : !newapi.data ? t("workspace.checking") : proxyRunning ? t("workspace.proxyOnline") : t("workspace.proxyOffline")}</strong>
          <p>{!newapi.data ? t(newapi.isError ? "workspace.checkFailed" : "workspace.checking") : routingEnabled && activeProvider
            ? t("workspace.activeProvider", { name: activeProvider.name })
            : routingEnabled ? t("workspace.routingEnabled") : t("workspace.directMode")}</p>
          <button type="button" className="overview-connection-link" onClick={() => onNavigate("newapi")}>
            {t("workspace.manageConnection")} <ArrowRight aria-hidden className="h-4 w-4" />
          </button>
          <div className="overview-proxy-row">
            <span>{t("settings.apiProxy")}</span>
            <strong title={apiProxy?.url ?? apiProxyMode}>{apiProxyMode}{apiProxy?.mode === "manual" && apiProxy.url ? ` · ${apiProxy.url}` : ""}</strong>
            <button type="button" onClick={onOpenProxySettings}>
              {t("workspace.proxySettings")} <ArrowRight aria-hidden className="h-3.5 w-3.5" />
            </button>
          </div>
        </div>
      </section>

      {errors.length > 0 && (
        <div role="alert" className="overview-error">
          <strong>{t("baseline.readFailed")}</strong>
          <details>
            <summary>{t("workspace.errorDetails")}</summary>
            <div className="overview-error-details">
              {errors.map((error, index) => <span key={index}>{String(error)}</span>)}
            </div>
          </details>
        </div>
      )}

      <section aria-labelledby="overview-capabilities-title">
        <div id="overview-capabilities-title">
          <SectionHeading index="01" title={t("workspace.capabilities")} />
        </div>
        <div className="overview-capabilities">
          <button type="button" className="overview-capability" onClick={() => onNavigate("newapi")}>
            <span className="overview-capability-heading"><Wallet aria-hidden />{t("newapi.title")}<ArrowRight aria-hidden className="overview-capability-arrow" /></span>
            <span className={cn("overview-capability-value", !newapi.data && "is-empty")}><strong>{newapi.isError || !newapi.data ? "—" : newapi.data.providers.length}</strong><small>{t("workspace.providersUnit")}</small></span>
            <span className="overview-capability-meta">{!newapi.data ? t(newapi.isError ? "workspace.checkFailed" : "workspace.checking") : routingEnabled ? t("workspace.smartRouting") : t("workspace.directMode")}</span>
          </button>
          <button type="button" className="overview-capability" onClick={() => onNavigate("mcp")}>
            <span className="overview-capability-heading"><Server aria-hidden />{t("nav.mcp")}<ArrowRight aria-hidden className="overview-capability-arrow" /></span>
            <span className={cn("overview-capability-value", !mcp.data && "is-empty")}><strong>{mcp.isError || !mcp.data ? "—" : mcpServers.length}</strong><small>{t("workspace.mcpUnit")}</small></span>
            <span className="overview-capability-meta">{mcp.data ? t("workspace.mcpSummary", { count: mcpEnabledCount }) : t(mcp.isError ? "workspace.checkFailed" : "workspace.checking")}</span>
          </button>
          <button type="button" className="overview-capability" onClick={() => onNavigate("skills")}>
            <span className="overview-capability-heading"><Sparkles aria-hidden />{t("nav.skills")}<ArrowRight aria-hidden className="overview-capability-arrow" /></span>
            <span className={cn("overview-capability-value", !skills.data && "is-empty")}><strong>{skills.isError || !skills.data ? "—" : skillsCount}</strong><small>{t("workspace.skillsUnit")}</small></span>
            <span className="overview-capability-meta">{skillBackups.data ? t("workspace.skillsSummary", { count: backupsCount }) : t(skillBackups.isError ? "workspace.checkFailed" : "workspace.checking")}</span>
          </button>
          <button type="button" className="overview-capability" onClick={() => onNavigate("customInstructions")}>
            <span className="overview-capability-heading"><FileCode2 aria-hidden />{t("nav.customInstructions")}<ArrowRight aria-hidden className="overview-capability-arrow" /></span>
            <span className={cn("overview-capability-value", !instructions.data && "is-empty")}><strong>{instructions.isError || !instructions.data ? "—" : instructionManaged ? t("workspace.applied") : t("workspace.notApplied")}</strong></span>
            <span className="overview-capability-meta">{lastTemplateTitle ?? t("workspace.instructionsSummary")}</span>
          </button>
        </div>
      </section>

      <section className="overview-system" aria-labelledby="overview-system-title">
        <div className="overview-system-header" id="overview-system-title">
          <div className="overview-system-title"><ShieldCheck aria-hidden /><h2>{t("baseline.localData")}</h2></div>
          <div className="overview-system-tools">
            {systemInfo.data && <span className="overview-system-platform"><Cpu aria-hidden />{systemInfo.data.os} ({systemInfo.data.arch})</span>}
            <Button variant="outline" size="sm" onClick={() => void handleRestartCodex()} className="overview-restart">
              <RotateCw aria-hidden className="h-3.5 w-3.5" />{t("workspace.restartCodex")}
            </Button>
          </div>
        </div>
        <div className="overview-path">
          <FolderOpen aria-hidden />
          <span title={status?.paths.codexHome}>{status?.paths.codexHome ?? t("workspace.checkingPath")}</span>
          <Button variant="ghost" size="icon-sm" onClick={() => void handleCopyCodexHome()} disabled={!status?.paths.codexHome} title={t("workspace.copyPath")} aria-label={t("workspace.copyPath")}>
            {copiedHome ? <Check className="h-4 w-4" /> : <Copy className="h-4 w-4" />}
          </Button>
          <Button variant="outline" size="sm" onClick={() => void handleOpenFolder(status?.paths.codexHome ?? "")} disabled={!status?.paths.codexHome}>{t("baseline.openFolder")}</Button>
        </div>
        <div className="overview-health-grid">
          <HealthItem name="auth.json" healthy={status?.paths.authExists} detail={t(!status ? "workspace.checking" : status.paths.authExists ? "workspace.authPresent" : "workspace.authMissing")} />
          <HealthItem name="registry.json" healthy={status?.paths.registryExists} detail={t(!status ? "workspace.checking" : status.paths.registryExists ? "workspace.registryPresent" : "workspace.registryMissing")} />
          <HealthItem name="sessions/" healthy={status?.paths.sessionsExists} detail={t(!status ? "workspace.checking" : status.paths.sessionsExists ? "workspace.sessionsPresent" : "workspace.sessionsMissing")} />
          <HealthItem name="AGENTS.md" healthy={instructions.data ? instructionManaged : undefined} detail={t(!instructions.data ? "workspace.checking" : instructionManaged ? "workspace.instructionsPresent" : "workspace.instructionsMissing")} />
        </div>
      </section>

    </div>
  );
}
