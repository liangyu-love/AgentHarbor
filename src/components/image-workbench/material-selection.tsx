import { useEffect, useMemo, useRef, useState, type MouseEvent, type PointerEvent, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { FolderInput, ImageMinus, ImagePlus, Pencil, Trash2 } from "lucide-react";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import type { ImageMaterial, ImageMaterialGroup } from "@/lib/api";

/** Pointer travel before a press on a picture turns into a drag. */
const DRAG_THRESHOLD = 6;

/**
 * Pictures picked for a batch action, in the group's order. Ctrl/⌘-click toggles one,
 * Shift-click takes the run from the last pick; once something is picked, a plain
 * click picks too, until the selection is cleared.
 */
export function useMaterialSelection(items: ImageMaterial[], groupId: string | undefined) {
  const [picked, setPicked] = useState<ReadonlySet<string>>(new Set());
  const anchor = useRef<string | null>(null);

  // A new group starts with nothing picked; pictures that left are dropped from the pick.
  useEffect(() => {
    setPicked(new Set());
    anchor.current = null;
  }, [groupId]);
  useEffect(() => {
    setPicked(current => {
      const kept = new Set([...current].filter(id => items.some(item => item.id === id)));
      return kept.size === current.size ? current : kept;
    });
  }, [items]);

  const ids = useMemo(() => items.filter(item => picked.has(item.id)).map(item => item.id), [items, picked]);

  /** Handle a click as a selection gesture; false when it should do the picture's own action. */
  const click = (event: MouseEvent, item: ImageMaterial) => {
    const toggle = event.ctrlKey || event.metaKey || (picked.size > 0 && !event.shiftKey);
    if (event.shiftKey) {
      const from = items.findIndex(other => other.id === (anchor.current ?? item.id));
      const to = items.findIndex(other => other.id === item.id);
      const [start, end] = from <= to ? [Math.max(from, 0), to] : [to, from];
      setPicked(current => new Set([...current, ...items.slice(start, end + 1).map(other => other.id)]));
      anchor.current = anchor.current ?? item.id;
      return true;
    }
    if (!toggle) return false;
    setPicked(current => {
      const next = new Set(current);
      if (!next.delete(item.id)) next.add(item.id);
      return next;
    });
    anchor.current = item.id;
    return true;
  };

  return {
    ids,
    click,
    has: (id: string) => picked.has(id),
    /** What an action on `item` applies to: the whole pick when it is part of it. */
    targetsOf: (item: ImageMaterial) => (picked.has(item.id) ? ids : [item.id]),
    selectAll: () => setPicked(new Set(items.map(item => item.id))),
    clear: () => {
      setPicked(new Set());
      anchor.current = null;
    },
  };
}

type Drag = { ids: string[]; x: number; y: number; over: string | null };

/**
 * Drag pictures onto a group tab (`data-material-drop` holds its id). Pointer events,
 * as in `usePointerSort`: HTML5 drag and drop does not reach the page in WebView2.
 */
export function useMaterialDrag({ groupId, onDrop }: {
  groupId: string | undefined;
  onDrop: (ids: string[], groupId: string) => void;
}) {
  const [drag, setDrag] = useState<Drag | null>(null);
  const press = useRef<{ ids: string[]; x: number; y: number; pointerId: number } | null>(null);
  const dragged = useRef(false);

  useEffect(() => {
    if (!drag) return;
    const cancel = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      press.current = null;
      setDrag(null);
    };
    window.addEventListener("keydown", cancel);
    return () => window.removeEventListener("keydown", cancel);
  }, [drag]);

  const dropTarget = (x: number, y: number) => {
    const target = document.elementFromPoint(x, y)?.closest<HTMLElement>("[data-material-drop]")?.dataset.materialDrop;
    return target && target !== groupId ? target : null;
  };

  /** Spread onto a picture; `ids` are what it carries when dragged. */
  const handleProps = (ids: () => string[]) => ({
    onPointerDown: (event: PointerEvent<HTMLElement>) => {
      // The click ending a drag does not always arrive; a new press starts clean either way.
      dragged.current = false;
      if (event.button !== 0 || event.shiftKey || event.ctrlKey || event.metaKey) return;
      press.current = { ids: ids(), x: event.clientX, y: event.clientY, pointerId: event.pointerId };
    },
    onPointerMove: (event: PointerEvent<HTMLElement>) => {
      const start = press.current;
      if (!start || start.pointerId !== event.pointerId) return;
      if (!drag && Math.hypot(event.clientX - start.x, event.clientY - start.y) < DRAG_THRESHOLD) return;
      if (!drag) event.currentTarget.setPointerCapture(event.pointerId);
      dragged.current = true;
      setDrag({ ids: start.ids, x: event.clientX, y: event.clientY, over: dropTarget(event.clientX, event.clientY) });
    },
    onPointerUp: (event: PointerEvent<HTMLElement>) => {
      const target = drag ? dropTarget(event.clientX, event.clientY) : null;
      if (drag && target) onDrop(drag.ids, target);
      press.current = null;
      setDrag(null);
    },
    onPointerCancel: () => {
      press.current = null;
      setDrag(null);
    },
  });

  return {
    drag,
    handleProps,
    /** True once for the click that ends a drag, which must not also toggle the picture. */
    consumeClick: () => {
      const was = dragged.current;
      dragged.current = false;
      return was;
    },
  };
}

/** The label that follows the pointer while pictures are dragged. */
export function DragGhost({ drag, groups }: { drag: Drag | null; groups: ImageMaterialGroup[] }) {
  if (!drag) return null;
  const target = groups.find(group => group.id === drag.over);
  return createPortal(
    <div
      aria-hidden
      className="pointer-events-none fixed z-[400] -translate-y-1/2 rounded-md bg-[#b83b38] px-2 py-1 font-serif text-[11px] text-white shadow-lg"
      style={{ left: drag.x + 14, top: drag.y }}
    >
      {drag.ids.length} 张图片{target ? ` → ${target.name}` : " · 拖到上方分组"}
    </div>,
    document.body,
  );
}

/** "移动到" entries: every group but the one the pictures are in. */
function MoveTargets({ groups, groupId, onMove }: {
  groups: ImageMaterialGroup[];
  groupId: string | undefined;
  onMove: (groupId: string) => void;
}) {
  const others = groups.filter(group => group.id !== groupId);
  if (others.length === 0) return <DropdownMenuItem disabled className="font-serif text-xs">没有其他分组</DropdownMenuItem>;
  return others.map(group => (
    <DropdownMenuItem key={group.id} className="font-serif text-xs" onSelect={() => onMove(group.id)}>
      <span className="truncate">{group.name}</span>
      <span className="ml-auto pl-3 font-sans text-[10px] tabular-nums text-muted-foreground">{group.items.length}</span>
    </DropdownMenuItem>
  ));
}

export type MaterialMenuRequest = { x: number; y: number; ids: string[] };

/** The right-click menu of one picture, or of the picked pictures it belongs to. */
export function MaterialMenu({ request, onClose, groups, groupId, items, chosen, onToggle, onEdit, onMove, onDelete }: {
  request: MaterialMenuRequest | null;
  onClose: () => void;
  groups: ImageMaterialGroup[];
  groupId: string | undefined;
  items: ImageMaterial[];
  /** Whether a picture is among the references. */
  chosen: (id: string) => boolean;
  onToggle: (item: ImageMaterial) => void;
  onEdit: (item: ImageMaterial) => void;
  onMove: (ids: string[], groupId: string) => void;
  onDelete: (ids: string[]) => void;
}) {
  const targets = request ? items.filter(item => request.ids.includes(item.id)) : [];
  const single = targets.length === 1 ? targets[0] : null;
  return createPortal(
    <DropdownMenu open={request !== null && targets.length > 0} onOpenChange={open => !open && onClose()} modal={false}>
      <DropdownMenuTrigger asChild>
        <span aria-hidden className="pointer-events-none fixed h-0 w-0" style={{ left: request?.x ?? 0, top: request?.y ?? 0 }} />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" sideOffset={2} className="min-w-44" aria-label="图片操作"
        onCloseAutoFocus={event => event.preventDefault()}>
        <DropdownMenuLabel className="max-w-56 truncate font-serif text-[11px] font-normal text-muted-foreground">
          {single ? single.name : `已选 ${targets.length} 张图片`}
        </DropdownMenuLabel>
        {single && (
          <>
            <DropdownMenuItem className="font-serif text-xs" onSelect={() => onToggle(single)}>
              {chosen(single.id) ? <ImageMinus /> : <ImagePlus />}
              {chosen(single.id) ? "移出参考图" : "加入参考图"}
            </DropdownMenuItem>
            <DropdownMenuItem className="font-serif text-xs" onSelect={() => onEdit(single)}>
              <Pencil /> 改名 / 备注
            </DropdownMenuItem>
          </>
        )}
        <DropdownMenuSub>
          <DropdownMenuSubTrigger className="font-serif text-xs">
            <FolderInput /> 移动到
          </DropdownMenuSubTrigger>
          <DropdownMenuSubContent className="max-h-72 min-w-36 overflow-y-auto">
            <MoveTargets groups={groups} groupId={groupId} onMove={target => onMove(request?.ids ?? [], target)} />
          </DropdownMenuSubContent>
        </DropdownMenuSub>
        <DropdownMenuSeparator />
        <DropdownMenuItem className="font-serif text-xs text-destructive focus:text-destructive"
          onSelect={() => onDelete(request?.ids ?? [])}>
          <Trash2 /> {single ? "删除图片" : `删除 ${targets.length} 张`}
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>,
    document.body,
  );
}

/** A button that opens the "移动到" list, for the picked pictures. */
export function MoveToButton({ groups, groupId, onMove, children }: {
  groups: ImageMaterialGroup[];
  groupId: string | undefined;
  onMove: (groupId: string) => void;
  children: ReactNode;
}) {
  return (
    <DropdownMenu modal={false}>
      <DropdownMenuTrigger asChild>{children}</DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="max-h-72 min-w-36 overflow-y-auto" aria-label="移动到分组">
        <MoveTargets groups={groups} groupId={groupId} onMove={onMove} />
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
