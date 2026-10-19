import { useEffect, useRef, useState } from "react";
import { ImagePlus, Loader2 } from "lucide-react";
import type { GalleryItem } from "@/lib/api";
import { cn } from "@/lib/utils";

/** Width over height from the listing, when the file header could be read. */
export function itemRatio(item: GalleryItem) {
  return item.width && item.height ? item.width / item.height : null;
}

/** Report whether `ref` is near the page's visible area, so off-screen images never load. */
export function useNearViewport(ref: React.RefObject<HTMLElement | null>) {
  const [visible, setVisible] = useState(false);
  useEffect(() => {
    const element = ref.current;
    if (!element) return;
    const stage = element.closest<HTMLElement>("[data-page-scroll-container]");
    let intersecting = false;
    const update = () => setVisible(intersecting && stage?.getAttribute("aria-hidden") !== "true");
    const observer = new IntersectionObserver(entries => {
      intersecting = entries[0]?.isIntersecting ?? false;
      update();
    }, { root: stage, rootMargin: "200px 0px" });
    observer.observe(element);
    const visibility = new MutationObserver(update);
    if (stage) visibility.observe(stage, { attributes: true, attributeFilter: ["aria-hidden"] });
    return () => { observer.disconnect(); visibility.disconnect(); };
  }, [ref]);
  return visible;
}

/** Load `item` through `load` while `visible`; null until it arrives, "failed" if it cannot. */
export function useGallerySource(item: GalleryItem, visible: boolean, load: (item: GalleryItem) => Promise<string>) {
  const [src, setSrc] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    setSrc(null);
    setFailed(false);
    if (!visible) return;
    let active = true;
    void load(item).then(value => {
      if (active) setSrc(value);
    }).catch(() => {
      if (active) setFailed(true);
    });
    return () => { active = false; };
  }, [visible, item.path, item.modifiedAt, item.size, load]);
  return { src, failed };
}

/** The whole image at its own proportions; the box is sized before it loads, so nothing jumps. */
export function GalleryThumbnail({ item, load, onPreview, opening, className }: {
  item: GalleryItem;
  load: (item: GalleryItem) => Promise<string>;
  onPreview: () => void;
  opening: boolean;
  className?: string;
}) {
  const anchor = useRef<HTMLButtonElement>(null);
  const visible = useNearViewport(anchor);
  const { src, failed } = useGallerySource(item, visible, load);
  // Images whose header could not be read take their shape once they arrive.
  const [natural, setNatural] = useState<number | null>(null);
  const ratio = itemRatio(item) ?? natural ?? 1;

  return (
    <button ref={anchor} type="button" className={cn("gallery-thumb relative block w-full", className)}
      style={{ aspectRatio: ratio }} onClick={onPreview} disabled={opening} title={item.name}>
      {src ? (
        <img src={src} alt={item.name} draggable={false}
          onLoad={event => !itemRatio(item) && setNatural(event.currentTarget.naturalWidth / event.currentTarget.naturalHeight || 1)}
          className="gallery-thumb-image h-full w-full object-cover transition-transform duration-500 group-hover:scale-[1.03]" />
      ) : (
        <span className="flex h-full w-full items-center justify-center">
          {failed ? <ImagePlus className="h-4 w-4 text-muted-foreground" /> :
            <Loader2 className={cn("h-4 w-4 text-muted-foreground/60", visible && "animate-spin")} />}
        </span>
      )}
      {opening && <Loader2 className="absolute left-1/2 top-1/2 h-5 w-5 animate-spin text-white" />}
    </button>
  );
}
