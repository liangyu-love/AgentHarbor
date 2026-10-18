import { useId, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  ChevronDown,
  ChevronRight,
  Folder,
  FolderGit2,
  FolderOpen,
  MessagesSquare,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { useScrollPaging } from "@/hooks/use-scroll-paging";
import type { FolderGroup } from "@/components/threads/thread-utils";
import type { ThreadList } from "@/components/threads/use-thread-list";

const PAGE_SIZE = 30;
const rowClass =
  "flex min-w-0 items-center gap-2 rounded-xl px-2.5 py-1.5 text-left text-xs transition-all duration-150 hover:bg-muted/70 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring active:scale-[0.99]";

type ProjectNavigation = Pick<
  ThreadList,
  | "projects"
  | "threads"
  | "matchingKeys"
  | "activeProject"
  | "focusedThread"
  | "selectProject"
  | "focusThread"
>;

function ProjectBranch({
  project,
  list,
  repeatedName,
}: {
  project: FolderGroup;
  list: ProjectNavigation;
  repeatedName: boolean;
}) {
  const { t } = useTranslation();
  const id = useId();
  const [expanded, setExpanded] = useState(false);

  const matching = project.items.filter((thread) =>
    list.matchingKeys.has(thread.key),
  );
  const paging = useScrollPaging(matching.length, PAGE_SIZE);
  const active = list.activeProject?.key === project.key;

  return (
    <li className="relative">
      <div
        className={cn(
          "group/proj flex items-center rounded-xl transition-all duration-150",
          active &&
            !list.focusedThread &&
            "bg-primary/10 text-primary shadow-xs ring-1 ring-primary/20",
          active && list.focusedThread && "bg-muted/50 text-foreground",
        )}
      >
        {/* Toggle chevron */}
        <button
          type="button"
          className={cn(
            rowClass,
            "h-8 w-7 shrink-0 justify-center px-0 text-muted-foreground hover:text-foreground",
          )}
          aria-label={t("threads.toggleProject", { name: project.folderName })}
          aria-expanded={expanded}
          aria-controls={id}
          onClick={() => {
            setExpanded((value) => !value);
            paging.reset();
          }}
        >
          <ChevronRight
            className={cn(
              "h-3.5 w-3.5 transition-transform duration-200",
              expanded && "rotate-90 text-foreground",
            )}
          />
        </button>

        {/* Project row */}
        <button
          type="button"
          className={cn(rowClass, "flex-1 pl-0.5 pr-2")}
          title={project.cwd || project.folderName}
          aria-current={active && !list.focusedThread ? "page" : undefined}
          onClick={() => list.selectProject(project.key)}
        >
          <div
            className={cn(
              "flex h-5 w-5 shrink-0 items-center justify-center rounded-md transition-colors",
              active
                ? "bg-primary/20 text-primary"
                : "bg-muted text-muted-foreground group-hover/proj:text-foreground",
            )}
          >
            {expanded ? (
              <FolderOpen className="h-3 w-3" />
            ) : (
              <Folder className="h-3 w-3" />
            )}
          </div>
          <span className="min-w-0 flex-1">
            <span className="block truncate font-medium tracking-tight">
              {project.folderName}
            </span>
            {repeatedName && (
              <span className="block truncate font-mono text-[10px] text-muted-foreground/75">
                {project.cwd}
              </span>
            )}
          </span>
          <span
            className={cn(
              "shrink-0 rounded-md px-1.5 py-0.2 font-mono text-[10px] tabular-nums transition-colors",
              active
                ? "bg-primary/20 font-semibold text-primary"
                : "bg-muted/80 text-muted-foreground",
            )}
          >
            {matching.length === project.items.length
              ? project.items.length
              : `${matching.length}/${project.items.length}`}
          </span>
        </button>
      </div>

      {/* Expanded threads list */}
      {expanded && (
        <ul
          id={id}
          className="relative ml-4 mt-1 space-y-0.5 border-l-2 border-border/60 pl-2.5 pb-1"
        >
          {matching.slice(0, paging.limit).map((thread) => {
            const isFocused = list.focusedThread?.key === thread.key;
            return (
              <li key={thread.key}>
                <button
                  type="button"
                  className={cn(
                    rowClass,
                    "w-full py-1 text-[11.5px] rounded-lg",
                    isFocused
                      ? "bg-primary/15 font-semibold text-primary shadow-xs ring-1 ring-primary/25"
                      : "text-muted-foreground hover:text-foreground",
                  )}
                  title={thread.title || thread.id}
                  aria-current={isFocused ? "page" : undefined}
                  onClick={() => list.focusThread(project.key, thread.key)}
                >
                  <MessagesSquare
                    className={cn(
                      "h-3 w-3 shrink-0 transition-colors",
                      isFocused ? "text-primary" : "text-muted-foreground/60",
                    )}
                  />
                  <span className="truncate">
                    {thread.title || t("threads.untitled")}
                  </span>
                </button>
              </li>
            );
          })}
          {matching.length === 0 && (
            <li className="px-2 py-1.5 text-xs text-muted-foreground italic">
              {t("threads.noFilteredResults")}
            </li>
          )}
          {paging.more && <li ref={paging.endRef} aria-hidden className="h-px" />}
        </ul>
      )}
    </li>
  );
}

export function ThreadProjectTree({ list }: { list: ProjectNavigation }) {
  const { t } = useTranslation();
  const id = useId();
  const [open, setOpen] = useState(
    () => typeof window !== "undefined" && window.matchMedia("(min-width: 1280px)").matches,
  );

  const names = new Map<string, number>();
  for (const project of list.projects) {
    names.set(project.folderName, (names.get(project.folderName) ?? 0) + 1);
  }

  return (
    <nav
      aria-label={t("threads.projectTree")}
      className="min-w-0 self-start rounded-2xl border border-border/70 bg-card/75 p-2.5 backdrop-blur-md shadow-xs transition-all duration-200 xl:sticky xl:top-3"
    >
      {/* Header bar */}
      <button
        type="button"
        className={cn(
          rowClass,
          "w-full justify-between rounded-xl px-2 py-1.5 font-semibold text-foreground hover:bg-muted/80",
        )}
        aria-expanded={open}
        aria-controls={id}
        onClick={() => setOpen((value) => !value)}
      >
        <span className="flex items-center gap-2">
          <div className="flex h-5 w-5 items-center justify-center rounded-md bg-primary/10 text-primary">
            <FolderGit2 className="h-3 w-3" />
          </div>
          <span className="text-xs font-bold tracking-tight">
            {t("threads.projectTree")}
          </span>
        </span>
        <span className="flex items-center gap-1.5 text-muted-foreground">
          <span className="rounded-full bg-muted px-2 py-0.2 font-mono text-[10px] font-semibold tabular-nums text-foreground">
            {list.projects.length}
          </span>
          {open ? (
            <ChevronDown className="h-3.5 w-3.5" />
          ) : (
            <ChevronRight className="h-3.5 w-3.5" />
          )}
        </span>
      </button>

      {open && (
        <div
          id={id}
          className="mt-2 max-h-[50vh] space-y-1 overflow-y-auto pr-0.5 xl:max-h-[calc(100vh-230px)]"
        >
          {/* All Threads Master Button */}
          <button
            type="button"
            className={cn(
              rowClass,
              "w-full justify-between rounded-xl px-2.5 py-2 font-medium transition-all",
              !list.activeProject
                ? "bg-primary text-primary-foreground shadow-xs font-semibold hover:bg-primary/95"
                : "text-foreground hover:bg-muted/80",
            )}
            aria-current={!list.activeProject ? "page" : undefined}
            onClick={() => list.selectProject(null)}
          >
            <span className="flex min-w-0 items-center gap-2">
              <MessagesSquare
                className={cn(
                  "h-3.5 w-3.5 shrink-0",
                  !list.activeProject ? "text-primary-foreground" : "text-primary",
                )}
              />
              <span className="truncate">{t("threads.allProjects")}</span>
            </span>
            <span
              className={cn(
                "rounded-md px-1.5 py-0.2 font-mono text-[10px] tabular-nums font-semibold",
                !list.activeProject
                  ? "bg-primary-foreground/20 text-primary-foreground"
                  : "bg-muted text-muted-foreground",
              )}
            >
              {list.matchingKeys.size === list.threads.length
                ? list.threads.length
                : `${list.matchingKeys.size}/${list.threads.length}`}
            </span>
          </button>

          {/* Project List */}
          <ul className="ml-3 space-y-1 border-l border-border pl-1 pt-1">
            {list.projects.map((project) => (
              <ProjectBranch
                key={project.key}
                project={project}
                list={list}
                repeatedName={(names.get(project.folderName) ?? 0) > 1}
              />
            ))}
          </ul>
        </div>
      )}
    </nav>
  );
}
