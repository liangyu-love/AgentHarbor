import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { ArrowRight, FileCode2, Server, Sparkles, RotateCw, FolderOpen } from "lucide-react";
import { api } from "@/lib/api";
import { BentoCard } from "@/components/ui/bento-card";
import { Button } from "@/components/ui/button";
import { toast } from "@/hooks/use-toast";
import type { Route } from "@/types/navigation";

export function OverviewPage({ onNavigate }: { onNavigate: (route: Route) => void }) {
  const { t } = useTranslation();
  const snapshot = useQuery({ queryKey: ["runtime-state", "display"], queryFn: () => api.loadSnapshot(true) });
  const mcp = useQuery({ queryKey: ["mcp-servers"], queryFn: api.loadMcpServers });
  const skills = useQuery({ queryKey: ["installed-skills"], queryFn: api.loadInstalledSkills });
  const instructions = useQuery({ queryKey: ["custom-instructions", "state"], queryFn: api.loadCustomInstructionState });
  const status = snapshot.data?.data.status;
  const errors = [snapshot.error, mcp.error, skills.error, instructions.error].filter(Boolean);
  const refresh = () => { void Promise.all([snapshot.refetch(), mcp.refetch(), skills.refetch(), instructions.refetch()]); };
  const busy = [snapshot, mcp, skills, instructions].some((query) => query.isFetching);
  const cards = [
    { route: "mcp" as const, icon: Server, label: t("nav.mcp"), value: mcp.isError ? "—" : mcp.data?.data.total ?? "…", description: t("baseline.mcpDesc") },
    { route: "skills" as const, icon: Sparkles, label: t("nav.skills"), value: skills.isError ? "—" : skills.data?.data.total ?? "…", description: t("baseline.skillsDesc") },
    { route: "customInstructions" as const, icon: FileCode2, label: t("nav.customInstructions"), value: instructions.isError ? "—" : !instructions.data ? "…" : t(instructions.data.data.current.managedBlockPresent ? "baseline.applied" : "baseline.notApplied"), description: t("baseline.instructionsDesc") },
  ];
  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between gap-4">
        <div><h1 className="text-xl font-semibold">{t("baseline.title")}</h1><p className="mt-1 text-sm text-muted-foreground">{t("baseline.description")}</p></div>
        <Button variant="outline" size="sm" onClick={refresh} disabled={busy}><RotateCw className={busy ? "h-4 w-4 animate-spin" : "h-4 w-4"} />{t("common.refresh")}</Button>
      </div>
      {errors.length > 0 && <div role="alert" className="rounded-xl border border-destructive/30 bg-destructive/5 p-4 text-sm"><p className="font-medium">{t("baseline.readFailed")}</p>{errors.map((error, index) => <p key={index} className="mt-1 break-words text-muted-foreground">{String(error)}</p>)}</div>}
      <div className="grid gap-4 md:grid-cols-3">
        {cards.map(({ route, icon: Icon, label, value, description }) => (
          <BentoCard key={route}>
            <div className="flex items-center gap-2 text-sm text-muted-foreground"><Icon className="h-4 w-4" />{label}</div>
            <div className="my-4 text-3xl font-semibold tracking-tight">{value}</div>
            <p className="mb-5 flex-1 text-sm text-muted-foreground">{description}</p>
            <Button variant="outline" className="w-full justify-between" onClick={() => onNavigate(route)}>{t("baseline.manage")}<ArrowRight className="h-4 w-4" /></Button>
          </BentoCard>
        ))}
      </div>
      <BentoCard>
        <h2 className="font-semibold">{t("baseline.localData")}</h2>
        <p className="mt-2 break-all font-mono text-xs text-muted-foreground">{status?.paths.codexHome ?? "…"}</p>
        <div className="mt-4 flex flex-wrap gap-x-8 gap-y-2 text-sm">
          <span>auth.json · {t(status?.paths.authExists ? "baseline.present" : "baseline.notFound")}</span>
          <span>{t("baseline.sessions")} · {t(status?.paths.sessionsExists ? "baseline.present" : "baseline.notFound")}</span>
        </div>
        <Button variant="outline" size="sm" className="mt-5 self-start" disabled={!status} onClick={() => {
          if (status) void api.openPath(status.paths.codexHome).catch((error) => toast({ title: t("baseline.readFailed"), description: String(error), variant: "destructive" }));
        }}><FolderOpen className="h-4 w-4" />{t("baseline.openFolder")}</Button>
      </BentoCard>
      <p className="rounded-xl border border-dashed p-4 text-xs leading-relaxed text-muted-foreground">{t("baseline.scope")}</p>
    </div>
  );
}
