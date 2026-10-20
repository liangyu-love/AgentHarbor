import { useTranslation } from "react-i18next";
import { AlertTriangle, Check, FolderOpen, Minus, MoreHorizontal, Power, PowerOff, Trash2 } from "lucide-react";
import { api } from "@/lib/api";
import { cn } from "@/lib/utils";
import { AGENT_GLYPHS, ClientGlyph } from "@/components/newapi/configuration-shortcuts";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuLabel, DropdownMenuSeparator, DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import type { SkillApp, SkillCell, SkillLibraryView, SkillView } from "@/types";
import type { SkillLibrary } from "./use-skill-library";

/** The agents in the order the matrix shows them. */
export const SKILL_APPS: SkillApp[] = ["codex", "claudeCode", "openCode", "pi", "cursor", "harness"];

function CellMark({ cell }: { cell: SkillCell }) {
  if (cell.state === "notInstalled") return <Minus className="h-3.5 w-3.5 text-muted-foreground/50" />;
  if (cell.state === "on") return <Check className="h-3.5 w-3.5" strokeWidth={3} />;
  if (cell.state === "occupied") return <AlertTriangle className="h-3.5 w-3.5" />;
  return <span className="h-2.5 w-2.5 rounded-full border-[1.5px] border-muted-foreground/45" />;
}

/** Rows are skills, columns are agents; each cell links the skill into that agent or takes the link out. */
export function SkillsMatrix({ view, skills, library, onUninstall }: {
  view: SkillLibraryView;
  /** The skills shown, after search. */
  skills: SkillView[];
  library: SkillLibrary;
  onUninstall: (skill: SkillView) => void;
}) {
  const { t } = useTranslation();
  const names = skills.map(skill => skill.name);
  const set = (list: string[], app: SkillApp | null, on: boolean) => void library.run(() => api.setSkills(list, app, on));

  return (
    <div className="overflow-x-auto" data-skills-matrix>
      <table className="w-full min-w-[720px] border-separate border-spacing-0 text-sm">
        <thead>
          <tr>
            <th scope="col" className="sticky left-0 z-10 bg-card px-3 py-3 text-left text-xs font-medium text-muted-foreground sm:px-5">
              {t("skills.skill")}
            </th>
            {SKILL_APPS.map(app => {
              const target = view.apps.find(item => item.app === app);
              return (
                <th key={app} scope="col" className="px-1 py-2 text-center align-bottom">
                  <DropdownMenu>
                    <DropdownMenuTrigger asChild>
                      <button type="button" data-skills-column={app} disabled={library.busy || !target?.installed}
                        title={target?.installed ? target.dir : t("skills.column.notInstalled")}
                        className={cn(
                          "inline-flex max-w-[6.5rem] flex-col items-center gap-1 rounded-lg px-1.5 py-1.5 text-[11px] font-medium leading-tight transition-colors hover:bg-muted disabled:hover:bg-transparent",
                          target?.installed ? "text-foreground" : "text-muted-foreground/60",
                        )}>
                        <ClientGlyph client={AGENT_GLYPHS[app]} className="h-4 w-4" />
                        {t(`skills.apps.${app}`)}
                        {!target?.installed && <span className="text-[10px] font-normal">{t("skills.column.notInstalled")}</span>}
                      </button>
                    </DropdownMenuTrigger>
                    <DropdownMenuContent align="center" className="w-56">
                      <DropdownMenuLabel className="break-all text-[11px] font-normal text-muted-foreground">{target?.dir}</DropdownMenuLabel>
                      <DropdownMenuSeparator />
                      <DropdownMenuItem onSelect={() => set(names, app, true)}><Power className="h-3.5 w-3.5" />{t("skills.column.enableAll")}</DropdownMenuItem>
                      <DropdownMenuItem onSelect={() => set(names, app, false)}><PowerOff className="h-3.5 w-3.5" />{t("skills.column.disableAll")}</DropdownMenuItem>
                    </DropdownMenuContent>
                  </DropdownMenu>
                </th>
              );
            })}
            <th scope="col" className="w-20 px-2" aria-label={t("skills.more")} />
          </tr>
        </thead>
        <tbody>
          {skills.map(skill => (
            <tr key={skill.name} data-skill={skill.name}>
              <th scope="row" className="sticky left-0 z-10 w-40 max-w-40 border-t border-border/50 bg-card px-3 py-2.5 text-left font-normal shadow-[1px_0_0_hsl(var(--border)/0.5)] sm:w-auto sm:max-w-[20rem] sm:px-5">
                <div className="flex min-w-0 items-baseline gap-2">
                  <span className="truncate font-mono text-[13px] font-semibold text-foreground">{skill.name}</span>
                  {skill.title && skill.title !== skill.name && <span className="truncate text-xs text-muted-foreground">{skill.title}</span>}
                </div>
                <p className="mt-0.5 line-clamp-2 text-[11px] leading-relaxed text-muted-foreground" title={skill.description}>
                  {skill.description || t("skills.noDescription")}
                </p>
              </th>
              {skill.cells.map(cell => {
                const app = t(`skills.apps.${cell.app}`);
                const locked = cell.state === "notInstalled";
                return (
                  <td key={cell.app} className="border-t border-border/50 px-1 py-2 text-center">
                    <button type="button" role="switch" aria-checked={cell.on} data-skill-cell={cell.app} data-state={cell.state}
                      aria-label={t("skills.toggle", { skill: skill.name, app })} title={`${app} · ${t(`skills.state.${cell.state}`)}`}
                      disabled={library.busy || locked}
                      onClick={() => set([skill.name], cell.app, !cell.on)}
                      className={cn(
                        "inline-flex h-8 w-8 items-center justify-center rounded-lg ring-1 ring-inset transition-colors disabled:cursor-not-allowed",
                        locked ? "bg-transparent ring-transparent"
                          : cell.on ? "bg-primary text-primary-foreground ring-primary/30 hover:bg-primary/90"
                            : cell.state === "occupied" ? "bg-amber-500/15 text-amber-700 ring-amber-500/40 hover:bg-amber-500/25 dark:text-amber-300"
                              : "bg-background ring-border hover:bg-muted",
                      )}>
                      <CellMark cell={cell} />
                    </button>
                  </td>
                );
              })}
              <td className="border-t border-border/50 px-2 py-2 text-right">
                <div className="flex items-center justify-end gap-0.5">
                  <Button variant="ghost" size="icon" className="h-8 w-8" aria-label={t("skills.row.openFolder")} title={t("skills.row.openFolder")}
                    onClick={() => void api.openPath(skill.path)}>
                    <FolderOpen className="h-3.5 w-3.5" />
                  </Button>
                  <DropdownMenu>
                    <DropdownMenuTrigger asChild>
                      <Button variant="ghost" size="icon" className="h-8 w-8" aria-label={t("skills.more")} disabled={library.busy}>
                        <MoreHorizontal className="h-4 w-4" />
                      </Button>
                    </DropdownMenuTrigger>
                    <DropdownMenuContent align="end">
                      <DropdownMenuItem onSelect={() => set([skill.name], null, true)}><Power className="h-3.5 w-3.5" />{t("skills.row.enableAll")}</DropdownMenuItem>
                      <DropdownMenuItem onSelect={() => set([skill.name], null, false)}><PowerOff className="h-3.5 w-3.5" />{t("skills.row.disableAll")}</DropdownMenuItem>
                      <DropdownMenuSeparator />
                      <DropdownMenuItem className="text-destructive focus:text-destructive" onSelect={() => onUninstall(skill)}>
                        <Trash2 className="h-3.5 w-3.5" />{t("skills.row.uninstall")}
                      </DropdownMenuItem>
                    </DropdownMenuContent>
                  </DropdownMenu>
                </div>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
