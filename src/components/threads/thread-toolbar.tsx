import { useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import {
  AlertTriangle, ArrowUpDown, Check, CheckSquare, ChevronDown, ChevronsDownUp, ChevronsUpDown, LayoutGrid, List, Search, X,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { SegmentedOptions } from "@/components/ui/segmented-options";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { cn } from "@/lib/utils";
import type { SortOption, StatusFilter } from "@/components/threads/thread-utils";
import type { ExportFormat, ThreadList } from "@/components/threads/use-thread-list";

const tool = "h-8 gap-1.5 rounded-xl bg-background/80 px-2.5 text-xs";

// Search first; filters, ordering and view controls sit on one line below it.
export function ThreadToolbar({ list }: { list: ThreadList }) {
  const { t } = useTranslation();
  const {
    query, setQuery, scopedThreads: threads, archivedCount, status, setStatus, sortBy, setSortBy, sortLabels, viewMode, setViewMode,
    collapsedGroups, expandAllGroups, collapseAllGroups, selectFiltered, misplacedCount, onlyMisplaced, setOnlyMisplaced,
  } = list;
  const search = useRef<HTMLInputElement>(null);

  // "/" jumps to the search box from anywhere on the page.
  useEffect(() => {
    const focus = (event: KeyboardEvent) => {
      const target = event.target as HTMLElement | null;
      if (event.key !== "/" || event.ctrlKey || event.metaKey || event.altKey) return;
      if (target?.closest("input, textarea, select, [contenteditable=true]")) return;
      event.preventDefault();
      search.current?.focus();
    };
    window.addEventListener("keydown", focus);
    return () => window.removeEventListener("keydown", focus);
  }, []);

  return (
    <div className="relative px-5 pb-4 pt-4 sm:px-6">
      <label className="group/search relative flex h-11 items-center">
        <Search aria-hidden className="pointer-events-none absolute left-3.5 h-[18px] w-[18px] text-muted-foreground transition-colors group-focus-within/search:text-primary" />
        <input ref={search} value={query} onChange={(event) => setQuery(event.target.value)}
          placeholder={t("threads.searchPlaceholder")}
          className="h-11 w-full rounded-2xl border border-border bg-background/85 pl-11 pr-16 text-sm shadow-sm outline-none transition-[border-color,box-shadow] placeholder:text-muted-foreground/70 focus:border-primary/50 focus:ring-4 focus:ring-primary/10" />
        {query ? (
          <button type="button" onClick={() => setQuery("")} aria-label={t("common.cancel")}
            className="absolute right-3 rounded-lg p-1 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground">
            <X className="h-4 w-4" />
          </button>
        ) : (
          <kbd aria-hidden className="pointer-events-none absolute right-3.5 rounded-md border border-border bg-muted/60 px-1.5 font-mono text-[11px] text-muted-foreground">/</kbd>
        )}
      </label>

      <div className="mt-3 flex flex-wrap items-center gap-2">
        {list.isCodex && <SegmentedOptions
          items={[
            { value: "all", label: t("threads.filterAll", { count: threads.length }) },
            { value: "active", label: t("threads.filterActive", { count: threads.length - archivedCount }) },
            { value: "archived", label: t("threads.filterArchived", { count: archivedCount }) },
          ]}
          value={status}
          onChange={(value) => setStatus(value as StatusFilter)}
        />}

        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button variant="outline" size="sm" className={tool}>
              <ArrowUpDown className="h-3 w-3 text-muted-foreground" />
              <span>{sortLabels[sortBy]}</span>
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="start" className="w-36">
            {(["updatedAt", "messages", "fileSize", "createdAt"] as SortOption[]).map((option) => (
              <DropdownMenuItem key={option} onClick={() => setSortBy(option)} className="flex items-center justify-between text-xs">
                <span>{sortLabels[option]}</span>
                {sortBy === option && <Check className="h-3.5 w-3.5 text-primary" />}
              </DropdownMenuItem>
            ))}
          </DropdownMenuContent>
        </DropdownMenu>

        {misplacedCount > 0 && (
          <button type="button" aria-pressed={onlyMisplaced} onClick={() => setOnlyMisplaced(!onlyMisplaced)}
            className={cn("inline-flex h-8 items-center gap-1.5 rounded-xl border px-2.5 text-xs font-medium transition-colors",
              onlyMisplaced ? "border-orange-500/50 bg-orange-500/10 text-orange-700 dark:text-orange-300"
                : "border-border bg-background/80 text-muted-foreground hover:text-foreground")}>
            <AlertTriangle className="h-3.5 w-3.5 text-orange-500" />
            {t("threads.misplacedFilter", { count: misplacedCount })}
          </button>
        )}

        <div className="ml-auto flex flex-wrap items-center gap-2">
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button variant="outline" size="sm" className={tool} aria-label={t("threads.exportFormat")}>
                {t("threads.exportFormat")}: {t(list.format === "markdown" ? "threads.formatMarkdown" : "threads.formatJsonl")}
                <ChevronDown className="h-3 w-3" />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end">
              {(["markdown", "jsonl"] as ExportFormat[]).map(format => <DropdownMenuItem key={format} onClick={() => list.setFormat(format)}>
                {t(format === "markdown" ? "threads.formatMarkdown" : "threads.formatJsonl")}
                {list.format === format && <Check className="ml-auto h-3.5 w-3.5" />}
              </DropdownMenuItem>)}
            </DropdownMenuContent>
          </DropdownMenu>
          <div className="flex items-center rounded-xl border border-border bg-background/80 p-0.5">
            {([["grid", LayoutGrid, "threads.viewGrid"], ["compact", List, "threads.viewCompact"]] as const).map(([mode, Icon, label]) => (
              <Button key={mode} variant={viewMode === mode ? "secondary" : "ghost"} size="icon-sm" className="h-7 w-7 rounded-lg"
                title={t(label)} aria-label={t(label)} aria-pressed={viewMode === mode} onClick={() => setViewMode(mode)}>
                <Icon className="h-3.5 w-3.5" />
              </Button>
            ))}
          </div>
          <Button variant="outline" size="sm" className={tool}
            onClick={collapsedGroups.size > 0 ? expandAllGroups : collapseAllGroups}>
            {collapsedGroups.size > 0
              ? <><ChevronsUpDown className="h-3.5 w-3.5" />{t("threads.expandAll")}</>
              : <><ChevronsDownUp className="h-3.5 w-3.5" />{t("threads.collapseAll")}</>}
          </Button>
          <Button variant="outline" size="sm" className={tool} onClick={selectFiltered}>
            <CheckSquare className="h-3.5 w-3.5" />
            {t("threads.selectAll")}
          </Button>
        </div>
      </div>
    </div>
  );
}
