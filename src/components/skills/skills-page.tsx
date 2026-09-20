import { useState } from "react";
import { useTranslation } from "react-i18next";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api";
import { BentoCard } from "@/components/ui/bento-card";
import { AuroraCard } from "@/components/ui/aurora-card";
import { RainbowButton } from "@/registry/magicui/rainbow-button";
import { Button } from "@/components/ui/button";
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
import {
  Sparkles,
  Upload,
  Trash2,
  RotateCcw,
  Archive,
  Copy,
} from "lucide-react";

type Tab = "installed" | "backups";

export function SkillsPage() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [tab, setTab] = useState<Tab>("installed");
  const [removing, setRemoving] = useState<string | null>(null);
  const [deletingBackup, setDeletingBackup] = useState<string | null>(null);

  const skillsQuery = useQuery({
    queryKey: ["installed-skills"],
    queryFn: () => api.loadInstalledSkills(),
    staleTime: Infinity,
  });

  const backupsQuery = useQuery({
    queryKey: ["skill-backups"],
    queryFn: () => api.loadSkillBackups(),
  });

  const importMutation = useMutation({
    mutationFn: async () => {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const path = await open({ directory: true });
      if (typeof path === "string") return api.importSkill(path);
      throw new Error("cancelled");
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["installed-skills"] });
      queryClient.invalidateQueries({ queryKey: ["skill-backups"] });
    },
  });

  const removeMutation = useMutation({
    mutationFn: (id: string) => api.removeSkill(id),
    onSuccess: () => {
      setRemoving(null);
      queryClient.invalidateQueries({ queryKey: ["installed-skills"] });
      queryClient.invalidateQueries({ queryKey: ["skill-backups"] });
    },
  });

  const restoreMutation = useMutation({
    mutationFn: (id: string) => api.restoreSkillBackup(id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["installed-skills"] });
      queryClient.invalidateQueries({ queryKey: ["skill-backups"] });
    },
  });

  const deleteBackupMutation = useMutation({
    mutationFn: (id: string) => api.deleteSkillBackup(id),
    onSuccess: () => {
      setDeletingBackup(null);
      queryClient.invalidateQueries({ queryKey: ["skill-backups"] });
    },
  });

  const skills = skillsQuery.data?.data.items ?? [];
  const backups = backupsQuery.data?.data.items ?? [];
  const skillsRootPath = skillsQuery.data?.data.rootPath ?? "";
  const backupsRootPath = backupsQuery.data?.data.rootPath ?? "";

  const copyPath = (path: string) => {
    navigator.clipboard.writeText(path);
    toast({
      title: t("skills.pathCopied"),
      description: t("skills.pathCopiedDesc"),
      variant: "default",
    });
  };

  const activeQuery = tab === "installed" ? skillsQuery : backupsQuery;
  if (activeQuery.error) return <div role="alert" className="space-y-3 p-4"><p>{t("baseline.readFailed")}</p><p className="text-sm text-muted-foreground">{String(activeQuery.error)}</p><Button onClick={() => void activeQuery.refetch()}>{t("common.refresh")}</Button></div>;
  if (activeQuery.isPending) return <p role="status" className="p-4 text-muted-foreground">{t("baseline.loading")}</p>;

  return (
    <div className="space-y-6">
      {/* Page header */}
      <div className="flex items-center justify-between">
        <p className="max-w-md text-sm text-muted-foreground">{t("skills.description")}</p>
        <div className="flex items-center gap-2">
          <SegmentedOptions
            items={[
              { value: "installed", label: t("skills.installed") },
              { value: "backups", label: t("skills.backups") },
            ]}
            value={tab}
            onChange={(value) => setTab(value as Tab)}
          />
          <RainbowButton size="sm" className="h-8 rounded-lg dark:text-black" onClick={() => importMutation.mutate()} disabled={importMutation.isPending}>
            <Upload className="h-3.5 w-3.5" />
            {t("skills.import")}
          </RainbowButton>
        </div>
      </div>

      {/* Stats row */}
      <div className="grid grid-cols-2 gap-4 xl:grid-cols-4">
        <AuroraCard color="blue">
          <div className="p-4">
            <span className="text-xs text-muted-foreground">{t("skills.skillCount")}</span>
            <span className="mt-0.5 block text-xl font-semibold tracking-tight tabular-nums">{skills.length}</span>
          </div>
        </AuroraCard>
        <AuroraCard color="purple">
          <div className="p-4">
            <span className="text-xs text-muted-foreground">{t("skills.backupCount")}</span>
            <span className="mt-0.5 block text-xl font-semibold tracking-tight tabular-nums">{backups.length}</span>
          </div>
        </AuroraCard>
        <AuroraCard color="green">
          <div className="p-4">
            <span className="text-xs text-muted-foreground">{t("skills.rootPath")}</span>
            <button
              className="mt-1 flex w-full items-center gap-1.5 text-left"
              title={skillsRootPath}
              onClick={() => copyPath(skillsRootPath)}
            >
              <span className="min-w-0 flex-1 truncate text-sm font-medium">{skillsRootPath}</span>
              <Copy className="h-3 w-3 shrink-0 text-muted-foreground" />
            </button>
          </div>
        </AuroraCard>
        <AuroraCard color="red">
          <div className="p-4">
            <span className="text-xs text-muted-foreground">{t("skills.backupRootPath")}</span>
            <button
              className="mt-1 flex w-full items-center gap-1.5 text-left"
              title={backupsRootPath}
              onClick={() => copyPath(backupsRootPath)}
            >
              <span className="min-w-0 flex-1 truncate text-sm font-medium">{backupsRootPath}</span>
              <Copy className="h-3 w-3 shrink-0 text-muted-foreground" />
            </button>
          </div>
        </AuroraCard>
      </div>

      {/* List content */}
      {tab === "installed" ? (
        skills.length === 0 ? (
          <BentoCard>
            <div className="flex h-48 flex-col items-center justify-center">
              <div className="flex h-12 w-12 items-center justify-center rounded-2xl bg-gradient-to-br from-purple-500/15 to-purple-500/5 ring-1 ring-inset ring-purple-500/10">
                <Sparkles className="h-5 w-5 text-purple-500" />
              </div>
              <p className="mt-3 text-sm text-muted-foreground">{t("skills.empty")}</p>
            </div>
          </BentoCard>
        ) : (
          <BentoCard className="p-0">
            <div className="divide-y divide-border">
              {skills.map((skill) => (
                <div
                  key={skill.id}
                  className="group flex items-center justify-between px-5 py-4 transition-colors hover:bg-accent"
                >
                  <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl bg-purple-500/10 text-purple-500 ring-1 ring-inset ring-purple-500/15 transition-transform group-hover:scale-105 dark:text-purple-400">
                    <Sparkles className="h-4 w-4" />
                  </div>
                  <div className="ml-3 min-w-0 flex-1">
                    <p className="text-[14px] font-semibold">{skill.title || skill.name}</p>
                    {skill.summary && (
                      <p className="mt-1.5 truncate text-[13px] text-muted-foreground">{skill.summary}</p>
                    )}
                  </div>
                  <div className="ml-4 flex shrink-0 items-center gap-2 opacity-0 transition-opacity group-hover:opacity-100 group-focus-within:opacity-100">
                    <Button
                      variant="outline"
                      size="icon-sm"
                      onClick={() => setRemoving(skill.id)}
                      className="text-muted-foreground hover:bg-destructive hover:text-white hover:border-destructive"
                    >
                      <Trash2 className="h-3.5 w-3.5" />
                    </Button>
                  </div>
                </div>
              ))}
            </div>
          </BentoCard>
        )
      ) : backups.length === 0 ? (
        <BentoCard>
          <div className="flex h-48 flex-col items-center justify-center">
            <div className="flex h-12 w-12 items-center justify-center rounded-2xl bg-gradient-to-br from-blue-500/15 to-blue-500/5 ring-1 ring-inset ring-blue-500/10">
              <Archive className="h-5 w-5 text-blue-500" />
            </div>
            <p className="mt-3 text-sm text-muted-foreground">{t("skills.noBackups")}</p>
          </div>
        </BentoCard>
      ) : (
        <BentoCard className="p-0">
          <div className="divide-y divide-border">
            {backups.map((backup) => (
              <div
                key={backup.id}
                className="group flex items-center justify-between px-5 py-4 transition-colors hover:bg-accent"
              >
                <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl bg-blue-500/10 text-blue-500 ring-1 ring-inset ring-blue-500/15 transition-transform group-hover:scale-105 dark:text-blue-400">
                  <Archive className="h-4 w-4" />
                </div>
                <div className="ml-3 min-w-0 flex-1">
                  <p className="text-[14px] font-semibold">{backup.title || backup.name}</p>
                  <p className="mt-1.5 text-[13px] text-muted-foreground">
                    {formatDateTime(backup.createdAt)} · {backup.relativePath}
                  </p>
                </div>
                <div className="ml-4 flex shrink-0 items-center gap-2 opacity-0 transition-opacity group-hover:opacity-100 group-focus-within:opacity-100">
                  <Button variant="outline" size="sm" onClick={() => restoreMutation.mutate(backup.id)} disabled={restoreMutation.isPending}>
                    <RotateCcw className="h-3.5 w-3.5" />
                    {t("skills.restore")}
                  </Button>
                  <Button
                    variant="outline"
                    size="icon-sm"
                    onClick={() => setDeletingBackup(backup.id)}
                    className="text-muted-foreground hover:bg-destructive hover:text-white hover:border-destructive"
                  >
                    <Trash2 className="h-3.5 w-3.5" />
                  </Button>
                </div>
              </div>
            ))}
          </div>
        </BentoCard>
      )}

      {/* Remove skill confirm dialog */}
      <AlertDialog open={removing !== null} onOpenChange={(v) => !v && setRemoving(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("skills.remove")}</AlertDialogTitle>
            <AlertDialogDescription>{t("skills.confirmRemove")}</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
              onClick={() => removing && removeMutation.mutate(removing)}
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
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
              onClick={() => deletingBackup && deleteBackupMutation.mutate(deletingBackup)}
            >
              {t("skills.deleteBackup")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}
