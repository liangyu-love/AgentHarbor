import { useCallback, useEffect, useRef, useState, type KeyboardEvent, type PointerEvent } from "react";
import { Check, ChevronLeft, ChevronRight, ChevronUp, Copy, Expand, Feather, Info, Loader2, Minus, Plus, Scan, X } from "lucide-react";
import type { GalleryMeta } from "@/lib/api";
import { formatDateTime, formatRelative } from "@/lib/format-time";
import { cn } from "@/lib/utils";
import { SaveAsTemplateButton } from "./prompt-template-dialog";
import { qualityLabel, sizeLabel } from "./workbench-types";

/** `scale` 1 is the image fitted to the window; `x`/`y` move its centre in pixels. */
type View = { scale: number; x: number; y: number; smooth: boolean };

const FIT: View = { scale: 1, x: 0, y: 0, smooth: true };
const STEP = 1.4;
const PAN_STEP = 80;
/** Pointer travel that makes a press a pan rather than a click. */
const CLICK_SLOP = 4;
/** Prompts longer than this start folded in the information card. */
const FOLDED_PROMPT_CHARS = 160;

const clamp = (value: number, low: number, high: number) => Math.min(high, Math.max(low, value));

/**
 * Full-window preview: wheel to zoom at the cursor, drag to pan, double-click to toggle,
 * arrows to step through its neighbours, and the prompt that made the image.
 */
export function ImageViewer({ src, alt, onClose, meta, takenAt, position, onStep, loading = false, onUsePrompt }: {
  src: string;
  alt: string;
  onClose: () => void;
  meta?: GalleryMeta | null;
  /** When the image was saved, in milliseconds. */
  takenAt?: number;
  position?: { index: number; total: number };
  onStep?: (step: number) => void;
  /** A neighbour is on its way; the current image stays until it arrives. */
  loading?: boolean;
  onUsePrompt?: (prompt: string) => void;
}) {
  const [view, setView] = useState<View>(FIT);
  // Fitted width to natural width: the scale that shows real pixels one to one.
  const [actual, setActual] = useState(1);
  const [panning, setPanning] = useState(false);
  const stage = useRef<HTMLDivElement>(null);
  const image = useRef<HTMLImageElement>(null);
  const press = useRef<{ id: number; x: number; y: number; from: View; moved: boolean } | null>(null);
  // The click that ends a pan must not close the viewer.
  const suppressClick = useRef(false);
  const maxScale = Math.max(4, actual * 4);
  const canStep = Boolean(onStep && position && position.total > 1);

  /** Keep a zoomed image covering the window, and a fitted one centred. */
  const bounded = useCallback((next: View): View => {
    const img = image.current;
    const box = stage.current?.getBoundingClientRect();
    if (!img || !box || next.scale <= 1) return { ...FIT, smooth: next.smooth };
    const limitX = Math.max(0, (img.offsetWidth * next.scale - box.width) / 2);
    const limitY = Math.max(0, (img.offsetHeight * next.scale - box.height) / 2);
    return { ...next, x: clamp(next.x, -limitX, limitX), y: clamp(next.y, -limitY, limitY) };
  }, []);

  /** Zoom to `target(current scale)`, keeping the point under (clientX, clientY) where it is. */
  const zoom = useCallback((target: (scale: number) => number, smooth: boolean, clientX?: number, clientY?: number) => {
    const box = stage.current?.getBoundingClientRect();
    if (!box) return;
    setView(current => {
      const scale = clamp(target(current.scale), 1, maxScale);
      const px = (clientX ?? box.left + box.width / 2) - (box.left + box.width / 2);
      const py = (clientY ?? box.top + box.height / 2) - (box.top + box.height / 2);
      const ratio = scale / current.scale;
      return bounded({ scale, x: px - (px - current.x) * ratio, y: py - (py - current.y) * ratio, smooth });
    });
  }, [bounded, maxScale]);

  const measure = useCallback(() => {
    const img = image.current;
    if (img?.offsetWidth) setActual(Math.max(1, img.naturalWidth / img.offsetWidth));
    setView(current => bounded(current));
  }, [bounded]);

  useEffect(() => {
    setView(FIT);
  }, [src]);

  useEffect(() => {
    window.addEventListener("resize", measure);
    return () => window.removeEventListener("resize", measure);
  }, [measure]);

  // React's wheel listener is passive; this one must be able to stop the page scrolling.
  useEffect(() => {
    const node = stage.current;
    if (!node) return;
    const onWheel = (event: WheelEvent) => {
      // The information card scrolls its own text.
      if ((event.target as Element).closest("[data-viewer-panel]")) return;
      event.preventDefault();
      zoom(scale => scale * Math.exp(-event.deltaY * 0.0015), false, event.clientX, event.clientY);
    };
    node.addEventListener("wheel", onWheel, { passive: false });
    return () => node.removeEventListener("wheel", onWheel);
  }, [zoom]);

  // Events from a dialog opened here (saving a template) reach this element through
  // React's tree although they happen elsewhere in the page; they are not the viewer's.
  const ownEvent = (event: { target: EventTarget; currentTarget: Element }) =>
    event.currentTarget.contains(event.target as Node);

  const onPointerDown = (event: PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0 || !ownEvent(event) || (event.target as HTMLElement).closest("[data-viewer-toolbar]")) return;
    press.current = { id: event.pointerId, x: event.clientX, y: event.clientY, from: view, moved: false };
  };
  const onPointerMove = (event: PointerEvent<HTMLDivElement>) => {
    const start = press.current;
    if (!start || start.id !== event.pointerId) return;
    const dx = event.clientX - start.x;
    const dy = event.clientY - start.y;
    if (!start.moved && Math.hypot(dx, dy) < CLICK_SLOP) return;
    if (!start.moved) {
      start.moved = true;
      // Captured only once it is a pan, so plain clicks and double-clicks keep their target.
      if (start.from.scale > 1) event.currentTarget.setPointerCapture(event.pointerId);
    }
    if (start.from.scale <= 1) return;
    setPanning(true);
    setView(bounded({ ...start.from, x: start.from.x + dx, y: start.from.y + dy, smooth: false }));
  };
  const onPointerUp = () => {
    suppressClick.current = Boolean(press.current?.moved);
    press.current = null;
    setPanning(false);
  };

  const toggle = (clientX?: number, clientY?: number) =>
    view.scale > 1 ? setView(FIT) : zoom(() => Math.max(2, actual), true, clientX, clientY);

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (!ownEvent(event)) return;
    const pan = (dx: number, dy: number) => setView(current => bounded({ ...current, x: current.x + dx, y: current.y + dy, smooth: true }));
    // Left and right step to a neighbour unless the picture is zoomed in to pan.
    const sideways = (step: number) => (view.scale <= 1 && canStep ? onStep?.(step) : pan(-step * PAN_STEP, 0));
    const actions: Record<string, () => void> = {
      Escape: onClose,
      "+": () => zoom(scale => scale * STEP, true),
      "=": () => zoom(scale => scale * STEP, true),
      "-": () => zoom(scale => scale / STEP, true),
      "0": () => setView(FIT),
      "1": () => zoom(() => actual, true),
      ArrowLeft: () => sideways(-1),
      ArrowRight: () => sideways(1),
      ArrowUp: () => pan(0, PAN_STEP),
      ArrowDown: () => pan(0, -PAN_STEP),
    };
    const action = actions[event.key];
    if (!action) return;
    event.preventDefault();
    action();
  };

  const percent = Math.round((view.scale / actual) * 100);
  const toolButton = "inline-flex h-8 w-8 items-center justify-center rounded-full text-white/80 transition-colors hover:bg-white/15 hover:text-white disabled:opacity-35 disabled:hover:bg-transparent";
  const sideButton = "absolute top-1/2 inline-flex h-11 w-11 -translate-y-1/2 items-center justify-center rounded-full bg-black/55 text-white/80 ring-1 ring-white/10 transition-all hover:bg-black/80 hover:text-white disabled:pointer-events-none disabled:opacity-0";
  return (
    <div
      ref={stage}
      data-image-viewer
      className={cn(
        "fixed inset-0 z-[200] flex touch-none select-none items-center justify-center overflow-hidden bg-black/85 outline-none backdrop-blur-sm animate-in fade-in-0 duration-200",
        view.scale > 1 && (panning ? "cursor-grabbing" : "cursor-grab"),
      )}
      role="dialog"
      aria-modal="true"
      aria-label={alt}
      // Focusable, so a click on the picture keeps the keyboard shortcuts working.
      tabIndex={-1}
      onKeyDown={onKeyDown}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={() => { press.current = null; setPanning(false); }}
      onClick={event => {
        // A plain click outside the picture closes, as before.
        if (suppressClick.current) suppressClick.current = false;
        else if (event.target === event.currentTarget) onClose();
      }}
    >
      <img
        ref={image}
        src={src}
        alt={alt}
        draggable={false}
        onLoad={measure}
        onDoubleClick={event => toggle(event.clientX, event.clientY)}
        className={cn(
          "max-h-[88vh] max-w-[92vw] rounded-xl object-contain shadow-2xl animate-in zoom-in-95 duration-200",
          view.scale <= 1 && "cursor-zoom-in",
        )}
        style={{
          transform: `translate(${view.x}px, ${view.y}px) scale(${view.scale})`,
          transition: view.smooth ? "transform 160ms ease-out" : "none",
        }}
      />

      {meta && <ViewerInfo key={src} meta={meta} takenAt={takenAt} onUsePrompt={onUsePrompt} />}

      {canStep && position && (
        <>
          <button type="button" data-viewer-toolbar className={cn(sideButton, "left-4")} aria-label="上一张" title="上一张（←）"
            disabled={position.index === 0 || loading} onClick={() => onStep?.(-1)}>
            <ChevronLeft className="h-5 w-5" />
          </button>
          <button type="button" data-viewer-toolbar className={cn(sideButton, "right-4")} aria-label="下一张" title="下一张（→）"
            disabled={position.index >= position.total - 1 || loading} onClick={() => onStep?.(1)}>
            <ChevronRight className="h-5 w-5" />
          </button>
        </>
      )}

      {loading && (
        <span role="status" className="absolute left-1/2 top-5 inline-flex -translate-x-1/2 items-center gap-1.5 rounded-full bg-black/65 px-3 py-1.5 text-xs text-white/85 ring-1 ring-white/10">
          <Loader2 className="h-3.5 w-3.5 animate-spin" /> 正在载入
        </span>
      )}

      <button
        type="button"
        autoFocus
        data-viewer-toolbar
        className="absolute right-5 top-5 inline-flex h-9 w-9 items-center justify-center rounded-full bg-black/60 text-white/80 transition-all hover:scale-105 hover:bg-black/80 hover:text-white active:scale-95"
        onClick={onClose}
        aria-label="关闭预览"
      >
        <X className="h-4 w-4" />
      </button>
      <div
        data-viewer-toolbar
        role="toolbar"
        aria-label="缩放"
        className="absolute bottom-5 left-1/2 flex -translate-x-1/2 items-center gap-0.5 rounded-full bg-black/65 px-1.5 py-1 text-white shadow-lg ring-1 ring-white/10"
      >
        <button type="button" className={toolButton} onClick={() => zoom(scale => scale / STEP, true)} disabled={view.scale <= 1}
          aria-label="缩小" title="缩小（-）">
          <Minus className="h-4 w-4" />
        </button>
        <span data-viewer-zoom className="min-w-[3.5rem] text-center font-mono text-xs tabular-nums text-white/85" aria-live="polite">
          {percent}%
        </span>
        <button type="button" className={toolButton} onClick={() => zoom(scale => scale * STEP, true)} disabled={view.scale >= maxScale}
          aria-label="放大" title="放大（+）">
          <Plus className="h-4 w-4" />
        </button>
        <span className="mx-1 h-4 w-px bg-white/20" aria-hidden />
        <button type="button" className={toolButton} onClick={() => setView(FIT)} aria-label="适应窗口" title="适应窗口（0）">
          <Scan className="h-4 w-4" />
        </button>
        <button type="button" className={toolButton} onClick={() => zoom(() => actual, true)} aria-label="原始尺寸" title="原始尺寸（1）">
          <Expand className="h-4 w-4" />
        </button>
        {canStep && position && (
          <span data-viewer-position className="px-2 font-mono text-xs tabular-nums text-white/70">
            {position.index + 1} / {position.total}
          </span>
        )}
        <span className="hidden px-2 font-serif text-[11px] text-white/55 lg:inline">滚轮缩放 · 拖动平移 · 双击切换</span>
      </div>
    </div>
  );
}

/** The prompt and parameters that made the image, with ways to reuse the prompt. */
function ViewerInfo({ meta, takenAt, onUsePrompt }: {
  meta: GalleryMeta;
  takenAt?: number;
  onUsePrompt?: (prompt: string) => void;
}) {
  const [open, setOpen] = useState(true);
  const [unfolded, setUnfolded] = useState(false);
  const [done, setDone] = useState<"used" | "copied" | null>(null);
  const foldable = Array.from(meta.prompt).length > FOLDED_PROMPT_CHARS;
  const flash = (what: "used" | "copied") => {
    setDone(what);
    window.setTimeout(() => setDone(current => (current === what ? null : current)), 1600);
  };
  const chip = "rounded-full bg-white/10 px-2 py-0.5";
  const action = "inline-flex h-7 items-center gap-1 rounded-full bg-white/10 px-2.5 text-[11px] text-white/85 transition-colors hover:bg-white/20 hover:text-white";

  if (!open) {
    return (
      <button type="button" data-viewer-toolbar onClick={() => setOpen(true)}
        className="absolute left-5 top-5 inline-flex h-9 items-center gap-1.5 rounded-full bg-black/60 px-3 text-xs text-white/80 ring-1 ring-white/10 hover:bg-black/80 hover:text-white">
        <Info className="h-3.5 w-3.5" /> 提示词
      </button>
    );
  }
  return (
    <section
      data-viewer-toolbar
      data-viewer-panel
      aria-label="生成信息"
      className="absolute left-5 top-5 max-h-[calc(100vh-8.5rem)] w-[min(26rem,calc(100vw-8rem))] cursor-auto select-text overflow-y-auto rounded-2xl bg-black/70 p-3.5 text-white shadow-2xl ring-1 ring-white/10 backdrop-blur-md"
    >
      <div className="flex items-center justify-between gap-2">
        <span className="font-serif text-xs font-semibold text-white/60">提示词</span>
        <button type="button" onClick={() => setOpen(false)} aria-label="收起生成信息" title="收起"
          className="rounded-full p-1 text-white/60 hover:bg-white/10 hover:text-white">
          <ChevronUp className="h-3.5 w-3.5" />
        </button>
      </div>
      <p data-viewer-prompt className={cn(
        "mt-1 whitespace-pre-wrap break-words font-serif text-sm leading-relaxed text-white/95",
        foldable && !unfolded && "line-clamp-4",
      )}>
        {meta.prompt}
      </p>
      {foldable && (
        <button type="button" onClick={() => setUnfolded(!unfolded)} className="mt-0.5 text-[11px] text-white/55 hover:text-white">
          {unfolded ? "收起" : "展开全文"}
        </button>
      )}
      {meta.revisedPrompt && (
        <details className="mt-2">
          <summary className="cursor-pointer text-[11px] text-white/55 hover:text-white">模型修订后的提示词</summary>
          <p className="mt-1 whitespace-pre-wrap break-words font-serif text-xs leading-relaxed text-white/75">{meta.revisedPrompt}</p>
        </details>
      )}
      <div className="mt-2.5 flex flex-wrap gap-1.5 text-[11px] text-white/70">
        {meta.model && <span className={cn(chip, "font-mono")}>{meta.model}</span>}
        {meta.size && <span className={chip}>{sizeLabel(meta.size)}{meta.size !== sizeLabel(meta.size) && ` · ${meta.size}`}</span>}
        {meta.quality && <span className={chip}>画质 {qualityLabel(meta.quality)}</span>}
        {takenAt !== undefined && <span className={chip} title={formatDateTime(takenAt / 1000)}>{formatRelative(takenAt / 1000)}</span>}
      </div>
      <div className="mt-3 flex flex-wrap items-center gap-1.5">
        {onUsePrompt && (
          <button type="button" className={action} onClick={() => { onUsePrompt(meta.prompt); flash("used"); }}>
            {done === "used" ? <Check className="h-3.5 w-3.5 text-emerald-400" /> : <Feather className="h-3.5 w-3.5" />}
            {done === "used" ? "已填入" : "填入提示词"}
          </button>
        )}
        <button type="button" className={action}
          onClick={() => void navigator.clipboard.writeText(meta.prompt).then(() => flash("copied"), () => undefined)}>
          {done === "copied" ? <Check className="h-3.5 w-3.5 text-emerald-400" /> : <Copy className="h-3.5 w-3.5" />}
          {done === "copied" ? "已复制" : "复制"}
        </button>
        <SaveAsTemplateButton prompt={meta.prompt} className={cn(action, "h-7 font-sans")} />
      </div>
    </section>
  );
}
