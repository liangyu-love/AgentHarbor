import { useEffect, useMemo, useRef, useState } from "react";
import type { ThreadSummary } from "@/types/threads";
import type { FolderGroup } from "@/components/threads/thread-utils";
import { ThreadCard } from "@/components/threads/thread-card";
import { estimateThreadGridHeight, threadGridStyle } from "./thread-grid-layout";
import {
  GROUP_HEADER_HEIGHT,
  ThreadGroupHeader,
} from "@/components/threads/thread-group-header";

type VirtualRow =
  | {
      type: "group";
      key: string;
      group: FolderGroup;
      collapsed: boolean;
      allSelected: boolean;
    }
  | {
      type: "thread";
      key: string;
      thread: ThreadSummary;
      selected: boolean;
    };

const GROUP_ROW_HEIGHT = GROUP_HEADER_HEIGHT;
const THREAD_ROW_HEIGHT = 52;
const OVERSCAN = 8;

export interface VirtualThreadListProps {
  groups: FolderGroup[];
  collapsedGroups: Set<string>;
  selected: Set<string>;
  detailKey?: string;
  viewMode: "grid" | "compact";
  onToggleSelect: (key: string) => void;
  onToggleGroup: (groupKeys: string[]) => void;
  onToggleCollapse: (groupKey: string) => void;
  onExport: (keys: string[]) => void;
  onOpenPath: (path: string) => void;
  onRecover?: (thread: ThreadSummary) => void;
  onDetails?: (thread: ThreadSummary) => void;
  onDelete?: (thread: ThreadSummary) => void;
  onCopyPath: (path: string, groupKey: string) => void;
  copiedGroupPath: string | null;
  t: (key: string, opts?: Record<string, unknown>) => string;
}

/**
 * Windowed thread list using the page scroll container.
 * Compact mode flattens groups into fixed-height rows;
 * grid mode uses normal-flow groups and skips offscreen work via content-visibility.
 */
export function VirtualThreadList({
  groups,
  collapsedGroups,
  selected,
  detailKey,
  viewMode,
  onToggleSelect,
  onToggleGroup,
  onToggleCollapse,
  onExport,
  onOpenPath,
  onRecover,
  onDetails,
  onDelete,
  onCopyPath,
  copiedGroupPath,
  t,
}: VirtualThreadListProps) {
  const anchorRef = useRef<HTMLDivElement>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [viewportHeight, setViewportHeight] = useState(
    () => (typeof window === "undefined" ? 640 : window.innerHeight),
  );
  const [width, setWidth] = useState(0);

  const rows = useMemo<VirtualRow[]>(() => {
    if (viewMode === "grid") return [];
    const list: VirtualRow[] = [];
    for (const group of groups) {
      const groupKeys = group.items.map(item => item.key);
      list.push({
        type: "group",
        key: `g:${group.key}`,
        group,
        collapsed: collapsedGroups.has(group.key),
        allSelected: groupKeys.length > 0 && groupKeys.every(key => selected.has(key)),
      });
      if (collapsedGroups.has(group.key)) continue;
      for (const thread of group.items) {
        list.push({
          type: "thread",
          key: thread.key,
          thread,
          selected: selected.has(thread.key),
        });
      }
    }
    return list;
  }, [groups, collapsedGroups, selected, viewMode]);

  const rowHeight = (row: VirtualRow) =>
    row.type === "group" ? GROUP_ROW_HEIGHT : THREAD_ROW_HEIGHT;

  const totalHeight = rows.reduce((sum, row) => sum + rowHeight(row), 0);

  const windowRange = useMemo(() => {
    const start = Math.max(0, scrollTop - OVERSCAN * THREAD_ROW_HEIGHT);
    const end = scrollTop + viewportHeight + OVERSCAN * THREAD_ROW_HEIGHT;
    return { start, end };
  }, [scrollTop, viewportHeight]);

  const visibleRows = useMemo(() => {
    let offset = 0;
    const visible: Array<{ row: VirtualRow; offset: number; height: number }> = [];
    for (const row of rows) {
      const height = rowHeight(row);
      const rowStart = offset;
      const rowEnd = offset + height;
      offset = rowEnd;
      if (rowEnd < windowRange.start || rowStart > windowRange.end) continue;
      visible.push({ row, offset: rowStart, height });
    }
    return visible;
  }, [rows, windowRange]);

  useEffect(() => {
    const anchor = anchorRef.current;
    if (!anchor) return;
    const container = anchor.closest<HTMLElement>("[data-page-scroll-container]");
    const scrollTarget = container ?? window;
    const measure = () => {
      if (viewMode === "grid") {
        setWidth(anchor.clientWidth);
        return;
      }
      const viewportTop = container
        ? container.getBoundingClientRect().top + container.clientTop
        : 0;
      setScrollTop(Math.max(0, viewportTop - anchor.getBoundingClientRect().top));
      setViewportHeight(container?.clientHeight ?? window.innerHeight);
    };
    measure();
    if (viewMode === "compact") scrollTarget.addEventListener("scroll", measure, { passive: true });
    window.addEventListener("resize", measure);
    container?.addEventListener("transitionend", measure);
    const observer = new ResizeObserver(measure);
    observer.observe(container ?? document.documentElement);
    observer.observe(anchor);
    return () => {
      scrollTarget.removeEventListener("scroll", measure);
      window.removeEventListener("resize", measure);
      container?.removeEventListener("transitionend", measure);
      observer.disconnect();
    };
  }, [viewMode, groups, collapsedGroups]);

  return (
    <div
      ref={anchorRef}
      data-thread-list
      className="relative"
      style={viewMode === "compact" ? { height: totalHeight } : undefined}
    >
      {viewMode === "grid" ? (
        <div className="space-y-3">
          {groups.map(group => {
            const collapsed = collapsedGroups.has(group.key);
            const groupKeys = group.items.map(item => item.key);
            const groupExportKeys = group.items
              .filter(item => item.rolloutStatus !== "missing")
              .map(item => item.key);
            const allInGroupSelected =
              groupKeys.length > 0 && groupKeys.every(key => selected.has(key));
            return (
              <div
                key={group.key}
                data-thread-group={group.key}
              >
                <ThreadGroupHeader
                  group={group}
                  collapsed={collapsed}
                  allSelected={allInGroupSelected}
                  compact
                  onToggleCollapse={() => onToggleCollapse(group.key)}
                  onToggleGroup={() => onToggleGroup(groupKeys)}
                  onExport={() => onExport(groupExportKeys)}
                  exportDisabled={groupExportKeys.length === 0}
                  onOpenPath={() => onOpenPath(group.cwd)}
                  onCopyPath={() => onCopyPath(group.cwd, group.key)}
                  copiedGroupPath={copiedGroupPath}
                  t={t}
                />
                {!collapsed && (
                  <div
                    data-thread-grid
                    className="mt-2 grid"
                    style={{
                      ...threadGridStyle,
                      contentVisibility: "auto",
                      containIntrinsicSize: `auto ${estimateThreadGridHeight(group.items.length, width)}px`,
                    }}
                  >
                    {group.items.map(thread => (
                      <ThreadCard
                        key={thread.key}
                        thread={thread}
                        selected={selected.has(thread.key)}
                        detailActive={detailKey === thread.key}
                        onToggleSelect={() => onToggleSelect(thread.key)}
                        onExport={() => onExport([thread.key])}
                        onOpenRaw={() => onOpenPath(thread.filePath)}
                        onRecover={onRecover ? () => onRecover(thread) : undefined}
                        onDetails={() => onDetails?.(thread)}
                        onDelete={onDelete ? () => onDelete(thread) : undefined}
                        variant="grid"
                      />
                    ))}
                  </div>
                )}
              </div>
            );
          })}
        </div>
      ) : (
        <>
          {visibleRows.map(item => {
            const row = item.row;
            if (row.type === "group") {
              const group = row.group;
              const groupKeys = group.items.map(thread => thread.key);
              const groupExportKeys = group.items
                .filter(thread => thread.rolloutStatus !== "missing")
                .map(thread => thread.key);
              return (
                <div
                  key={row.key}
                  style={{ position: "absolute", top: item.offset, left: 0, right: 0, height: item.height }}
                >
                  <ThreadGroupHeader
                    group={group}
                    collapsed={row.collapsed}
                    allSelected={row.allSelected}
                    compact
                    onToggleCollapse={() => onToggleCollapse(group.key)}
                    onToggleGroup={() => onToggleGroup(groupKeys)}
                    onExport={() => onExport(groupExportKeys)}
                    exportDisabled={groupExportKeys.length === 0}
                    onOpenPath={() => onOpenPath(group.cwd)}
                    onCopyPath={() => onCopyPath(group.cwd, group.key)}
                    copiedGroupPath={copiedGroupPath}
                    t={t}
                  />
                </div>
              );
            }
            return (
              <div
                key={row.key}
                style={{ position: "absolute", top: item.offset, left: 0, right: 0, height: item.height }}
                className="overflow-hidden"
              >
                <ThreadCard
                  thread={row.thread}
                  selected={row.selected}
                  detailActive={detailKey === row.thread.key}
                  onToggleSelect={() => onToggleSelect(row.thread.key)}
                  onExport={() => onExport([row.thread.key])}
                  onOpenRaw={() => onOpenPath(row.thread.filePath)}
                  onRecover={onRecover ? () => onRecover(row.thread) : undefined}
                  onDetails={() => onDetails?.(row.thread)}
                  onDelete={onDelete ? () => onDelete(row.thread) : undefined}
                  variant="compact"
                />
              </div>
            );
          })}
        </>
      )}
    </div>
  );
}
