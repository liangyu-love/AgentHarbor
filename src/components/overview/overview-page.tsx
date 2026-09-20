import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import {
  ArrowRight,
  FileCode2,
  Server,
  Sparkles,
  RotateCw,
  FolderOpen,
  Wallet,
  Zap,
  CheckCircle2,
  XCircle,
  Copy,
  Check,
  Terminal,
  ShieldCheck,
  Plus,
  Upload,
  Cpu,
} from "lucide-react";
import { api } from "@/lib/api";
import { AuroraCard } from "@/components/ui/aurora-card";
import { BentoCard } from "@/components/ui/bento-card";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { toast } from "@/hooks/use-toast";
import { cn } from "@/lib/utils";
import type { Route } from "@/types/navigation";

export function OverviewPage({ onNavigate }: { onNavigate: (route: Route) => void }) {
  const { t } = useTranslation();
  const [copiedHome, setCopiedHome] = useState(false);

  const snapshot = useQuery({ queryKey: ["runtime-state", "display"], queryFn: () => api.loadSnapshot(true) });
  const newapi = useQuery({ queryKey: ["newapi-state"], queryFn: api.loadNewApiState });
  const mcp = useQuery({ queryKey: ["mcp-servers"], queryFn: api.loadMcpServers });
  const skills = useQuery({ queryKey: ["installed-skills"], queryFn: api.loadInstalledSkills });
  const skillBackups = useQuery({ queryKey: ["skill-backups"], queryFn: api.loadSkillBackups });
  const instructions = useQuery({ queryKey: ["custom-instructions", "state"], queryFn: api.loadCustomInstructionState });
  const systemInfo = useQuery({ queryKey: ["system-info"], queryFn: api.getSystemInfo, staleTime: Infinity });

  const status = snapshot.data?.data.status;
  const errors = [snapshot.error, newapi.error, mcp.error, skills.error, instructions.error].filter(Boolean);

  const refresh = () => {
    void Promise.all([
      snapshot.refetch(),
      newapi.refetch(),
      mcp.refetch(),
      skills.refetch(),
      skillBackups.refetch(),
      instructions.refetch(),
    ]);
  };

  const busy = [snapshot, newapi, mcp, skills, instructions].some((query) => query.isFetching);

  const getGreeting = () => {
    const hour = new Date().getHours();
    if (hour >= 5 && hour < 12) return "早上好";
    if (hour >= 12 && hour < 18) return "下午好";
    return "晚上好";
  };

  const handleCopyCodexHome = () => {
    if (!status?.paths.codexHome) return;
    navigator.clipboard.writeText(status.paths.codexHome);
    setCopiedHome(true);
    setTimeout(() => setCopiedHome(false), 2000);
    toast({ title: "已复制 .codex 路径", variant: "default" });
  };

  const handleOpenFolder = async (path: string, label = "目录") => {
    if (!path) return;
    try {
      await api.openPath(path);
      toast({ title: `${label} 已打开`, variant: "default" });
    } catch (err) {
      toast({ title: t("common.error"), description: String(err), variant: "destructive" });
    }
  };

  const handleRestartCodex = async () => {
    try {
      await api.restartCodex();
      toast({ title: "Codex 重启指令已发送", variant: "default" });
    } catch (err) {
      toast({ title: t("common.error"), description: String(err), variant: "destructive" });
    }
  };

  // Data aggregations
  const proxyRunning = newapi.data?.proxyRunning ?? false;
  const routingEnabled = newapi.data?.router.enabled ?? false;
  const activeProvider = newapi.data?.providers.find((p) => p.id === newapi.data?.activeProviderId);

  const mcpServers = mcp.data?.data.items ?? [];
  const mcpEnabledCount = mcpServers.filter((s) => s.enabled).length;
  const mcpStdioCount = mcpServers.filter((s) => s.transport === "stdio").length;

  const skillsCount = skills.data?.data.total ?? 0;
  const backupsCount = skillBackups.data?.data.total ?? 0;

  const instructionManaged = instructions.data?.data.current.managedBlockPresent ?? false;
  const lastTemplateTitle = instructions.data?.data.current.lastTemplateTitle;

  return (
    <div className="space-y-6 pb-4">
      {/* 1. Mission Control Header & Greeting */}
      <div className="flex flex-wrap items-center justify-between gap-4">
        <div>
          <div className="flex items-center gap-2.5">
            <h1 className="text-2xl font-bold tracking-tight text-foreground">
              {getGreeting()}，开发者 👋
            </h1>
            <Badge
              variant="outline"
              className={cn(
                "gap-1.5 px-2.5 py-0.5 text-[11px] font-medium",
                proxyRunning
                  ? "border-emerald-500/20 bg-emerald-500/10 text-emerald-600 dark:text-emerald-400"
                  : "border-border bg-muted/50 text-muted-foreground"
              )}
            >
              <span
                className={cn(
                  "inline-block h-1.5 w-1.5 rounded-full",
                  proxyRunning ? "bg-emerald-500 animate-pulse" : "bg-zinc-400"
                )}
              />
              {proxyRunning ? "本地中转代理在线" : "本地代理已就绪"}
            </Badge>
          </div>
          <p className="mt-1 text-sm text-muted-foreground">
            CodexHarbor 本地开发环境健康稳定，随时提供模型注入、工具链扩展与全局指令支持。
          </p>
        </div>

        <div className="flex items-center gap-2">
          <Button
            variant="outline"
            size="sm"
            onClick={refresh}
            disabled={busy}
            className="h-9 gap-1.5"
          >
            <RotateCw className={cn("h-3.5 w-3.5", busy && "animate-spin")} />
            {t("common.refresh")}
          </Button>
        </div>
      </div>

      {errors.length > 0 && (
        <div role="alert" className="rounded-xl border border-destructive/30 bg-destructive/5 p-4 text-sm text-destructive">
          <p className="font-medium">{t("baseline.readFailed")}</p>
          {errors.map((error, index) => (
            <p key={index} className="mt-1 break-words text-xs text-muted-foreground">
              {String(error)}
            </p>
          ))}
        </div>
      )}

      {/* 2. Four Core Capacity Showcase Pillars */}
      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
        {/* Pillar 1: New API / Relay */}
        <AuroraCard color="blue" className="transition-all duration-200 hover:-translate-y-0.5 hover:shadow-md">
          <div className="flex h-full flex-col justify-between p-4">
            <div>
              <div className="flex items-center justify-between text-xs font-semibold uppercase tracking-wider text-muted-foreground">
                <span className="flex items-center gap-1.5 text-blue-500">
                  <Wallet className="h-4 w-4" />
                  {t("newapi.title")}
                </span>
                <span className="font-mono text-[11px] text-muted-foreground/80">
                  {routingEnabled ? "智能路由" : "直连模式"}
                </span>
              </div>
              <div className="my-3 flex items-baseline gap-1.5">
                <span className="text-3xl font-bold tracking-tight tabular-nums text-foreground">
                  {newapi.isError ? "—" : !newapi.data ? "…" : newapi.data.providers.length}
                </span>
                <span className="text-xs text-muted-foreground">个中转站点</span>
              </div>
              <p className="text-xs leading-relaxed text-muted-foreground line-clamp-2">
                {routingEnabled && activeProvider
                  ? `当前生效路由：${activeProvider.name}`
                  : "支持智能分发 OpenAI 兼容 API、官方中转与余额自动查询。"}
              </p>
            </div>
            <Button
              variant="outline"
              size="sm"
              className="mt-4 w-full justify-between text-xs h-8"
              onClick={() => onNavigate("newapi")}
            >
              管理中转模型
              <ArrowRight className="h-3.5 w-3.5" />
            </Button>
          </div>
        </AuroraCard>

        {/* Pillar 2: MCP Console */}
        <AuroraCard color="purple" className="transition-all duration-200 hover:-translate-y-0.5 hover:shadow-md">
          <div className="flex h-full flex-col justify-between p-4">
            <div>
              <div className="flex items-center justify-between text-xs font-semibold uppercase tracking-wider text-muted-foreground">
                <span className="flex items-center gap-1.5 text-purple-500">
                  <Server className="h-4 w-4" />
                  {t("nav.mcp")}
                </span>
                <span className="font-mono text-[11px] text-muted-foreground/80">
                  {mcpEnabledCount} 运行中
                </span>
              </div>
              <div className="my-3 flex items-baseline gap-1.5">
                <span className="text-3xl font-bold tracking-tight tabular-nums text-foreground">
                  {mcp.isError ? "—" : !mcp.data ? "…" : mcpServers.length}
                </span>
                <span className="text-xs text-muted-foreground">个 MCP 服务</span>
              </div>
              <p className="text-xs leading-relaxed text-muted-foreground line-clamp-2">
                {mcpServers.length > 0
                  ? `包含 ${mcpStdioCount} 个 STDIO 本地进程与 ${mcpServers.length - mcpStdioCount} 个远程端点。`
                  : t("baseline.mcpDesc")}
              </p>
            </div>
            <Button
              variant="outline"
              size="sm"
              className="mt-4 w-full justify-between text-xs h-8"
              onClick={() => onNavigate("mcp")}
            >
              配置 MCP 工具
              <ArrowRight className="h-3.5 w-3.5" />
            </Button>
          </div>
        </AuroraCard>

        {/* Pillar 3: Skills Gallery */}
        <AuroraCard color="green" className="transition-all duration-200 hover:-translate-y-0.5 hover:shadow-md">
          <div className="flex h-full flex-col justify-between p-4">
            <div>
              <div className="flex items-center justify-between text-xs font-semibold uppercase tracking-wider text-muted-foreground">
                <span className="flex items-center gap-1.5 text-emerald-500">
                  <Sparkles className="h-4 w-4" />
                  {t("nav.skills")}
                </span>
                <span className="font-mono text-[11px] text-muted-foreground/80">
                  {backupsCount} 备份归档
                </span>
              </div>
              <div className="my-3 flex items-baseline gap-1.5">
                <span className="text-3xl font-bold tracking-tight tabular-nums text-foreground">
                  {skills.isError ? "—" : !skills.data ? "…" : skillsCount}
                </span>
                <span className="text-xs text-muted-foreground">个本地技能</span>
              </div>
              <p className="text-xs leading-relaxed text-muted-foreground line-clamp-2">
                {skillsCount > 0
                  ? "已加载本地 SKILL.md 技能包，支持一键在 Explorer 中调试。"
                  : t("baseline.skillsDesc")}
              </p>
            </div>
            <Button
              variant="outline"
              size="sm"
              className="mt-4 w-full justify-between text-xs h-8"
              onClick={() => onNavigate("skills")}
            >
              浏览技能货架
              <ArrowRight className="h-3.5 w-3.5" />
            </Button>
          </div>
        </AuroraCard>

        {/* Pillar 4: Custom Instructions */}
        <AuroraCard color="red" className="transition-all duration-200 hover:-translate-y-0.5 hover:shadow-md">
          <div className="flex h-full flex-col justify-between p-4">
            <div>
              <div className="flex items-center justify-between text-xs font-semibold uppercase tracking-wider text-muted-foreground">
                <span className="flex items-center gap-1.5 text-rose-500">
                  <FileCode2 className="h-4 w-4" />
                  {t("nav.customInstructions")}
                </span>
                <span className="font-mono text-[11px] text-muted-foreground/80">
                  {instructionManaged ? "受控托管" : "独立模式"}
                </span>
              </div>
              <div className="my-3 flex items-baseline gap-1.5">
                <span className="text-2xl font-bold tracking-tight text-foreground">
                  {instructions.isError ? "—" : !instructions.data ? "…" : instructionManaged ? "已应用" : "未生效"}
                </span>
              </div>
              <p className="text-xs leading-relaxed text-muted-foreground line-clamp-2">
                {lastTemplateTitle
                  ? `当前应用模板：「${lastTemplateTitle}」，安全保护已开启。`
                  : t("baseline.instructionsDesc")}
              </p>
            </div>
            <Button
              variant="outline"
              size="sm"
              className="mt-4 w-full justify-between text-xs h-8"
              onClick={() => onNavigate("customInstructions")}
            >
              编写自定义指令
              <ArrowRight className="h-3.5 w-3.5" />
            </Button>
          </div>
        </AuroraCard>
      </div>

      {/* 3. Quick Action Dock (高频快捷操作坞) */}
      <div className="space-y-3">
        <h2 className="text-sm font-semibold tracking-tight text-foreground flex items-center gap-2">
          <Zap className="h-4 w-4 text-amber-500" />
          快捷开发者操作坞
        </h2>
        <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-6">
          <button
            type="button"
            onClick={() => onNavigate("newapi")}
            className="flex flex-col items-center justify-center p-3 rounded-xl border border-border/80 bg-card hover:bg-muted/60 hover:border-primary/40 transition-all group text-center cursor-pointer"
          >
            <div className="h-9 w-9 rounded-lg bg-blue-500/10 text-blue-500 flex items-center justify-center ring-1 ring-inset ring-blue-500/20 group-hover:scale-110 transition-transform">
              <Plus className="h-4 w-4" />
            </div>
            <span className="mt-2 text-xs font-semibold text-foreground">添加中转</span>
            <span className="text-[10px] text-muted-foreground">注入模型站点</span>
          </button>

          <button
            type="button"
            onClick={() => onNavigate("mcp")}
            className="flex flex-col items-center justify-center p-3 rounded-xl border border-border/80 bg-card hover:bg-muted/60 hover:border-primary/40 transition-all group text-center cursor-pointer"
          >
            <div className="h-9 w-9 rounded-lg bg-purple-500/10 text-purple-500 flex items-center justify-center ring-1 ring-inset ring-purple-500/20 group-hover:scale-110 transition-transform">
              <Terminal className="h-4 w-4" />
            </div>
            <span className="mt-2 text-xs font-semibold text-foreground">添加 MCP</span>
            <span className="text-[10px] text-muted-foreground">本地或远程工具</span>
          </button>

          <button
            type="button"
            onClick={() => onNavigate("skills")}
            className="flex flex-col items-center justify-center p-3 rounded-xl border border-border/80 bg-card hover:bg-muted/60 hover:border-primary/40 transition-all group text-center cursor-pointer"
          >
            <div className="h-9 w-9 rounded-lg bg-emerald-500/10 text-emerald-500 flex items-center justify-center ring-1 ring-inset ring-emerald-500/20 group-hover:scale-110 transition-transform">
              <Upload className="h-4 w-4" />
            </div>
            <span className="mt-2 text-xs font-semibold text-foreground">导入 Skill</span>
            <span className="text-[10px] text-muted-foreground">加载本地扩展包</span>
          </button>

          <button
            type="button"
            onClick={() => onNavigate("customInstructions")}
            className="flex flex-col items-center justify-center p-3 rounded-xl border border-border/80 bg-card hover:bg-muted/60 hover:border-primary/40 transition-all group text-center cursor-pointer"
          >
            <div className="h-9 w-9 rounded-lg bg-rose-500/10 text-rose-500 flex items-center justify-center ring-1 ring-inset ring-rose-500/20 group-hover:scale-110 transition-transform">
              <FileCode2 className="h-4 w-4" />
            </div>
            <span className="mt-2 text-xs font-semibold text-foreground">指令中心</span>
            <span className="text-[10px] text-muted-foreground">切换提示词模板</span>
          </button>

          <button
            type="button"
            onClick={handleRestartCodex}
            className="flex flex-col items-center justify-center p-3 rounded-xl border border-border/80 bg-card hover:bg-muted/60 hover:border-primary/40 transition-all group text-center cursor-pointer"
          >
            <div className="h-9 w-9 rounded-lg bg-amber-500/10 text-amber-500 flex items-center justify-center ring-1 ring-inset ring-amber-500/20 group-hover:scale-110 transition-transform">
              <RotateCw className="h-4 w-4" />
            </div>
            <span className="mt-2 text-xs font-semibold text-foreground">重启 Codex</span>
            <span className="text-[10px] text-muted-foreground">重载全部配置</span>
          </button>

          <button
            type="button"
            onClick={() => status && handleOpenFolder(status.paths.codexHome, ".codex 根目录")}
            className="flex flex-col items-center justify-center p-3 rounded-xl border border-border/80 bg-card hover:bg-muted/60 hover:border-primary/40 transition-all group text-center cursor-pointer"
          >
            <div className="h-9 w-9 rounded-lg bg-blue-500/10 text-blue-500 flex items-center justify-center ring-1 ring-inset ring-blue-500/20 group-hover:scale-110 transition-transform">
              <FolderOpen className="h-4 w-4" />
            </div>
            <span className="mt-2 text-xs font-semibold text-foreground">打开 .codex</span>
            <span className="text-[10px] text-muted-foreground">资源管理器中查看</span>
          </button>
        </div>
      </div>

      {/* 4. Codex Local Environment Inspection Matrix */}
      <BentoCard glowColor="bg-primary/10">
        <div className="flex flex-wrap items-center justify-between gap-3 mb-4">
          <div>
            <h2 className="font-semibold text-base text-foreground flex items-center gap-2">
              <ShieldCheck className="h-4 w-4 text-primary" />
              {t("baseline.localData")}
            </h2>
            <p className="mt-0.5 text-xs text-muted-foreground">
              Codex 核心数据链与系统运行时映射检测
            </p>
          </div>

          {systemInfo.data && (
            <div className="flex items-center gap-2 text-xs font-mono text-muted-foreground bg-muted/50 px-2.5 py-1 rounded-lg border border-border/60">
              <Cpu className="h-3.5 w-3.5 text-primary" />
              <span>{systemInfo.data.os} ({systemInfo.data.arch})</span>
            </div>
          )}
        </div>

        {/* Home Directory Banner */}
        <div className="flex items-center justify-between gap-2 p-2.5 rounded-xl bg-muted/40 border border-border/70 font-mono text-xs text-foreground/90">
          <div className="flex items-center gap-2 min-w-0">
            <FolderOpen className="h-4 w-4 text-muted-foreground shrink-0" />
            <span className="truncate">{status?.paths.codexHome ?? "正在检测 ~/.codex ..."}</span>
          </div>
          <div className="flex items-center gap-1 shrink-0">
            <Button
              variant="ghost"
              size="icon-sm"
              className="h-7 w-7 text-muted-foreground hover:text-foreground"
              onClick={handleCopyCodexHome}
              title="复制路径"
            >
              {copiedHome ? <Check className="h-3.5 w-3.5 text-emerald-500" /> : <Copy className="h-3.5 w-3.5" />}
            </Button>
            <Button
              variant="outline"
              size="sm"
              className="h-7 px-2.5 text-xs gap-1.5"
              disabled={!status}
              onClick={() => status && handleOpenFolder(status.paths.codexHome, ".codex 根目录")}
            >
              <FolderOpen className="h-3.5 w-3.5" />
              {t("baseline.openFolder")}
            </Button>
          </div>
        </div>

        {/* Key Configuration Files Status Grid */}
        <div className="mt-4 grid grid-cols-2 gap-3 sm:grid-cols-4">
          <div className="flex items-center gap-2.5 p-3 rounded-xl border border-border/60 bg-card/60">
            {status?.paths.authExists ? (
              <CheckCircle2 className="h-4 w-4 text-emerald-500 shrink-0" />
            ) : (
              <XCircle className="h-4 w-4 text-muted-foreground/60 shrink-0" />
            )}
            <div className="min-w-0 flex-1">
              <div className="text-xs font-semibold text-foreground truncate">auth.json</div>
              <div className="text-[11px] text-muted-foreground">
                {status?.paths.authExists ? "认证凭据正常" : "未检测到凭据"}
              </div>
            </div>
          </div>

          <div className="flex items-center gap-2.5 p-3 rounded-xl border border-border/60 bg-card/60">
            {status?.paths.registryExists ? (
              <CheckCircle2 className="h-4 w-4 text-emerald-500 shrink-0" />
            ) : (
              <XCircle className="h-4 w-4 text-muted-foreground/60 shrink-0" />
            )}
            <div className="min-w-0 flex-1">
              <div className="text-xs font-semibold text-foreground truncate">registry.json</div>
              <div className="text-[11px] text-muted-foreground">
                {status?.paths.registryExists ? "注册表索引正常" : "无附加注册表"}
              </div>
            </div>
          </div>

          <div className="flex items-center gap-2.5 p-3 rounded-xl border border-border/60 bg-card/60">
            {status?.paths.sessionsExists ? (
              <CheckCircle2 className="h-4 w-4 text-emerald-500 shrink-0" />
            ) : (
              <XCircle className="h-4 w-4 text-muted-foreground/60 shrink-0" />
            )}
            <div className="min-w-0 flex-1">
              <div className="text-xs font-semibold text-foreground truncate">sessions/</div>
              <div className="text-[11px] text-muted-foreground">
                {status?.paths.sessionsExists ? "历史会话归档" : "暂无会话日志"}
              </div>
            </div>
          </div>

          <div className="flex items-center gap-2.5 p-3 rounded-xl border border-border/60 bg-card/60">
            {instructionManaged ? (
              <CheckCircle2 className="h-4 w-4 text-emerald-500 shrink-0" />
            ) : (
              <XCircle className="h-4 w-4 text-muted-foreground/60 shrink-0" />
            )}
            <div className="min-w-0 flex-1">
              <div className="text-xs font-semibold text-foreground truncate">AGENTS.md</div>
              <div className="text-[11px] text-muted-foreground">
                {instructionManaged ? "全局指令已注入" : "默认规则模式"}
              </div>
            </div>
          </div>
        </div>
      </BentoCard>

      <p className="rounded-xl border border-dashed border-border/80 p-3.5 text-xs leading-relaxed text-muted-foreground">
        💡 <strong>小提示</strong>：按 <kbd className="font-mono bg-muted px-1.5 py-0.5 rounded text-[11px] text-foreground">Ctrl K</kbd>（Mac 上为 <kbd className="font-mono bg-muted px-1.5 py-0.5 rounded text-[11px] text-foreground">⌘K</kbd>）可随时呼出全局 Spotlight 命令面板，秒级跳转任意页面或直接运行维护操作。
      </p>
    </div>
  );
}
