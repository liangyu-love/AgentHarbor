import { useId, useState } from "react";
import { useTranslation } from "react-i18next";
import { ChevronDown, ChevronRight, Folder, MessagesSquare } from "lucide-react";
import { cn } from "@/lib/utils";
import type { FolderGroup } from "@/components/threads/thread-utils";
import type { ThreadList } from "@/components/threads/use-thread-list";

const PAGE_SIZE = 30;
const rowClass = "flex min-w-0 items-center gap-2 rounded-lg px-2 py-2 text-left text-xs hover:bg-muted focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring";
type ProjectNavigation = Pick<ThreadList, "projects" | "threads" | "matchingKeys" | "activeProject" | "focusedThread" | "selectProject" | "focusThread">;

function ProjectBranch({ project, list, repeatedName }: { project: FolderGroup; list: ProjectNavigation; repeatedName: boolean }) {
  const { t } = useTranslation();
  const id = useId();
  const [expanded, setExpanded] = useState(false);
  const [limit, setLimit] = useState(PAGE_SIZE);
  const matching = project.items.filter(thread => list.matchingKeys.has(thread.key));
  const active = list.activeProject?.key === project.key;
  return (
    <li>
      <div className={cn("flex rounded-lg", active && "bg-primary/10 text-primary")}>
        <button type="button" className={cn(rowClass, "shrink-0 px-1")}
          aria-label={t("threads.toggleProject", { name: project.folderName })} aria-expanded={expanded} aria-controls={id}
          onClick={() => { setExpanded(value => !value); setLimit(PAGE_SIZE); }}>
          {expanded ? <ChevronDown className="h-3.5 w-3.5" /> : <ChevronRight className="h-3.5 w-3.5" />}
        </button>
        <button type="button" className={cn(rowClass, "flex-1 pl-0")} title={project.cwd || project.folderName}
          aria-current={active && !list.focusedThread ? "page" : undefined} onClick={() => list.selectProject(project.key)}>
          <Folder className="h-3.5 w-3.5 shrink-0" />
          <span className="min-w-0 flex-1">
            <span className="block truncate font-medium">{project.folderName}</span>
            {repeatedName && <span className="block truncate text-[10px] text-muted-foreground">{project.cwd}</span>}
          </span>
          <span className="shrink-0 tabular-nums text-muted-foreground">{matching.length === project.items.length ? project.items.length : `${matching.length}/${project.items.length}`}</span>
        </button>
      </div>
      {expanded && <ul id={id} className="ml-3 border-l border-border pl-2">
        {matching.slice(0, limit).map(thread => <li key={thread.key}>
          <button type="button" className={cn(rowClass, "w-full", list.focusedThread?.key === thread.key && "bg-primary/10 text-primary")}
            title={thread.title || thread.id} aria-current={list.focusedThread?.key === thread.key ? "page" : undefined}
            onClick={() => list.focusThread(project.key, thread.key)}>
            <MessagesSquare className="h-3 w-3 shrink-0 text-muted-foreground" />
            <span className="truncate">{thread.title || t("threads.untitled")}</span>
          </button>
        </li>)}
        {matching.length === 0 && <li className="px-2 py-2 text-xs text-muted-foreground">{t("threads.noFilteredResults")}</li>}
        {matching.length > limit && <li><button type="button" className={cn(rowClass, "w-full text-primary")}
          onClick={() => setLimit(value => value + PAGE_SIZE)}>{t("threads.showMoreThreads", { count: matching.length - limit })}</button></li>}
      </ul>}
    </li>
  );
}

export function ThreadProjectTree({ list }: { list: ProjectNavigation }) {
  const { t } = useTranslation();
  const id = useId();
  const [open, setOpen] = useState(() => window.matchMedia("(min-width: 1280px)").matches);
  const names = new Map<string, number>();
  for (const project of list.projects) names.set(project.folderName, (names.get(project.folderName) ?? 0) + 1);
  return (
    <nav aria-label={t("threads.projectTree")} className="min-w-0 self-start rounded-xl bg-muted/25 p-2 xl:sticky xl:top-0">
      <button type="button" className={cn(rowClass, "w-full justify-between font-semibold")}
        aria-expanded={open} aria-controls={id} onClick={() => setOpen(value => !value)}>
        <span>{t("threads.projectTree")}</span>
        <span className="flex items-center gap-2 text-muted-foreground"><span className="tabular-nums">{list.projects.length}</span>
          {open ? <ChevronDown className="h-3.5 w-3.5" /> : <ChevronRight className="h-3.5 w-3.5" />}</span>
      </button>
      {open && <div id={id} className="max-h-[50vh] overflow-y-auto xl:max-h-[calc(100vh-220px)]">
        <button type="button" className={cn(rowClass, "my-1 w-full", !list.activeProject && "bg-primary/10 text-primary")}
          aria-current={!list.activeProject ? "page" : undefined} onClick={() => list.selectProject(null)}>
          <MessagesSquare className="h-3.5 w-3.5 shrink-0" /><span className="flex-1">{t("threads.allThreads")}</span>
          <span className="tabular-nums">{list.matchingKeys.size === list.threads.length ? list.threads.length : `${list.matchingKeys.size}/${list.threads.length}`}</span>
        </button>
        <ul className="space-y-0.5">{list.projects.map(project => <ProjectBranch key={project.key} project={project} list={list} repeatedName={(names.get(project.folderName) ?? 0) > 1} />)}</ul>
      </div>}
    </nav>
  );
}
