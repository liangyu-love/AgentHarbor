import { useTranslation } from "react-i18next";
import { ArrowUpDown, Check, CheckSquare, ChevronDown, ChevronRight, LayoutGrid, List, Search, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { SegmentedOptions } from "@/components/ui/segmented-options";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import type { SortOption, StatusFilter } from "@/components/threads/thread-utils";
import type { ThreadList } from "@/components/threads/use-thread-list";

// Search, status tabs, sorting, view mode, fold and select-all controls.
export function ThreadToolbar({ list }: { list: ThreadList }) {
  const { t } = useTranslation();
  const {
    query, setQuery, threads, archivedCount, status, setStatus, sortBy, setSortBy, sortLabels, viewMode, setViewMode,
    collapsedGroups, expandAllGroups, collapseAllGroups, selectFiltered,
  } = list;

  return (
    <div className="flex flex-wrap items-center justify-between gap-3">
      {/* Left: Search & Filter Tabs */}
      <div className="flex flex-wrap items-center gap-2.5">
        {/* Search Box */}
        <div className="relative w-full sm:w-64">
          <Search className="pointer-events-none absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground/70" />
          <Input
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder={t("threads.searchPlaceholder")}
            className="h-8.5 pl-8 pr-7 text-[13px] bg-background/80"
          />
          {query && (
            <button
              type="button"
              onClick={() => setQuery("")}
              className="absolute right-2 top-1/2 -translate-y-1/2 text-muted-foreground/70 hover:text-foreground"
              aria-label={t("common.cancel")}
            >
              <X className="h-3.5 w-3.5" />
            </button>
          )}
        </div>

        {/* Status Tabs */}
        <SegmentedOptions
          items={[
            { value: "all", label: t("threads.filterAll", { count: threads.length }) },
            {
              value: "active",
              label: t("threads.filterActive", { count: threads.length - archivedCount }),
            },
            { value: "archived", label: t("threads.filterArchived", { count: archivedCount }) },
          ]}
          value={status}
          onChange={(value) => setStatus(value as StatusFilter)}
        />

        {/* Sorting Dropdown */}
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button variant="outline" size="sm" className="h-8.5 gap-1.5 px-2.5 text-xs">
              <ArrowUpDown className="h-3 w-3 text-muted-foreground" />
              <span>{sortLabels[sortBy]}</span>
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="start" className="w-36">
            {(["updatedAt", "messages", "fileSize", "createdAt"] as SortOption[]).map((option) => (
              <DropdownMenuItem
                key={option}
                onClick={() => setSortBy(option)}
                className="flex items-center justify-between text-xs"
              >
                <span>{sortLabels[option]}</span>
                {sortBy === option && <Check className="h-3.5 w-3.5 text-primary" />}
              </DropdownMenuItem>
            ))}
          </DropdownMenuContent>
        </DropdownMenu>
      </div>

      {/* Right: View mode & Folder Fold / Batch Select */}
      <div className="flex items-center gap-2">
        {/* View Mode Switcher */}
        <div className="flex items-center rounded-lg border border-border/70 p-0.5 bg-muted/20">
          <Button
            variant={viewMode === "grid" ? "secondary" : "ghost"}
            size="icon-sm"
            className="h-7 w-7 rounded-md"
            title={t("threads.viewGrid")}
            onClick={() => setViewMode("grid")}
          >
            <LayoutGrid className="h-3.5 w-3.5" />
          </Button>
          <Button
            variant={viewMode === "compact" ? "secondary" : "ghost"}
            size="icon-sm"
            className="h-7 w-7 rounded-md"
            title={t("threads.viewCompact")}
            onClick={() => setViewMode("compact")}
          >
            <List className="h-3.5 w-3.5" />
          </Button>
        </div>

        {/* Fold / Unfold All */}
        <Button
          variant="outline"
          size="sm"
          className="h-8.5 gap-1 text-xs"
          onClick={collapsedGroups.size > 0 ? expandAllGroups : collapseAllGroups}
        >
          {collapsedGroups.size > 0 ? (
            <>
              <ChevronDown className="h-3.5 w-3.5" />
              {t("threads.expandAll")}
            </>
          ) : (
            <>
              <ChevronRight className="h-3.5 w-3.5" />
              {t("threads.collapseAll")}
            </>
          )}
        </Button>

        {/* Select All in filtered */}
        <Button
          variant="outline"
          size="sm"
          className="h-8.5 gap-1.5 text-xs"
          onClick={selectFiltered}
        >
          <CheckSquare className="h-3.5 w-3.5" />
          {t("threads.selectAll")}
        </Button>
      </div>
    </div>
  );
}
