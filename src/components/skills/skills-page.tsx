import { useState, useMemo } from "react";
import { useTranslation } from "react-i18next";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api";
import { BentoCard } from "@/components/ui/bento-card";
import { AuroraCard } from "@/components/ui/aurora-card";
import { RainbowButton } from "@/registry/magicui/rainbow-button";
import { Button } from "@/components/ui/button";
import { ButtonBusyContent } from "@/components/ui/button-busy-content";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { SegmentedOptions } from "@/components/ui/segmented-options";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { toast } from "@/hooks/use-toast";
import { formatDateTime } from "@/lib/format-time";
import { useBusyAction } from "@/hooks/use-busy-action";
import {
  Sparkles,
  Upload,
  Trash2,
  RotateCcw,
  Archive,
  Copy,
  FolderOpen,
  Folder,
  Clock,
  Search,
  X,
  RotateCw,
} from "lucide-react";
import { queryKeys } from "@/lib/query-keys";
import { errorMessage, formatInvokeError } from "@/lib/invoke-error";

type Tab = "installed" | "backups";

export function SkillsPage() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [tab, setTab] = useState<Tab>("installed");
  const [searchQuery, setSearchQuery] = useState("");
  const [removing, setRemoving] = useState<string | null>(null);
  const [deletingBackup, setDeletingBackup] = useState<string | null>(null);
  const [conflictChoice, setConflictChoice] = useState<{ id: string; keepEnabled: boolean } | null>(null);

  const refreshAction = useBusyAction({ minVisibleMs: 800 });

  const skillsQuery = useQuery({
    queryKey: queryKeys.installedSkills,
    queryFn: () => api.loadInstalledSkills(),
    staleTime: 30_000,
  });

  const backupsQuery = useQuery({
    queryKey: queryKeys.skillBackups,
    queryFn: () => api.loadSkillBackups(),
  });

  const refreshBusy = refreshAction.busy;

  const handleRefresh = async () => {
    await refreshAction.run(async () => {
      await Promise.all([skillsQuery.refetch(), backupsQuery.refetch()]);
    });
  };

  const refreshSkills = () => Promise.all([
    queryClient.invalidateQueries({ queryKey: queryKeys.installedSkills }),
    queryClient.invalidateQueries({ queryKey: queryKeys.skillBackups }),
  ]);
  const importMutation = useMutation({
    mutationFn: async () => {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const path = await open({ directory: true });
      if (typeof path === "string") return api.importSkill(path);
      throw new Error("cancelled");
    },
    onSuccess: () => {
      toast({
        title: t("common.success"),
        variant: "default",
      });
    },
    onError: (err) => {
      if (errorMessage(err).includes("cancelled")) return;
      toast({
        title: t("common.error"),
        description: formatInvokeError(err),
        variant: "destructive",
      });
    },
    onSettled: refreshSkills,
  });

  const showMutationError = (err: unknown) => toast({
    title: t("common.error"), description: formatInvokeError(err), variant: "destructive",
  });
  const toggleMutation = useMutation({
    mutationFn: ({ id, enabled }: { id: string; enabled: boolean }) => api.setSkillEnabled(id, enabled),
    onError: showMutationError,
    onSettled: refreshSkills,
  });

  const conflictMutation = useMutation({
    mutationFn: ({ id, keepEnabled }: { id: string; keepEnabled: boolean }) => api.resolveSkillConflict(id, keepEnabled),
    onError: showMutationError,
    onSettled: refreshSkills,
  });

  const removeMutation = useMutation({
    mutationFn: (id: string) => api.removeSkill(id),
    onSuccess: () => {
      setRemoving(null);
    },
    onError: showMutationError,
    onSettled: refreshSkills,
  });

  const restoreMutation = useMutation({
    mutationFn: (id: string) => api.restoreSkillBackup(id),
    onSuccess: () => {
      toast({
        title: t("common.success"),
        variant: "default",
      });
    },
    onError: showMutationError,
    onSettled: refreshSkills,
  });

  const deleteBackupMutation = useMutation({
    mutationFn: (id: string) => api.deleteSkillBackup(id),
    onSuccess: () => {
      setDeletingBackup(null);
    },
    onError: showMutationError,
    onSettled: refreshSkills,
  });

  const skillsBusy = toggleMutation.isPending || conflictMutation.isPending || importMutation.isPending || removeMutation.isPending || restoreMutation.isPending || deleteBackupMutation.isPending;
  const skills = skillsQuery.data?.data.items ?? [];
  const backups = backupsQuery.data?.data.items ?? [];
  const scanWarnings = (tab === "installed" ? skillsQuery.data?.warnings : backupsQuery.data?.warnings) ?? [];
  const skillsRootPath = skillsQuery.data?.data.rootPath ?? "";
  const backupsRootPath = backupsQuery.data?.data.rootPath ?? "";

  const copyPath = (path: string) => {
    if (!path) return;
    navigator.clipboard.writeText(path);
    toast({
      title: t("skills.pathCopied"),
      description: t("skills.pathCopiedDesc"),
      variant: "default",
    });
  };

  const handleOpenFolder = async (path: string) => {
    if (!path) return;
    try {
      await api.openPath(path);
    } catch (err) {
      toast({
        title: t("skills.openFolderFailed"),
        description: formatInvokeError(err),
        variant: "destructive",
      });
    }
  };

  const filteredSkills = useMemo(() => {
    if (!searchQuery.trim()) return skills;
    const q = searchQuery.toLowerCase().trim();
    return skills.filter(
      (skill) =>
        (skill.title || "").toLowerCase().includes(q) ||
        skill.name.toLowerCase().includes(q) ||
        (skill.summary || "").toLowerCase().includes(q) ||
        skill.relativePath.toLowerCase().includes(q)
    );
  }, [skills, searchQuery]);

  const filteredBackups = useMemo(() => {
    if (!searchQuery.trim()) return backups;
    const q = searchQuery.toLowerCase().trim();
    return backups.filter(
      (backup) =>
        (backup.title || "").toLowerCase().includes(q) ||
        backup.name.toLowerCase().includes(q) ||
        backup.relativePath.toLowerCase().includes(q)
    );
  }, [backups, searchQuery]);

  const activeQuery = tab === "installed" ? skillsQuery : backupsQuery;
  if (activeQuery.error) {
    return (
      <div role="alert" className="space-y-3 p-4">
        <p>{t("baseline.readFailed")}</p>
        <p className="text-sm text-muted-foreground">{formatInvokeError(activeQuery.error)}</p>
        <Button onClick={() => void activeQuery.refetch()}>{t("common.refresh")}</Button>
      </div>
    );
  }
  if (activeQuery.isPending) {
    return <p role="status" className="p-4 text-muted-foreground">{t("baseline.loading")}</p>;
  }

  const currentRootPath = tab === "installed" ? skillsRootPath : backupsRootPath;

  return (
    <div className="space-y-6">
      {/* Page header */}
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h1 className="text-xl font-bold tracking-tight text-foreground">{t("skills.title")}</h1>
          <p className="mt-1 text-sm text-muted-foreground">{t("skills.description")}</p>
        </div>
        <div className="flex items-center gap-2">
          <RainbowButton
            size="sm"
            className="h-9 rounded-lg font-medium"
            onClick={() => importMutation.mutate()}
            disabled={skillsBusy}
          >
            <Upload className="h-4 w-4" />
            {t("skills.import")}
          </RainbowButton>
          <Button
            variant="outline"
            size="icon"
            className="h-9 w-9"
            onClick={handleRefresh}
            disabled={refreshBusy}
            aria-busy={refreshBusy}
            title={refreshBusy ? t("common.refreshing") : t("common.refresh")}
            aria-label={refreshBusy ? t("common.refreshing") : t("common.refresh")}
          >
            <ButtonBusyContent
              busy={refreshBusy}
              idleIcon={<RotateCw className="h-4 w-4" />}
            />
          </Button>
        </div>
      </div>

      {/* Stats & Quick directory access bar */}
      <div className="app-metric-grid grid grid-cols-1 gap-3 md:grid-cols-3">
        <AuroraCard color="blue">
          <div className="p-4 flex items-center justify-between">
            <div>
              <span className="text-xs font-medium text-muted-foreground">{t("skills.skillCount")}</span>
              <div className="mt-1 flex items-baseline gap-2">
                <span className="text-2xl font-bold tracking-tight tabular-nums">
                  {skills.length}
                </span>
              </div>
            </div>
            <div className="flex h-10 w-10 items-center justify-center rounded-xl bg-primary/10 text-primary">
              <Sparkles className="h-5 w-5" />
            </div>
          </div>
        </AuroraCard>

        <AuroraCard color="blue">
          <div className="p-4 flex items-center justify-between">
            <div>
              <span className="text-xs font-medium text-muted-foreground">{t("skills.backupCount")}</span>
              <div className="mt-1 flex items-baseline gap-2">
                <span className="text-2xl font-bold tracking-tight tabular-nums">{backups.length}</span>
              </div>
            </div>
            <div className="flex h-10 w-10 items-center justify-center rounded-xl bg-primary/10 text-primary">
              <Archive className="h-5 w-5" />
            </div>
          </div>
        </AuroraCard>

        <AuroraCard color="blue">
          <div className="p-4 flex flex-col justify-between h-full">
            <div className="flex items-center justify-between">
              <span className="text-xs font-medium text-muted-foreground">
                {tab === "installed" ? t("skills.rootPath") : t("skills.backupRootPath")}
              </span>
              <div className="flex items-center gap-1">
                <Button
                  variant="ghost"
                  size="icon-sm"
                  className="h-6 w-6 text-muted-foreground hover:text-foreground"
                  title={t("skills.openFolder")}
                  aria-label={t("skills.openFolder")}
                  onClick={() => handleOpenFolder(currentRootPath)}
                >
                  <FolderOpen className="h-3.5 w-3.5" />
                </Button>
                <Button
                  variant="ghost"
                  size="icon-sm"
                  className="h-6 w-6 text-muted-foreground hover:text-foreground"
                  title={t("skills.pathCopied")}
                  aria-label={t("skills.pathCopied")}
                  onClick={() => copyPath(currentRootPath)}
                >
                  <Copy className="h-3.5 w-3.5" />
                </Button>
              </div>
            </div>
            <div
              className="mt-1 flex items-center gap-1.5 cursor-pointer rounded px-1.5 py-1 -mx-1.5 transition-colors hover:bg-black/[0.04] dark:hover:bg-white/[0.04]"
              onClick={() => copyPath(currentRootPath)}
              title={currentRootPath}
            >
              <span className="truncate font-mono text-xs font-medium text-foreground/80">{currentRootPath}</span>
            </div>
          </div>
        </AuroraCard>
      </div>

      {/* Toolbar: Search input & Tab switch */}
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="relative min-w-[240px] max-w-sm flex-1">
          <Search className="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground pointer-events-none" />
          <Input
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            placeholder={t("skills.searchPlaceholder")}
            className="pl-9 pr-8 h-9 text-sm rounded-lg"
          />
          {searchQuery && (
            <button
              type="button"
              onClick={() => setSearchQuery("")}
              aria-label={t("skills.clearSearch")}
              className="absolute right-2.5 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground p-0.5 rounded"
            >
              <X className="h-3.5 w-3.5" />
            </button>
          )}
        </div>

        <SegmentedOptions
          items={[
            { value: "installed", label: `${t("skills.installed")} (${skills.length})` },
            { value: "backups", label: `${t("skills.backups")} (${backups.length})` },
          ]}
          value={tab}
          onChange={(value) => {
            setTab(value as Tab);
            setSearchQuery("");
          }}
        />
      </div>

      {/* Content Gallery Grid */}
      {scanWarnings.length > 0 && <div role="status" className="space-y-2 rounded-lg border border-amber-500/30 bg-amber-500/5 p-3">
        {scanWarnings.map((warning, index) => <div key={`${warning.code}-${index}`} className="flex min-w-0 flex-wrap items-center gap-2 text-xs">
          <span>{t(warning.code, { defaultValue: warning.code })}</span>
          <button type="button" className="min-w-0 break-all font-mono text-muted-foreground underline" onClick={() => handleOpenFolder(warning.message)}>{warning.message}</button>
        </div>)}
      </div>}
      {tab === "installed" ? (
        skills.length === 0 ? (
          <BentoCard>
            <div className="flex h-56 flex-col items-center justify-center text-center p-6">
              <div className="flex h-12 w-12 items-center justify-center rounded-2xl bg-primary/10">
                <Sparkles className="h-6 w-6 text-primary" />
              </div>
              <p className="mt-3.5 text-base font-semibold text-foreground">{t("skills.empty")}</p>
              <p className="mt-1 text-xs text-muted-foreground max-w-sm">
                直接导入包含 SKILL.md 的技能文件夹，让 Codex 拥有丰富的领域扩展能力。
              </p>
              <RainbowButton
                size="sm"
                className="mt-4 rounded-lg font-medium"
                onClick={() => importMutation.mutate()}
              >
                <Upload className="h-3.5 w-3.5" />
                {t("skills.import")}
              </RainbowButton>
            </div>
          </BentoCard>
        ) : filteredSkills.length === 0 ? (
          <BentoCard>
            <div className="flex h-44 flex-col items-center justify-center text-center p-6">
              <Search className="h-8 w-8 text-muted-foreground/60" />
              <p className="mt-2.5 text-sm font-medium text-muted-foreground">{t("skills.noFilterResults")}</p>
              <Button variant="outline" size="sm" className="mt-3" onClick={() => setSearchQuery("")}>
                {t("skills.clearSearch")}
              </Button>
            </div>
          </BentoCard>
        ) : (
          <div className="grid gap-4 sm:grid-cols-1 md:grid-cols-2 xl:grid-cols-3">
            {filteredSkills.map((skill) => (
              <BentoCard
                key={skill.id}
                compact
                className="group flex flex-col justify-between transition-all duration-200 hover:border-purple-500/40 hover:shadow-md"
              >
                <div>
                  {/* Card Header: Icon + Title + Slug */}
                  <div className="flex items-start gap-3">
                    <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl bg-primary/10 text-primary transition-transform group-hover:scale-105">
                      <Sparkles className="h-5 w-5" />
                    </div>
                    <div className="min-w-0 flex-1">
                      <div className="flex items-center gap-1.5">
                        <h2
                          className="text-[15px] font-bold tracking-tight text-foreground truncate"
                          title={skill.title || skill.name}
                        >
                          {skill.title || skill.name}
                        </h2>
                      </div>
                      <div className="mt-0.5">
                        <span className="font-mono text-[11px] text-muted-foreground bg-muted/80 px-1.5 py-0.5 rounded">
                          {skill.name}
                        </span>
                      </div>
                    </div>
                  </div>

                  {/* Summary / Description */}
                  <p
                    className="mt-3 text-xs text-muted-foreground line-clamp-2 leading-relaxed min-h-[32px]"
                    title={skill.summary ?? undefined}
                  >
                    {skill.summary || t("skills.noDescription")}
                  </p>

                  {/* Metadata Chips: Path & Update Time */}
                  <div className="mt-3 space-y-1 text-[11px] text-muted-foreground">
                    <div className="flex items-center gap-1.5 truncate">
                      <Folder className="h-3.5 w-3.5 shrink-0 opacity-60" />
                      <span className="truncate font-mono">{skill.relativePath}</span>
                    </div>
                    {skill.updatedAt != null && (
                      <div className="flex items-center gap-1.5">
                        <Clock className="h-3.5 w-3.5 shrink-0 opacity-60" />
                        <span>
                          {t("skills.updatedAt")} {formatDateTime(skill.updatedAt)}
                        </span>
                      </div>
                    )}
                  </div>
                </div>

                {/* Card Footer: Open Folder & Remove Action */}
                {skill.conflict && <div className="mt-3 flex flex-wrap gap-2">
                  <Button size="sm" variant="outline" disabled={skillsBusy} onClick={() => setConflictChoice({ id: skill.id, keepEnabled: true })}>{t("skills.keepActive")}</Button>
                  <Button size="sm" variant="outline" disabled={skillsBusy} onClick={() => setConflictChoice({ id: skill.id, keepEnabled: false })}>{t("skills.keepDisabled")}</Button>
                  {skill.conflictDirectoryPath && <Button size="sm" variant="ghost" title={skill.conflictDirectoryPath} onClick={() => handleOpenFolder(skill.conflictDirectoryPath!)}>{t(skill.enabled ? "skills.openDisabledFolder" : "skills.openActiveFolder")}</Button>}
                </div>}
                <div className="mt-4 pt-3 border-t border-border/50 flex flex-wrap gap-2 items-center justify-between">
                  {skill.conflict ? <span className="text-xs text-muted-foreground">{t("skills.conflict")}</span> : <label className="flex items-center gap-2 text-xs text-muted-foreground">
                    <Switch checked={skill.enabled !== false}
                      disabled={skillsBusy || skill.conflict}
                      aria-label={t("skills.toggle", { name: skill.title || skill.name })}
                      onCheckedChange={(enabled) => toggleMutation.mutate({ id: skill.id, enabled })} />
                    {t(skill.enabled === false ? "skills.disabled" : "skills.enabled")}
                  </label>}
                  <Button
                    variant="outline"
                    size="sm"
                    className="h-7 px-2.5 text-xs gap-1.5 text-foreground/85 hover:text-foreground"
                    onClick={() => handleOpenFolder(skill.directoryPath)}
                    title={skill.directoryPath}
                  >
                    <FolderOpen className="h-3.5 w-3.5 text-muted-foreground" />
                    {t("skills.openFolder")}
                  </Button>

                  <Button
                    variant="ghost"
                    size="icon-sm"
                    onClick={() => setRemoving(skill.id)}
                    disabled={skillsBusy || skill.conflict}
                    className="h-7 w-7 text-muted-foreground hover:text-destructive hover:bg-destructive/10"
                    title={t("skills.remove")}
                    aria-label={t("skills.remove")}
                  >
                    <Trash2 className="h-3.5 w-3.5" />
                  </Button>
                </div>
              </BentoCard>
            ))}
          </div>
        )
      ) : backups.length === 0 ? (
        <BentoCard>
          <div className="flex h-56 flex-col items-center justify-center text-center p-6">
            <div className="flex h-12 w-12 items-center justify-center rounded-2xl bg-primary/10">
              <Archive className="h-6 w-6 text-primary" />
            </div>
            <p className="mt-3.5 text-base font-semibold text-foreground">{t("skills.noBackups")}</p>
            <p className="mt-1 text-xs text-muted-foreground max-w-sm">
              当您移除已安装的 Skill 时，系统会自动创建完整归档备份，随时可以在此一键恢复。
            </p>
          </div>
        </BentoCard>
      ) : filteredBackups.length === 0 ? (
        <BentoCard>
          <div className="flex h-44 flex-col items-center justify-center text-center p-6">
            <Search className="h-8 w-8 text-muted-foreground/60" />
            <p className="mt-2.5 text-sm font-medium text-muted-foreground">{t("skills.noFilterResults")}</p>
            <Button variant="outline" size="sm" className="mt-3" onClick={() => setSearchQuery("")}>
              {t("skills.clearSearch")}
            </Button>
          </div>
        </BentoCard>
      ) : (
        <div className="grid gap-4 sm:grid-cols-1 md:grid-cols-2 xl:grid-cols-3">
          {filteredBackups.map((backup) => (
            <BentoCard
              key={backup.id}
              compact
              className="group flex flex-col justify-between transition-all duration-200 hover:border-blue-500/40 hover:shadow-md"
            >
              <div>
                {/* Header */}
                <div className="flex items-start gap-3">
                  <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl bg-primary/10 text-primary transition-transform group-hover:scale-105">
                    <Archive className="h-5 w-5" />
                  </div>
                  <div className="min-w-0 flex-1">
                    <h2
                      className="text-[15px] font-bold tracking-tight text-foreground truncate"
                      title={backup.title || backup.name}
                    >
                      {backup.title || backup.name}
                    </h2>
                    <div className="mt-0.5">
                      <span className="font-mono text-[11px] text-muted-foreground bg-muted/80 px-1.5 py-0.5 rounded">
                        {backup.name}
                      </span>
                    </div>
                  </div>
                </div>

                {/* Metadata */}
                <div className="mt-4 space-y-1.5 text-[11px] text-muted-foreground">
                  <div className="flex items-center gap-1.5">
                    <Clock className="h-3.5 w-3.5 shrink-0 opacity-60" />
                    <span>{formatDateTime(backup.createdAt)}</span>
                  </div>
                  <div className="flex items-center gap-1.5 truncate">
                    <Folder className="h-3.5 w-3.5 shrink-0 opacity-60" />
                    <span className="truncate font-mono">{backup.relativePath}</span>
                  </div>
                </div>
              </div>

              {/* Footer: Restore & Delete Backup */}
              <div className="mt-4 pt-3 border-t border-border/50 flex items-center justify-between">
                <Button
                  variant="outline"
                  size="sm"
                  className="h-7 px-2.5 text-xs gap-1.5 text-foreground/85 hover:text-foreground"
                  onClick={() => restoreMutation.mutate(backup.id)}
                  disabled={skillsBusy}
                >
                  <RotateCcw className="h-3.5 w-3.5 text-blue-500" />
                  {t("skills.restore")}
                </Button>

                <Button
                  variant="ghost"
                  size="icon-sm"
                  onClick={() => setDeletingBackup(backup.id)}
                  disabled={skillsBusy}
                  className="h-7 w-7 text-muted-foreground hover:text-destructive hover:bg-destructive/10"
                  title={t("skills.deleteBackup")}
                >
                  <Trash2 className="h-3.5 w-3.5" />
                </Button>
              </div>
            </BentoCard>
          ))}
        </div>
      )}

      {/* Remove skill confirm dialog */}
      <AlertDialog open={conflictChoice !== null} onOpenChange={(v) => !v && setConflictChoice(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t(conflictChoice?.keepEnabled ? "skills.keepActive" : "skills.keepDisabled")}</AlertDialogTitle>
            <AlertDialogDescription>{t("skills.confirmConflict")}</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction disabled={skillsBusy} onClick={() => { if (conflictChoice && !skillsBusy) conflictMutation.mutate(conflictChoice); }}>{t("common.confirm")}</AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
      <AlertDialog open={removing !== null} onOpenChange={(v) => !v && setRemoving(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("skills.remove")}</AlertDialogTitle>
            <AlertDialogDescription>{t("skills.confirmRemove")}</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction
              disabled={skillsBusy}
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
              onClick={() => removing && !skillsBusy && removeMutation.mutate(removing)}
            >
              {t("skills.remove")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      {/* Delete backup confirm dialog */}
      <AlertDialog open={deletingBackup !== null} onOpenChange={(v) => !v && setDeletingBackup(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("skills.deleteBackup")}</AlertDialogTitle>
            <AlertDialogDescription>{t("skills.confirmDeleteBackup")}</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction
              disabled={skillsBusy}
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
              onClick={() => deletingBackup && !skillsBusy && deleteBackupMutation.mutate(deletingBackup)}
            >
              {t("skills.deleteBackup")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}
