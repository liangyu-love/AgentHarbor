import { useEffect, useRef, useState } from "react";
import { ImagePlus, Loader2 } from "lucide-react";
import type { GalleryItem } from "@/lib/api";
import { cn } from "@/lib/utils";

export function GalleryThumbnail({ item, load, onPreview, opening }: {
  item: GalleryItem;
  load: (item: GalleryItem) => Promise<string>;
  onPreview: () => void;
  opening: boolean;
}) {
  const anchor = useRef<HTMLButtonElement>(null);
  const [visible, setVisible] = useState(false);
  const [src, setSrc] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    const element = anchor.current;
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
  }, []);

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

  return (
    <button ref={anchor} type="button" className="relative block aspect-square w-full bg-muted/30"
      onClick={onPreview} disabled={opening} title={item.name}>
      {src ? (
        <img src={src} alt={item.name} className="h-full w-full object-cover transition-transform duration-300 group-hover:scale-[1.03]" />
      ) : (
        <span className="flex h-full w-full items-center justify-center">
          {failed ? <ImagePlus className="h-4 w-4 text-muted-foreground" /> :
            <Loader2 className={cn("h-4 w-4 text-muted-foreground", visible && "animate-spin")} />}
        </span>
      )}
      {opening && <Loader2 className="absolute left-1/2 top-1/2 h-5 w-5 animate-spin text-white" />}
    </button>
  );
}
