import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Archive, FolderOpen, Import, MoreHorizontal, Plus, RotateCcw, Search, Sparkles, Trash2, X } from "lucide-react";
import { api } from "@/lib/api";
import { cn } from "@/lib/utils";
import { formatDateTime } from "@/lib/format-time";
import { formatInvokeError } from "@/lib/invoke-error";
import { storageKeys } from "@/lib/storage-keys";
import { PageHero } from "@/components/layout/page-hero";
import { BentoCard } from "@/components/ui/bento-card";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { ConfirmDialog, type ConfirmRequest } from "@/components/ui/confirm-dialog";
import { Dialog, DialogContent, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import { SegmentedOptions } from "@/components/ui/segmented-options";
import { RainbowButton } from "@/registry/magicui/rainbow-button";
import type { SkillApp, SkillLibraryView, SkillView, UnmanagedSkill } from "@/types";
import { SKILL_APPS, SkillsMatrix } from "./skills-matrix";
import { useSkillLibrary, type SkillLibrary } from "./use-skill-library";

type Tab = "library" | "unmanaged" | "backups";

/** The agents new skills were last put in. */
function rememberedApps(installed: SkillApp[]): SkillApp[] {
  try {
    const saved = JSON.parse(localStorage.getItem(storageKeys.skillTargets) ?? "null");
    if (Array.isArray(saved)) return saved.filter((app): app is SkillApp => installed.includes(app));
  } catch {
    // Unreadable: fall through to the default.
  }
  return installed.filter(app => app === "codex" || app === "claudeCode");
}

/** Every skill kept once and linked into the agents it is on. */
export function SkillsPage() {
  const { t } = useTranslation();
  const library = useSkillLibrary();
  const { view } = library;
  const [tab, setTab] = useState<Tab>("library");
  const [query, setQuery] = useState("");
  const [adding, setAdding] = useState<string | null>(null);
  const [confirm, setConfirm] = useState<ConfirmRequest | null>(null);

  const skills = useMemo(() => {
    const needle = query.trim().toLowerCase();
    if (!view || !needle) return view?.skills ?? [];
    return view.skills.filter(skill => [skill.name, skill.title, skill.description].some(text => text?.toLowerCase().includes(needle)));
  }, [view, query]);

  if (library.error) {
    return (
      <div role="alert" className="space-y-3 p-4">
        <p>{t("baseline.readFailed")}</p>
        <p className="text-sm text-muted-foreground">{formatInvokeError(library.error)}</p>
        <Button onClick={() => void library.refetch()}>{t("common.refresh")}</Button>
      </div>
    );
  }
  if (!view) return <p role="status" className="p-4 text-muted-foreground">{t("baseline.loading")}</p>;

  const importable = view.unmanaged.filter(skill => !skill.external);
  const pick = async () => {
    const path = await api.pickFolder().catch(() => null);
    if (path) setAdding(path);
  };
  const uninstall = (skill: SkillView) => setConfirm({
    title: t("skills.confirmUninstall", { name: skill.name }),
    description: t("skills.confirmUninstallDesc"),
    confirmLabel: t("skills.uninstall"),
    destructive: true,
    onConfirm: () => {
      setConfirm(null);
      void library.run(() => api.uninstallSkill(skill.name));
    },
  });

  return (
    <div className="space-y-6" data-skills-page>
      <PageHero kicker="Skills" icon={<Sparkles />} title={t("skills.title")} description={t("skills.description")} actions={<>
        <RainbowButton size="sm" className="h-9 rounded-lg font-medium" disabled={library.busy} title={t("skills.addHint")} onClick={() => void pick()}>
          <Plus className="h-4 w-4" /> {t("skills.add")}
        </RainbowButton>
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button variant="outline" size="icon" className="h-9 w-9" aria-label={t("skills.more")} title={t("skills.more")}>
              <MoreHorizontal className="h-4 w-4" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuItem onSelect={() => void api.openPath(view.library)}>
              <FolderOpen className="h-3.5 w-3.5" /> {t("skills.openLibrary")}
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </>} />

      {importable.length > 0 && tab !== "unmanaged" && (
        <div className="flex flex-wrap items-center justify-between gap-3 rounded-xl border border-primary/25 bg-primary/[0.04] px-4 py-2.5 text-sm" data-skills-unmanaged-banner>
          <span>{t("skills.unmanaged.banner", { count: importable.length })}</span>
          <Button size="sm" variant="outline" className="h-8" onClick={() => setTab("unmanaged")}>{t("skills.unmanaged.review")}</Button>
        </div>
      )}

      <SegmentedOptions value={tab} onChange={value => setTab(value as Tab)} items={[
        { value: "library", label: `${t("skills.tabs.library")} (${view.skills.length})` },
        { value: "unmanaged", label: `${t("skills.tabs.unmanaged")} (${view.unmanaged.length})` },
        { value: "backups", label: `${t("skills.tabs.backups")} (${view.backups.length})` },
      ]} />

      {tab === "library" && (view.skills.length === 0 ? (
        <BentoCard>
          <div className="flex h-56 flex-col items-center justify-center p-6 text-center">
            <div className="flex h-12 w-12 items-center justify-center rounded-2xl bg-gradient-to-br from-primary/20 to-primary/5 ring-1 ring-inset ring-primary/15">
              <Sparkles className="h-6 w-6 text-primary" />
            </div>
            <p className="mt-3.5 text-base font-semibold text-foreground">{t("skills.empty")}</p>
            <p className="mt-1 max-w-sm text-xs text-muted-foreground">{t("skills.emptyHint")}</p>
            <div className="mt-4 flex gap-2">
              <RainbowButton size="sm" className="rounded-lg" onClick={() => void pick()}><Plus className="h-3.5 w-3.5" /> {t("skills.add")}</RainbowButton>
              {importable.length > 0 && <Button variant="outline" size="sm" onClick={() => setTab("unmanaged")}>{t("skills.unmanaged.review")}</Button>}
            </div>
          </div>
        </BentoCard>
      ) : (
        <BentoCard className="p-0">
          <div className="flex flex-wrap items-center justify-between gap-3 border-b border-border/50 px-4 py-3">
            <div className="relative w-full max-w-xs">
              <Search className="pointer-events-none absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
              <Input value={query} onChange={event => setQuery(event.target.value)} placeholder={t("skills.searchPlaceholder")}
                aria-label={t("skills.searchPlaceholder")} className="h-8 pl-8 pr-8 text-xs" />
              {query && (
                <button type="button" aria-label={t("skills.clearSearch")} onClick={() => setQuery("")}
                  className="absolute right-2 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground">
                  <X className="h-3.5 w-3.5" />
                </button>
              )}
            </div>
            <span className="text-xs text-muted-foreground">{t("skills.count", { count: view.skills.length })}</span>
          </div>
          {skills.length === 0 ? (
            <div className="flex h-40 flex-col items-center justify-center text-center">
              <p className="text-sm text-muted-foreground">{t("skills.noFilterResults")}</p>
              <Button variant="outline" size="sm" className="mt-3" onClick={() => setQuery("")}>{t("skills.clearSearch")}</Button>
            </div>
          ) : (
            <SkillsMatrix view={view} skills={skills} library={library} onUninstall={uninstall} />
          )}
        </BentoCard>
      ))}

      {tab === "unmanaged" && <UnmanagedList view={view} library={library} />}
      {tab === "backups" && <BackupList view={view} library={library} onConfirm={setConfirm} />}

      {adding && <AddDialog path={adding} view={view} library={library} onClose={() => setAdding(null)} />}
      <ConfirmDialog request={confirm} onClose={() => setConfirm(null)} />
    </div>
  );
}

function UnmanagedList({ view, library }: { view: SkillLibraryView; library: SkillLibrary }) {
  const { t } = useTranslation();
  const importable = view.unmanaged.filter(skill => !skill.external);
  const bring = (items: UnmanagedSkill[]) => void library.run(
    () => api.importSkills(items.map(item => ({ name: item.name, app: item.app, legacyDisabled: item.legacyDisabled }))),
    () => t("skills.imported", { count: items.length }),
  );
  if (view.unmanaged.length === 0) {
    return <BentoCard><p className="py-10 text-center text-sm text-muted-foreground">{t("skills.unmanaged.empty")}</p></BentoCard>;
  }
  return (
    <BentoCard className="p-0" data-skills-unmanaged>
      <div className="flex flex-wrap items-center justify-between gap-3 border-b border-border/50 px-4 py-3">
        <p className="max-w-xl text-xs leading-relaxed text-muted-foreground">{t("skills.unmanaged.hint")}</p>
        <Button size="sm" className="h-8 gap-1.5" disabled={library.busy || importable.length === 0} onClick={() => bring(importable)}>
          <Import className="h-3.5 w-3.5" /> {t("skills.unmanaged.importAll")}
        </Button>
      </div>
      <ul className="divide-y divide-border/50">
        {view.unmanaged.map(skill => (
          <li key={`${skill.app}-${skill.legacyDisabled}-${skill.name}`} data-unmanaged-skill={skill.name} className="flex items-center gap-3 px-4 py-2.5">
            <div className="min-w-0 flex-1">
              <div className="flex flex-wrap items-center gap-1.5">
                <span className="font-mono text-[13px] font-semibold">{skill.name}</span>
                <span className="rounded bg-muted px-1.5 py-px text-[10px] text-muted-foreground">{t(`skills.apps.${skill.app}`)}</span>
                {skill.legacyDisabled && <span className="rounded bg-amber-500/10 px-1.5 py-px text-[10px] text-amber-700 dark:text-amber-300">{t("skills.unmanaged.legacy")}</span>}
                {skill.external && <span className="rounded bg-muted px-1.5 py-px text-[10px] text-muted-foreground">{t("skills.unmanaged.external")}</span>}
                {skill.inLibrary && <span className="rounded bg-primary/10 px-1.5 py-px text-[10px] text-primary">{t("skills.unmanaged.inLibrary")}</span>}
              </div>
              <p className="mt-0.5 truncate text-[11px] text-muted-foreground" title={skill.path}>{skill.description || skill.path}</p>
            </div>
            {!skill.external && (
              <Button size="sm" variant="outline" className="h-8 shrink-0" disabled={library.busy} onClick={() => bring([skill])}>
                {t("skills.unmanaged.import")}
              </Button>
            )}
          </li>
        ))}
      </ul>
    </BentoCard>
  );
}

function BackupList({ view, library, onConfirm }: {
  view: SkillLibraryView;
  library: SkillLibrary;
  onConfirm: (request: ConfirmRequest | null) => void;
}) {
  const { t } = useTranslation();
  if (view.backups.length === 0) {
    return <BentoCard><p className="py-10 text-center text-sm text-muted-foreground">{t("skills.backups.empty")}</p></BentoCard>;
  }
  return (
    <BentoCard className="p-0" data-skills-backups>
      <ul className="divide-y divide-border/50">
        {view.backups.map(backup => (
          <li key={backup.id} data-skill-backup={backup.id} className="flex items-center gap-3 px-4 py-2.5">
            <Archive className="h-4 w-4 shrink-0 text-muted-foreground" />
            <div className="min-w-0 flex-1">
              <p className="truncate font-mono text-[13px] font-semibold">{backup.name}</p>
              <p className="truncate text-[11px] text-muted-foreground" title={backup.backupPath}>{formatDateTime(backup.createdAt)}</p>
            </div>
            <Button size="sm" variant="outline" className="h-8 shrink-0 gap-1.5" disabled={library.busy}
              onClick={() => void library.run(() => api.restoreSkill(backup.id), () => t("skills.restored"))}>
              <RotateCcw className="h-3.5 w-3.5" /> {t("skills.backups.restore")}
            </Button>
            <Button size="icon" variant="ghost" className="h-8 w-8 shrink-0" disabled={library.busy}
              aria-label={t("skills.backups.delete")} title={t("skills.backups.delete")}
              onClick={() => onConfirm({
                title: t("skills.backups.confirmDelete"),
                description: t("skills.backups.confirmDeleteDesc"),
                confirmLabel: t("skills.backups.delete"),
                destructive: true,
                onConfirm: () => {
                  onConfirm(null);
                  void library.run(() => api.deleteSkillBackup(backup.id));
                },
              })}>
              <Trash2 className="h-3.5 w-3.5" />
            </Button>
          </li>
        ))}
      </ul>
    </BentoCard>
  );
}

function AddDialog({ path, view, library, onClose }: {
  path: string;
  view: SkillLibraryView;
  library: SkillLibrary;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const installed = view.apps.filter(app => app.installed).map(app => app.app);
  const [apps, setApps] = useState<SkillApp[]>(() => rememberedApps(installed));
  const name = path.split(/[\\/]/).filter(Boolean).pop() ?? path;
  const add = async () => {
    try {
      localStorage.setItem(storageKeys.skillTargets, JSON.stringify(apps));
    } catch {
      // Storage unavailable; next time starts from the default.
    }
    const added = await library.run(() => api.addSkill(path, apps), () => t("skills.added", { name }));
    if (added) onClose();
  };
  return (
    <Dialog open onOpenChange={open => !open && !library.busy && onClose()}>
      <DialogContent className="sm:max-w-lg" data-skills-add>
        <DialogHeader>
          <DialogTitle>{t("skills.addDialog.title")}</DialogTitle>
        </DialogHeader>
        <p className="break-all rounded-lg bg-muted/50 px-3 py-2 font-mono text-xs">{path}</p>
        <div className="space-y-2">
          <p className="text-xs font-medium text-muted-foreground">{t("skills.addDialog.writeTo")}</p>
          <div className="flex flex-wrap gap-x-4 gap-y-2">
            {SKILL_APPS.map(app => {
              const ready = installed.includes(app);
              return (
                <label key={app} title={ready ? view.apps.find(item => item.app === app)?.dir : t("skills.column.notInstalled")}
                  className={cn("inline-flex items-center gap-2 text-sm", ready ? "cursor-pointer" : "cursor-not-allowed opacity-50")}>
                  <Checkbox checked={ready && apps.includes(app)} disabled={!ready} aria-label={t(`skills.apps.${app}`)}
                    onCheckedChange={on => setApps(current => on === true ? [...new Set([...current, app])] : current.filter(item => item !== app))} />
                  {t(`skills.apps.${app}`)}
                </label>
              );
            })}
          </div>
        </div>
        <DialogFooter>
          <Button variant="outline" size="sm" disabled={library.busy} onClick={onClose}>{t("common.cancel")}</Button>
          <Button size="sm" disabled={library.busy} onClick={() => void add()}>{t("skills.addDialog.confirm")}</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
