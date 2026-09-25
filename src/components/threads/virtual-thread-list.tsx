import { useEffect, useMemo, useRef, useState } from "react";
import type { ThreadSummary } from "@/types/threads";
import type { FolderGroup } from "@/components/threads/thread-utils";
import { ThreadCard } from "@/components/threads/thread-card";
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
const THREAD_ROW_HEIGHT = 40;
const OVERSCAN = 8;

function estimateGridColumns() {
  if (typeof window === "undefined") return 3;
  const width = window.innerWidth;
  if (width >= 1536) return 5;
  if (width >= 1280) return 4;
  if (width >= 768) return 3;
  return width >= 640 ? 2 : 1;
}

function estimateGridBodyHeight(itemCount: number, columns: number) {
  if (itemCount <= 0) return 0;
  const rows = Math.ceil(itemCount / columns);
  // Card target height + gap, with slack for provider/model/source badges.
  return rows * 132 + Math.max(0, rows - 1) * 8 + 8;
}

export interface VirtualThreadListProps {
  groups: FolderGroup[];
  collapsedGroups: Set<string>;
  selected: Set<string>;
  viewMode: "grid" | "compact";
  onToggleSelect: (key: string) => void;
  onToggleGroup: (groupKeys: string[]) => void;
  onToggleCollapse: (groupKey: string) => void;
  onExport: (keys: string[]) => void;
  onOpenPath: (path: string) => void;
  onRecover: (thread: ThreadSummary) => void;
  onCopyPath: (path: string, groupKey: string) => void;
  copiedGroupPath: string | null;
  t: (key: string, opts?: Record<string, unknown>) => string;
}

/**
 * Windowed thread list using the page scroll container.
 * Compact mode flattens groups into fixed-height rows;
 * grid mode keeps group blocks and skips offscreen paint via content-visibility.
 */
export function VirtualThreadList({
  groups,
  collapsedGroups,
  selected,
  viewMode,
  onToggleSelect,
  onToggleGroup,
  onToggleCollapse,
  onExport,
  onOpenPath,
  onRecover,
  onCopyPath,
  copiedGroupPath,
  t,
}: VirtualThreadListProps) {
  const anchorRef = useRef<HTMLDivElement>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [viewportHeight, setViewportHeight] = useState(
    () => (typeof window === "undefined" ? 640 : window.innerHeight),
  );
  const [columns, setColumns] = useState(() => estimateGridColumns());

  const rows = useMemo<VirtualRow[]>(() => {
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
      if (collapsedGroups.has(group.key) || viewMode === "grid") continue;
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

  const gridLayout = useMemo(() => {
    let offset = 0;
    const blocks = groups.map(group => {
      const collapsed = collapsedGroups.has(group.key);
      const bodyHeight = collapsed ? 0 : estimateGridBodyHeight(group.items.length, columns);
      const blockHeight = GROUP_ROW_HEIGHT + bodyHeight + 12;
      const block = { group, collapsed, bodyHeight, blockHeight, offset };
      offset += blockHeight;
      return block;
    });
    return { blocks, totalHeight: offset };
  }, [groups, collapsedGroups, columns]);

  const totalHeight = viewMode === "grid"
    ? gridLayout.totalHeight
    : rows.reduce((sum, row) => sum + rowHeight(row), 0);

  const windowRange = useMemo(() => {
    const start = Math.max(0, scrollTop - OVERSCAN * THREAD_ROW_HEIGHT);
    const end = scrollTop + viewportHeight + OVERSCAN * THREAD_ROW_HEIGHT;
    return { start, end };
  }, [scrollTop, viewportHeight]);

  const visibleRows = useMemo(() => {
    if (viewMode === "grid") return [] as Array<{ row: VirtualRow; offset: number; height: number }>;
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
  }, [rows, windowRange, viewMode]);

  useEffect(() => {
    const anchor = anchorRef.current;
    if (!anchor) return;
    const container = anchor.closest<HTMLElement>("[data-page-scroll-container]");
    const scrollTarget = container ?? window;
    const measure = () => {
      const viewportTop = container
        ? container.getBoundingClientRect().top + container.clientTop
        : 0;
      setScrollTop(Math.max(0, viewportTop - anchor.getBoundingClientRect().top));
      setViewportHeight(container?.clientHeight ?? window.innerHeight);
      setColumns(estimateGridColumns());
    };
    measure();
    scrollTarget.addEventListener("scroll", measure, { passive: true });
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
    <div ref={anchorRef} className="relative" style={{ height: totalHeight }}>
      {viewMode === "grid" ? (
        <div className="space-y-3">
          {gridLayout.blocks.map(({ group, collapsed, bodyHeight, blockHeight, offset }) => {
            const groupKeys = group.items.map(item => item.key);
            const groupExportKeys = group.items
              .filter(item => item.rolloutStatus !== "missing")
              .map(item => item.key);
            const allInGroupSelected =
              groupKeys.length > 0 && groupKeys.every(key => selected.has(key));
            const visible =
              offset + blockHeight >= windowRange.start && offset <= windowRange.end;
            return (
              <div
                key={group.key}
                style={{
                  position: "absolute",
                  top: offset,
                  left: 0,
                  right: 0,
                  minHeight: blockHeight,
                  visibility: visible ? "visible" : "hidden",
                }}
                aria-hidden={!visible}
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
                    className="mt-1.5 grid gap-2 sm:grid-cols-2 md:grid-cols-3 xl:grid-cols-4 2xl:grid-cols-5"
                    style={{
                      contentVisibility: "auto",
                      containIntrinsicSize: `${Math.max(bodyHeight, 132)}px`,
                    }}
                  >
                    {group.items.map(thread => (
                      <ThreadCard
                        key={thread.key}
                        thread={thread}
                        selected={selected.has(thread.key)}
                        onToggleSelect={() => onToggleSelect(thread.key)}
                        onExport={() => onExport([thread.key])}
                        onOpenRaw={() => onOpenPath(thread.filePath)}
                        onRecover={() => onRecover(thread)}
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
                  onToggleSelect={() => onToggleSelect(row.thread.key)}
                  onExport={() => onExport([row.thread.key])}
                  onOpenRaw={() => onOpenPath(row.thread.filePath)}
                  onRecover={() => onRecover(row.thread)}
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
