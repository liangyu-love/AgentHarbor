import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Archive, FolderOpen, Import, MoreHorizontal, Plus, RotateCcw, Sparkles, Trash2 } from "lucide-react";
import { api } from "@/lib/api";
import { cn } from "@/lib/utils";
import { formatDateTime } from "@/lib/format-time";
import { formatInvokeError } from "@/lib/invoke-error";
import { storageKeys } from "@/lib/storage-keys";
import { PageHero } from "@/components/layout/page-hero";
import { AGENT_GLYPHS, ClientGlyph } from "@/components/newapi/configuration-shortcuts";
import { AgentChannels, type AgentChannel } from "@/components/ui/agent-channels";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { ConfirmDialog, type ConfirmRequest } from "@/components/ui/confirm-dialog";
import { Dialog, DialogContent, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
import { SearchField } from "@/components/ui/search-field";
import { SegmentedOptions } from "@/components/ui/segmented-options";
import type { SkillApp, SkillInfo, SkillLibraryView, SkillView, UnmanagedSkill } from "@/types";
import { SKILL_APPS, SkillsMatrix } from "./skills-matrix";
import { useSkillLibrary, type SkillLibrary } from "./use-skill-library";

type Tab = "library" | "unmanaged" | "backups";
type Channel = "all" | SkillApp;
/** What an agent's own skill is to the library. */
type Kind = "importable" | "legacy" | "external" | "inLibrary";
const KINDS: Kind[] = ["importable", "legacy", "external", "inLibrary"];

const kindOf = (skill: UnmanagedSkill): Kind =>
  skill.external ? "external" : skill.inLibrary ? "inLibrary" : skill.legacyDisabled ? "legacy" : "importable";

/** The search matches the folder name, the title and the description. */
const matches = (needle: string, skill: Pick<SkillInfo, "name" | "title" | "description">) =>
  !needle || [skill.name, skill.title, skill.description].some(text => text?.toLowerCase().includes(needle));

/** A stored value, or the fallback when it is missing, unreadable or not what it should be. */
function readStored<T>(key: string, valid: (value: unknown) => value is T): T | null {
  try {
    const saved: unknown = JSON.parse(localStorage.getItem(key) ?? "null");
    return valid(saved) ? saved : null;
  } catch {
    return null;
  }
}

/** The agents new skills were last put in. */
function rememberedApps(installed: SkillApp[]): SkillApp[] {
  const saved = readStored(storageKeys.skillTargets, Array.isArray);
  if (!saved) return installed.filter(app => app === "codex" || app === "claudeCode");
  return saved.filter((app): app is SkillApp => installed.includes(app as SkillApp));
}

const isKinds = (value: unknown): value is Kind[] => Array.isArray(value) && value.every(kind => KINDS.includes(kind as Kind));

/** The kinds hidden last time: a hidden kind stays hidden until it is shown again or the filters are cleared. */
function useHiddenKinds() {
  const [hidden, setHidden] = useState<Kind[]>(() => readStored(storageKeys.skillHiddenKinds, isKinds) ?? []);
  useEffect(() => {
    try {
      localStorage.setItem(storageKeys.skillHiddenKinds, JSON.stringify(hidden));
    } catch {
      // Storage unavailable; the choice lasts this session.
    }
  }, [hidden]);
  const toggle = (kind: Kind) => setHidden(current => current.includes(kind) ? current.filter(item => item !== kind) : [...current, kind]);
  return { hidden, toggle, clear: () => setHidden([]) };
}

/** Every skill kept once and linked into the agents it is on. */
export function SkillsPage() {
  const { t } = useTranslation();
  const library = useSkillLibrary();
  const { view } = library;
  const [tab, setTab] = useState<Tab>("library");
  const [query, setQuery] = useState("");
  const [channel, setChannel] = useState<Channel>("all");
  const kinds = useHiddenKinds();
  const [adding, setAdding] = useState<string | null>(null);
  const [confirm, setConfirm] = useState<ConfirmRequest | null>(null);
  const needle = query.trim().toLowerCase();

  const skills = useMemo(() => (view?.skills ?? []).filter(skill =>
    matches(needle, skill) && (channel === "all" || skill.cells.some(cell => cell.app === channel && cell.on))),
  [view, needle, channel]);

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
  const clearFilters = () => {
    setQuery("");
    setChannel("all");
    kinds.clear();
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

  const channels: AgentChannel<Channel>[] = [
    {
      id: "all", label: t("skills.channels.all"), count: view.skills.length,
      note: view.unmanaged.length > 0 ? t("skills.channels.unmanaged", { count: view.unmanaged.length }) : undefined,
    },
    ...SKILL_APPS.map(app => {
      const target = view.apps.find(item => item.app === app);
      const own = view.unmanaged.filter(skill => skill.app === app).length;
      return {
        id: app, label: t(`skills.apps.${app}`), glyph: AGENT_GLYPHS[app],
        count: view.skills.filter(skill => skill.cells.some(cell => cell.app === app && cell.on)).length,
        note: !target?.installed ? t("skills.column.notInstalled") : own > 0 ? t("skills.channels.unmanaged", { count: own }) : undefined,
        muted: !target?.installed,
        title: target?.installed ? target.dir : t("skills.column.notInstalled"),
      };
    }),
  ];

  return (
    <div className="space-y-5" data-skills-page>
      <PageHero kicker="Skills" icon={<Sparkles />} title={t("skills.title")} description={t("skills.description")} actions={<>
        <Button size="sm" className="h-9 gap-1.5 rounded-lg" disabled={library.busy} title={t("skills.addHint")} onClick={() => void pick()}>
          <Plus className="h-4 w-4" /> {t("skills.add")}
        </Button>
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
      </>}>
        {/* Backups belong to no agent, so the channels stand aside there. */}
        {tab !== "backups" && <AgentChannels label={t("skills.channels.label")} channels={channels} value={channel} onChange={setChannel} />}
      </PageHero>

      {importable.length > 0 && tab !== "unmanaged" && (
        <div className="harbor-card route-rise flex flex-wrap items-center justify-between gap-3 !py-3 text-sm" data-skills-unmanaged-banner>
          <span className="flex items-center gap-2">
            <Import className="h-4 w-4 text-primary" />
            {t("skills.unmanaged.banner", { count: importable.length })}
          </span>
          <Button size="sm" variant="outline" className="h-8" onClick={() => setTab("unmanaged")}>{t("skills.unmanaged.review")}</Button>
        </div>
      )}

      <div className="route-rise flex flex-col gap-3 lg:flex-row lg:items-center lg:justify-between">
        <SegmentedOptions value={tab} onChange={value => setTab(value as Tab)} items={[
          { value: "library", label: `${t("skills.tabs.library")} (${view.skills.length})` },
          { value: "unmanaged", label: `${t("skills.tabs.unmanaged")} (${view.unmanaged.length})` },
          { value: "backups", label: `${t("skills.tabs.backups")} (${view.backups.length})` },
        ]} />
        <SearchField value={query} onChange={setQuery} placeholder={t("skills.searchPlaceholder")}
          clearLabel={t("skills.clearSearch")} className="lg:max-w-sm" />
      </div>

      {tab === "library" && (view.skills.length === 0 ? (
        <section className="harbor-card route-rise">
          <div className="flex h-56 flex-col items-center justify-center p-6 text-center">
            <span className="flex h-12 w-12 items-center justify-center rounded-2xl bg-foreground text-background">
              <Sparkles className="h-5 w-5" />
            </span>
            <p className="mt-3.5 text-base font-semibold text-foreground">{t("skills.empty")}</p>
            <p className="mt-1 max-w-sm text-xs text-muted-foreground">{t("skills.emptyHint")}</p>
            <div className="mt-4 flex gap-2">
              <Button size="sm" className="gap-1.5 rounded-lg" onClick={() => void pick()}><Plus className="h-3.5 w-3.5" /> {t("skills.add")}</Button>
              {importable.length > 0 && <Button variant="outline" size="sm" onClick={() => setTab("unmanaged")}>{t("skills.unmanaged.review")}</Button>}
            </div>
          </div>
        </section>
      ) : (
        <section className="harbor-card route-rise overflow-hidden !p-0">
          <div className="flex items-center justify-between gap-3 border-b border-border/60 px-4 py-3 sm:px-5">
            <p className="poster-kicker text-[10px] text-primary">Library</p>
            <span className="text-xs text-muted-foreground" data-skills-shown>
              {skills.length === view.skills.length
                ? t("skills.count", { count: view.skills.length })
                : t("skills.shown", { shown: skills.length, count: view.skills.length })}
            </span>
          </div>
          {skills.length === 0 ? (
            <NoMatch onClear={clearFilters} />
          ) : (
            <SkillsMatrix view={view} skills={skills} library={library} onUninstall={uninstall} />
          )}
        </section>
      ))}

      {tab === "unmanaged" && <UnmanagedList view={view} library={library} needle={needle} channel={channel} kinds={kinds} onClear={clearFilters} />}
      {tab === "backups" && <BackupList view={view} library={library} needle={needle} onConfirm={setConfirm} onClear={clearFilters} />}

      {adding && <AddDialog path={adding} view={view} library={library} onClose={() => setAdding(null)} />}
      <ConfirmDialog request={confirm} onClose={() => setConfirm(null)} />
    </div>
  );
}

function NoMatch({ onClear }: { onClear: () => void }) {
  const { t } = useTranslation();
  return (
    <div className="flex h-40 flex-col items-center justify-center text-center">
      <p className="text-sm text-muted-foreground">{t("skills.noFilterResults")}</p>
      <Button variant="outline" size="sm" className="mt-3" onClick={onClear}>{t("skills.clearFilters")}</Button>
    </div>
  );
}

/** Skills the agents hold themselves, by agent, without the kinds the user hid. */
function UnmanagedList({ view, library, needle, channel, kinds: { hidden, toggle }, onClear }: {
  view: SkillLibraryView;
  library: SkillLibrary;
  needle: string;
  channel: Channel;
  kinds: ReturnType<typeof useHiddenKinds>;
  onClear: () => void;
}) {
  const { t } = useTranslation();
  const inChannel = view.unmanaged.filter(skill => (channel === "all" || skill.app === channel) && matches(needle, skill));
  const shown = inChannel.filter(skill => !hidden.includes(kindOf(skill)));
  const importable = shown.filter(skill => !skill.external);
  const bring = (items: UnmanagedSkill[]) => void library.run(
    () => api.importSkills(items.map(item => ({ name: item.name, app: item.app, legacyDisabled: item.legacyDisabled }))),
    () => t("skills.imported", { count: items.length }),
  );
  if (view.unmanaged.length === 0) {
    return <section className="harbor-card route-rise"><p className="py-10 text-center text-sm text-muted-foreground">{t("skills.unmanaged.empty")}</p></section>;
  }
  const groups = SKILL_APPS.map(app => ({ app, items: shown.filter(skill => skill.app === app) })).filter(group => group.items.length > 0);
  const kindLabel = (kind: Kind) => kind === "importable" ? t("skills.kinds.importable") : t(`skills.unmanaged.${kind}`);
  return (
    <section className="harbor-card route-rise overflow-hidden !p-0" data-skills-unmanaged>
      <div className="space-y-3 border-b border-border/60 px-4 py-3.5 sm:px-5">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <p className="max-w-xl text-xs leading-relaxed text-muted-foreground">{t("skills.unmanaged.hint")}</p>
          <Button size="sm" className="h-8 gap-1.5" disabled={library.busy || importable.length === 0} onClick={() => bring(importable)}>
            <Import className="h-3.5 w-3.5" /> {t("skills.unmanaged.importAll")}
            {importable.length > 0 && <span className="tabular-nums opacity-80">({importable.length})</span>}
          </Button>
        </div>
        <div className="flex flex-wrap items-center gap-1.5" role="group" aria-label={t("skills.kinds.label")} data-skills-kinds>
          <span className="mr-1 text-[11px] text-muted-foreground">{t("skills.kinds.label")}</span>
          {KINDS.map(kind => {
            const count = inChannel.filter(skill => kindOf(skill) === kind).length;
            const off = hidden.includes(kind);
            return (
              <button key={kind} type="button" className="kind-chip" aria-pressed={!off} data-skills-kind={kind}
                title={off ? t("skills.kinds.show") : t("skills.kinds.hide")} onClick={() => toggle(kind)}>
                {kindLabel(kind)}<span className="tabular-nums opacity-70">{count}</span>
              </button>
            );
          })}
        </div>
      </div>
      {groups.length === 0 ? (
        <NoMatch onClear={onClear} />
      ) : groups.map(group => (
        <div key={group.app} data-skills-group={group.app}>
          <h3 className="flex items-center gap-2 bg-muted/35 px-4 py-2 text-xs font-semibold sm:px-5">
            <ClientGlyph client={AGENT_GLYPHS[group.app]} className="h-3.5 w-3.5" />
            {t(`skills.apps.${group.app}`)}
            <span className="font-normal tabular-nums text-muted-foreground">{group.items.length}</span>
          </h3>
          <ul className="divide-y divide-border/50">
            {group.items.map(skill => (
              <li key={`${skill.app}-${skill.legacyDisabled}-${skill.name}`} data-unmanaged-skill={skill.name} className="flex items-center gap-3 px-4 py-2.5 sm:px-5">
                <div className="min-w-0 flex-1">
                  <div className="flex flex-wrap items-center gap-1.5">
                    <span className="font-mono text-[13px] font-semibold">{skill.name}</span>
                    {skill.title && skill.title !== skill.name && <span className="truncate text-xs text-muted-foreground">{skill.title}</span>}
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
        </div>
      ))}
    </section>
  );
}

function BackupList({ view, library, needle, onConfirm, onClear }: {
  view: SkillLibraryView;
  library: SkillLibrary;
  needle: string;
  onConfirm: (request: ConfirmRequest | null) => void;
  onClear: () => void;
}) {
  const { t } = useTranslation();
  if (view.backups.length === 0) {
    return <section className="harbor-card route-rise"><p className="py-10 text-center text-sm text-muted-foreground">{t("skills.backups.empty")}</p></section>;
  }
  const backups = view.backups.filter(backup => matches(needle, { name: backup.name, title: backup.title ?? undefined }));
  if (backups.length === 0) return <section className="harbor-card route-rise !p-0"><NoMatch onClear={onClear} /></section>;
  return (
    <section className="harbor-card route-rise overflow-hidden !p-0" data-skills-backups>
      <ul className="divide-y divide-border/50">
        {backups.map(backup => (
          <li key={backup.id} data-skill-backup={backup.id} className="flex items-center gap-3 px-4 py-2.5 sm:px-5">
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
    </section>
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
