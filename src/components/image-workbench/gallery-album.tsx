import { useEffect, useRef, useState, type CSSProperties, type KeyboardEvent, type PointerEvent, type WheelEvent } from "react";
import { useTranslation } from "react-i18next";
import { ChevronLeft, ChevronRight, Download, Expand, Feather, ImageUp, Loader2, Pause, Play, Trash2 } from "lucide-react";
import type { GalleryItem } from "@/lib/api";
import { formatDateTime, formatRelative } from "@/lib/format-time";
import { cn } from "@/lib/utils";
import { itemRatio, useGallerySource, useNearViewport } from "./gallery-thumbnail";
import type { ImageGallery } from "./use-image-gallery";
import { qualityLabel, sizeLabel } from "./workbench-types";

/** The stage asks for this edge: sharp at its full height, far lighter than an original. */
const STAGE_EDGE = 1600;
/** How long one image stays on air while the album plays. */
const DWELL_MS = 6500;
/** Room the counter above and the lower third below keep clear of the image. */
const TOP_ROOM = 56;
const BOTTOM_ROOM = 112;
const GAP = 22;
/** Drag past this, or flick faster than FLICK px/ms, and the album steps. */
const SWIPE = 60;
const FLICK = 0.45;
/** Each neighbour shrinks and dims the further it sits from the one on air. */
const SCALES = [1, 0.74, 0.56, 0.5];
const reducedMotion = () => window.matchMedia("(prefers-reduced-motion: reduce)").matches;

function useSize(ref: React.RefObject<HTMLElement | null>) {
  const [size, setSize] = useState({ width: 0, height: 0 });
  useEffect(() => {
    const element = ref.current;
    if (!element) return;
    const observer = new ResizeObserver(([entry]) => setSize({ width: entry.contentRect.width, height: entry.contentRect.height }));
    observer.observe(element);
    return () => observer.disconnect();
  }, [ref]);
  return size;
}

/**
 * The album: one image on air at a time with its neighbours waiting in the wings,
 * a blurred wash of its colours behind, and a broadcast lower third for what made it.
 * Drag, flick, swipe sideways, use the arrows, or let it play.
 */
export function GalleryAlbum({ items, gallery, onUsePrompt, onUseAsReference }: {
  items: GalleryItem[];
  gallery: ImageGallery;
  onUsePrompt: (prompt: string) => void;
  onUseAsReference: (item: GalleryItem) => void;
}) {
  const { t } = useTranslation();
  const stage = useRef<HTMLDivElement>(null);
  const strip = useRef<HTMLDivElement>(null);
  const { width, height } = useSize(stage);
  const visible = useNearViewport(stage);
  // The album follows a file, so filtering or deleting keeps the place.
  const [at, setAt] = useState<{ path: string | null; index: number }>({ path: null, index: 0 });
  const found = at.path ? items.findIndex(item => item.path === at.path) : -1;
  const index = found >= 0 ? found : Math.min(at.index, items.length - 1);
  const current = items[index];
  const [playing, setPlaying] = useState(() => !reducedMotion());
  const [drag, setDrag] = useState(0);
  const press = useRef<{ id: number; x: number; time: number; moved: boolean } | null>(null);
  const suppressClick = useRef(false);
  const wheel = useRef({ sum: 0, until: 0 });
  const loadStage = (item: GalleryItem) => gallery.loadGalleryThumbnail(item, STAGE_EDGE);

  const go = (next: number) => {
    if (!items.length) return;
    const wrapped = (next + items.length) % items.length;
    setAt({ path: items[wrapped].path, index: wrapped });
  };
  const open = () => current && gallery.openGalleryImage(current, items);

  // Keep the film strip's active frame in the middle without moving the page.
  useEffect(() => {
    const frame = strip.current?.querySelector<HTMLElement>("[data-active]");
    const box = strip.current;
    if (!frame || !box) return;
    box.scrollTo({ left: frame.offsetLeft - box.clientWidth / 2 + frame.offsetWidth / 2, behavior: reducedMotion() ? "auto" : "smooth" });
  }, [index]);

  if (!current) return null;

  // Every slide fits the room left between the counter and the lower third.
  const narrow = width < 640;
  const maxWidth = width * (narrow ? 0.84 : 0.6);
  const maxHeight = Math.max(120, height - TOP_ROOM - BOTTOM_ROOM);
  const box = (item: GalleryItem) => {
    const ratio = itemRatio(item) ?? 1;
    const w = Math.min(maxWidth, maxHeight * ratio);
    return { w, h: w / ratio };
  };
  // Lay the wings out from the centre: each neighbour sits a gap beyond the last.
  const slides: { item: GalleryItem; offset: number; x: number; w: number; h: number }[] = [];
  const centre = box(current);
  slides.push({ item: current, offset: 0, x: 0, ...centre });
  for (const side of [-1, 1]) {
    let edge = centre.w / 2;
    for (let distance = 1; distance <= 3; distance += 1) {
      const item = items[index + side * distance];
      if (!item) break;
      const size = box(item);
      const half = size.w * SCALES[distance] / 2;
      slides.push({ item, offset: side * distance, x: side * (edge + GAP + half), ...size });
      edge += GAP + half * 2;
    }
  }
  const edgePull = (index === 0 && drag > 0) || (index === items.length - 1 && drag < 0) ? 0.35 : 1;
  const dx = drag * edgePull;

  const onPointerDown = (event: PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0 || (event.target as HTMLElement).closest("[data-album-control]")) return;
    press.current = { id: event.pointerId, x: event.clientX, time: performance.now(), moved: false };
  };
  const onPointerMove = (event: PointerEvent<HTMLDivElement>) => {
    const state = press.current;
    if (!state || state.id !== event.pointerId) return;
    const distance = event.clientX - state.x;
    if (!state.moved && Math.abs(distance) > 6) {
      state.moved = true;
      event.currentTarget.setPointerCapture(event.pointerId);
    }
    if (state.moved) setDrag(distance);
  };
  const onPointerUp = (event: PointerEvent<HTMLDivElement>) => {
    const state = press.current;
    press.current = null;
    if (!state || state.id !== event.pointerId || !state.moved) return;
    const distance = event.clientX - state.x;
    const speed = distance / Math.max(1, performance.now() - state.time);
    setDrag(0);
    suppressClick.current = true;
    requestAnimationFrame(() => { suppressClick.current = false; });
    if ((distance < -SWIPE || speed < -FLICK) && index < items.length - 1) go(index + 1);
    else if ((distance > SWIPE || speed > FLICK) && index > 0) go(index - 1);
  };
  // Sideways trackpad swipes step once per gesture; vertical wheels keep scrolling the page.
  const onWheel = (event: WheelEvent<HTMLDivElement>) => {
    if (Math.abs(event.deltaX) <= Math.abs(event.deltaY)) return;
    const now = performance.now();
    if (now < wheel.current.until) return;
    wheel.current.sum += event.deltaX;
    if (Math.abs(wheel.current.sum) < 40) return;
    const step = Math.sign(wheel.current.sum);
    wheel.current = { sum: 0, until: now + 420 };
    if (index + step >= 0 && index + step < items.length) go(index + step);
  };
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.target !== event.currentTarget) return;
    const next = event.key === "ArrowLeft" ? index - 1 : event.key === "ArrowRight" ? index + 1
      : event.key === "Home" ? 0 : event.key === "End" ? items.length - 1 : null;
    if (next !== null) {
      event.preventDefault();
      if (next >= 0 && next < items.length) go(next);
    } else if (event.key === " ") {
      event.preventDefault();
      setPlaying(!playing);
    } else if (event.key === "Enter") {
      event.preventDefault();
      open();
    }
  };

  const meta = current.meta;
  const prompt = meta?.prompt;
  const control = "gallery-album-control";
  return (
    <div className="space-y-3" data-gallery-album>
      <div ref={stage} className="gallery-album" tabIndex={0} role="region" aria-roledescription="carousel"
        aria-label={t("image.album.label")} data-dragging={drag !== 0 || undefined} data-playing={playing || undefined}
        onPointerDown={onPointerDown} onPointerMove={onPointerMove} onPointerUp={onPointerUp} onPointerCancel={onPointerUp}
        onWheel={onWheel} onKeyDown={onKeyDown}>
        <AlbumBackdrop key={current.path} item={current} visible={visible} load={gallery.loadGalleryThumbnail} />

        <div className="gallery-album-track" aria-hidden>
          {slides.map(slide => {
            const distance = Math.abs(slide.offset);
            const style = {
              width: slide.w, height: slide.h,
              transform: `translate(-50%, -50%) translateX(${slide.x + dx}px) scale(${SCALES[distance]}) rotateY(${-Math.sign(slide.offset) * Math.min(distance, 2) * 7}deg)`,
              zIndex: 10 - distance,
              opacity: distance === 3 ? 0 : distance === 2 ? 0.45 : 1,
              top: TOP_ROOM + maxHeight / 2,
            } as CSSProperties;
            return <button key={slide.item.path} type="button" tabIndex={-1} className="gallery-album-slide" style={style}
              data-on-air={slide.offset === 0 || undefined}
              onClick={() => {
                if (suppressClick.current) return;
                if (slide.offset === 0) open(); else go(index + slide.offset);
              }}>
              <AlbumImage item={slide.item} visible={visible && distance <= 2} sharp={distance <= 1}
                load={gallery.loadGalleryThumbnail} loadStage={loadStage}
                drift={slide.offset === 0 ? (index % 2 ? "right" : "left") : undefined} />
              {slide.offset === 0 && gallery.galleryPreviewPath === slide.item.path &&
                <Loader2 className="absolute left-1/2 top-1/2 h-6 w-6 -translate-x-1/2 -translate-y-1/2 animate-spin text-white" />}
            </button>;
          })}
        </div>

        <div className="gallery-album-top">
          <span className="gallery-album-onair" data-playing={playing || undefined}>
            <i aria-hidden />{playing ? t("image.album.playing") : t("image.album.paused")}
          </span>
          <span className="gallery-album-count poster-number" aria-live="polite">
            <strong>{String(index + 1).padStart(2, "0")}</strong>
            <span>/ {String(items.length).padStart(2, "0")}</span>
          </span>
          <button type="button" data-album-control className={cn(control, "ml-auto")} onClick={() => setPlaying(!playing)}
            aria-label={playing ? t("image.album.pause") : t("image.album.play")} title={playing ? t("image.album.pause") : t("image.album.play")}>
            {playing ? <Pause className="h-3.5 w-3.5" /> : <Play className="h-3.5 w-3.5" />}
          </button>
        </div>
        <div className="gallery-album-progress" aria-hidden>
          <i key={`${current.path}-${playing}`} data-running={(playing && visible && items.length > 1) || undefined}
            style={{ animationDuration: `${DWELL_MS}ms` }} onAnimationEnd={() => go(index + 1)} />
        </div>

        {index > 0 && <button type="button" data-album-control className={cn(control, "gallery-album-arrow left-3")} style={{ top: TOP_ROOM + maxHeight / 2 }}
          onClick={() => go(index - 1)} aria-label={t("image.album.previous")}><ChevronLeft className="h-5 w-5" /></button>}
        {index < items.length - 1 && <button type="button" data-album-control className={cn(control, "gallery-album-arrow right-3")} style={{ top: TOP_ROOM + maxHeight / 2 }}
          onClick={() => go(index + 1)} aria-label={t("image.album.next")}><ChevronRight className="h-5 w-5" /></button>}

        <div className="gallery-album-lower">
          <div key={current.path} className="gallery-album-caption">
            <span className="gallery-album-kicker">
              <span title={formatDateTime(current.modifiedAt / 1000)}>{formatRelative(current.modifiedAt / 1000)}</span>
              {meta?.model && <span className="font-mono">{meta.model}</span>}
              {meta?.size && <span>{sizeLabel(meta.size)}</span>}
              {meta?.quality && <span>{t("image.album.quality", { quality: qualityLabel(meta.quality) })}</span>}
            </span>
            <p className={cn(prompt ? "font-serif" : "font-mono text-[12px]")} title={prompt ?? current.name}>{prompt ?? current.name}</p>
          </div>
          <div className="gallery-album-actions" data-album-control>
            {prompt && <button type="button" className={control} onClick={() => onUsePrompt(prompt)}
              title="填入提示词" aria-label="填入提示词"><Feather className="h-3.5 w-3.5" /></button>}
            <button type="button" className={control} onClick={() => onUseAsReference(current)} title="用作底本" aria-label="用作底本">
              <ImageUp className="h-3.5 w-3.5" />
            </button>
            <button type="button" className={control} onClick={open} title={t("image.album.open")} aria-label={t("image.album.open")}>
              <Expand className="h-3.5 w-3.5" />
            </button>
            <button type="button" className={control} title={t("image.saveAs")} aria-label={t("image.saveAs")}
              onClick={() => void gallery.saveGalleryImageAs(current, gallery.galleryItems.indexOf(current))}>
              <Download className="h-3.5 w-3.5" />
            </button>
            <button type="button" className={cn(control, "hover:!bg-destructive")} title={t("image.galleryDelete")} aria-label={t("image.galleryDelete")}
              onClick={() => gallery.setPendingDelete(current)}>
              <Trash2 className="h-3.5 w-3.5" />
            </button>
          </div>
        </div>
      </div>

      <div ref={strip} className="gallery-album-strip" role="group" aria-label={t("image.album.strip")}>
        {items.map((item, position) => <FilmFrame key={item.path} item={item} active={position === index}
          label={t("image.album.goTo", { index: position + 1 })} load={gallery.loadGalleryThumbnail} onSelect={() => go(position)} />)}
      </div>
    </div>
  );
}

/** The on-air image's own colours, blurred into the stage behind it. */
function AlbumBackdrop({ item, visible, load }: { item: GalleryItem; visible: boolean; load: (item: GalleryItem) => Promise<string> }) {
  const { src } = useGallerySource(item, visible, load);
  return src ? <img src={src} alt="" aria-hidden className="gallery-album-backdrop" draggable={false} /> : null;
}

/** The grid thumbnail at once, then the sharper stage copy fading in over it. */
function AlbumImage({ item, visible, sharp, load, loadStage, drift }: {
  item: GalleryItem;
  visible: boolean;
  sharp: boolean;
  load: (item: GalleryItem) => Promise<string>;
  loadStage: (item: GalleryItem) => Promise<string>;
  drift?: "left" | "right";
}) {
  const small = useGallerySource(item, visible, load);
  const large = useGallerySource(item, visible && sharp, loadStage);
  const [ready, setReady] = useState(false);
  useEffect(() => setReady(false), [large.src]);
  return <span className="gallery-album-image" data-drift={drift}>
    {small.src && <img src={small.src} alt={item.name} draggable={false} />}
    {large.src && <img src={large.src} alt="" aria-hidden draggable={false} data-ready={ready || undefined} onLoad={() => setReady(true)} />}
  </span>;
}

/** One frame of the film strip, loaded only once it scrolls near. */
function FilmFrame({ item, active, label, load, onSelect }: {
  item: GalleryItem;
  active: boolean;
  label: string;
  load: (item: GalleryItem) => Promise<string>;
  onSelect: () => void;
}) {
  const anchor = useRef<HTMLButtonElement>(null);
  const visible = useNearViewport(anchor);
  const { src } = useGallerySource(item, visible, load);
  return <button ref={anchor} type="button" className="gallery-album-frame" style={{ aspectRatio: itemRatio(item) ?? 1 }}
    data-active={active || undefined} aria-current={active || undefined} aria-label={label} title={item.meta?.prompt ?? item.name} onClick={onSelect}>
    {src && <img src={src} alt="" draggable={false} />}
  </button>;
}
