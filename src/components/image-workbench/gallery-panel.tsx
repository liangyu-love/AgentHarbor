import { useMemo, useState } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import { Download, Feather, FolderOpen, History, RefreshCw, Search, Trash2, X } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { formatDateTime, formatRelative } from "@/lib/format-time";
import type { GalleryItem } from "@/lib/api";
import { cn } from "@/lib/utils";
import { GalleryThumbnail } from "./gallery-thumbnail";
import { ImageViewer } from "./image-viewer";
import type { ImageGallery } from "./use-image-gallery";

/** Gallery images by the day they were saved, newest day first: 今天, 昨天, then dates. */
function byDay(items: GalleryItem[]) {
  const dayStart = (ms: number) => new Date(new Date(ms).toDateString()).getTime();
  const today = dayStart(Date.now());
  const groups: { label: string; items: GalleryItem[] }[] = [];
  for (const item of items) {
    const day = dayStart(item.modifiedAt);
    const days = Math.round((today - day) / 86_400_000);
    const date = new Date(day);
    const label = days === 0 ? "今天" : days === 1 ? "昨天"
      : date.getFullYear() === new Date().getFullYear()
        ? `${date.getMonth() + 1}月${date.getDate()}日`
        : `${date.getFullYear()}年${date.getMonth() + 1}月${date.getDate()}日`;
    const last = groups[groups.length - 1];
    if (last?.label === label) last.items.push(item);
    else groups.push({ label, items: [item] });
  }
  return groups;
}

const matches = (item: GalleryItem, query: string) =>
  [item.meta?.prompt, item.meta?.revisedPrompt, item.meta?.model, item.name]
    .some(text => text?.toLowerCase().includes(query));

/** The history gallery section, shown once the user opens it. */
export function GalleryPanel({ gallery, onUsePrompt }: {
  gallery: ImageGallery;
  /** Put a saved prompt back in the prompt box. */
  onUsePrompt: (prompt: string) => void;
}) {
  const { t } = useTranslation();
  const [query, setQuery] = useState("");
  const shown = useMemo(() => {
    const q = query.trim().toLowerCase();
    return q ? gallery.galleryItems.filter(item => matches(item, q)) : gallery.galleryItems;
  }, [gallery.galleryItems, query]);
  const days = useMemo(() => byDay(shown), [shown]);
  return (
    <div ref={gallery.galleryRef} data-gallery-section className="ink-scroll-frame p-5 sm:p-6 scroll-mt-6">
      <div className="mb-3 flex flex-wrap items-center justify-between gap-2 border-b border-border/50 pb-3">
        <div className="flex items-center gap-2">
          <History className="h-4 w-4 text-amber-600 dark:text-amber-400" />
          <h2 className="font-serif text-base font-bold tracking-wide text-foreground">
            {t("image.galleryTitle")}
          </h2>
          <Badge variant="outline" className="font-mono text-[11px]">
            {gallery.galleryItems.length}
          </Badge>
          {gallery.galleryQuery.data?.hasMore && (
            <span className="text-xs text-muted-foreground">{t("image.galleryLimit")}</span>
          )}
        </div>
        <div className="flex items-center gap-1.5">
          <Button
            type="button"
            size="sm"
            variant="ghost"
            className="h-8 gap-1 text-xs"
            onClick={() => void gallery.chooseGalleryDirectory()}
          >
            <FolderOpen className="h-3.5 w-3.5" />
            {t("image.chooseGalleryDirectory")}
          </Button>
          <Button
            type="button"
            size="sm"
            variant="ghost"
            className="h-8 gap-1 text-xs"
            onClick={gallery.refreshGallery}
          >
            <RefreshCw className={cn("h-3.5 w-3.5", gallery.galleryQuery.isFetching && "animate-spin")} />
            {t("image.galleryRefresh")}
          </Button>
          <Button
            type="button"
            size="icon"
            variant="ghost"
            className="h-8 w-8"
            onClick={gallery.closeGallery}
            aria-label="收起画册"
          >
            <X className="h-4 w-4" />
          </Button>
        </div>
      </div>
      {gallery.savedGalleryDir && (
        <div className="mb-4 flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1.5 text-xs text-muted-foreground">
          <span className="shrink-0">{t("image.galleryCurrentDirectory")}</span>
          <span className="min-w-0 flex-1 truncate text-foreground/80" title={gallery.savedGalleryDir}>{gallery.savedGalleryDir}</span>
          <Button
            type="button"
            size="xs"
            variant="ghost"
            className="shrink-0"
            onClick={() => void gallery.openGalleryDirectory()}
          >
            <FolderOpen className="h-3.5 w-3.5" />
            {t("image.openGallery")}
          </Button>
        </div>
      )}

      {gallery.galleryItems.length > 0 && (
        <div className="relative mb-4">
          <Search className="pointer-events-none absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground/70" />
          <input
            value={query}
            onChange={event => setQuery(event.target.value)}
            placeholder="按提示词、模型或文件名检索画册"
            aria-label="检索画册"
            className="h-9 w-full min-w-0 rounded-lg border border-border/60 bg-background/80 pl-8 pr-8 text-xs outline-none transition-colors focus:border-[#b83b38]/40"
          />
          {query && (
            <button type="button" onClick={() => setQuery("")} aria-label="清除检索"
              className="absolute right-2.5 top-1/2 -translate-y-1/2 text-muted-foreground/70 hover:text-foreground">
              <X className="h-3.5 w-3.5" />
            </button>
          )}
        </div>
      )}

      {gallery.galleryQuery.isLoading ? (
        <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5">
          {Array.from({ length: 5 }).map((_, index) => (
            <div
              key={index}
              className="aspect-square animate-pulse rounded-lg border border-border/50 bg-muted/40"
            />
          ))}
        </div>
      ) : gallery.galleryQuery.isError ? (
        <p className="py-10 text-center font-serif text-sm text-destructive">
          {t("image.galleryLoadFailed")}
        </p>
      ) : gallery.galleryItems.length === 0 ? (
        <p className="py-10 text-center font-serif text-sm text-muted-foreground">
          {t("image.galleryEmpty")}
        </p>
      ) : shown.length === 0 ? (
        <p className="py-10 text-center font-serif text-sm text-muted-foreground">
          没有提示词、模型或文件名包含「{query.trim()}」的图片
        </p>
      ) : (
        <div className="space-y-5">
          {days.map(day => (
            <section key={day.label} aria-label={day.label}>
              <h3 className="mb-2 flex items-baseline gap-2 font-serif text-xs font-semibold text-foreground/80">
                {day.label}
                <span className="font-sans text-[11px] font-normal text-muted-foreground">{day.items.length} 张</span>
              </h3>
              <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5">
                {day.items.map(item => (
                  <GalleryCard
                    key={item.path}
                    item={item}
                    gallery={gallery}
                    onOpen={() => gallery.openGalleryImage(item, shown)}
                    onUsePrompt={onUsePrompt}
                  />
                ))}
              </div>
            </section>
          ))}
        </div>
      )}
    </div>
  );
}

/** One saved image: its thumbnail, what made it, and quick actions on hover. */
function GalleryCard({ item, gallery, onOpen, onUsePrompt }: {
  item: GalleryItem;
  gallery: ImageGallery;
  onOpen: () => void;
  onUsePrompt: (prompt: string) => void;
}) {
  const { t } = useTranslation();
  const prompt = item.meta?.prompt;
  const overlayButton = "h-7 w-7 bg-black/55 text-white hover:bg-black/75";
  return (
    <div data-gallery-card className="group relative overflow-hidden rounded-xl border border-border/60 bg-card/70 shadow-sm transition-[border-color,box-shadow] hover:border-[#b83b38]/35 hover:shadow-md">
      <GalleryThumbnail
        item={item}
        load={gallery.loadGalleryThumbnail}
        onPreview={onOpen}
        opening={gallery.galleryPreviewPath === item.path}
      />
      <div className="pointer-events-none absolute inset-x-0 bottom-0 space-y-0.5 bg-gradient-to-t from-black/80 via-black/45 to-transparent px-2.5 pb-2 pt-8 text-white">
        <p className={cn("line-clamp-2 break-words text-[11px] leading-snug", prompt ? "font-serif" : "font-mono text-[10px] opacity-90")}
          title={prompt ?? item.name}>
          {prompt ?? item.name}
        </p>
        <div className="flex min-w-0 items-center gap-1.5 text-[10px] text-white/75">
          {/* The gallery reports milliseconds; the formatters take seconds. */}
          <span className="pointer-events-auto shrink-0" title={formatDateTime(item.modifiedAt / 1000)}>
            {formatRelative(item.modifiedAt / 1000)}
          </span>
          {item.meta?.model && <span className="truncate font-mono">· {item.meta.model}</span>}
        </div>
      </div>
      <div className="absolute right-1.5 top-1.5 flex gap-1 opacity-0 transition-opacity focus-within:opacity-100 group-hover:opacity-100">
        {prompt && (
          <Button type="button" size="icon" variant="secondary" className={cn(overlayButton, "hover:bg-[#b83b38]")}
            title="填入提示词" aria-label="填入提示词" onClick={() => onUsePrompt(prompt)}>
            <Feather className="h-3.5 w-3.5" />
          </Button>
        )}
        <Button type="button" size="icon" variant="secondary" className={overlayButton}
          title={t("image.saveAs")} aria-label={t("image.saveAs")}
          onClick={() => void gallery.saveGalleryImageAs(item, gallery.galleryItems.indexOf(item))}>
          <Download className="h-3.5 w-3.5" />
        </Button>
        <Button type="button" size="icon" variant="secondary" className={cn(overlayButton, "hover:bg-destructive")}
          title={t("image.galleryDelete")} aria-label={t("image.galleryDelete")}
          onClick={() => gallery.setPendingDelete(item)}>
          <Trash2 className="h-3.5 w-3.5" />
        </Button>
      </div>
    </div>
  );
}

/** Delete confirmation and the zoomable full-size preview. */
export function GalleryOverlays({ gallery, pageActive, onUsePrompt }: {
  gallery: ImageGallery;
  pageActive: boolean;
  onUsePrompt: (prompt: string) => void;
}) {
  const { t } = useTranslation();
  const preview = gallery.preview;
  const entry = preview?.entries[preview.index];
  return (
    <>
      <AlertDialog open={gallery.pendingDelete !== null} onOpenChange={open => !open && gallery.setPendingDelete(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("image.galleryDeleteConfirm")}</AlertDialogTitle>
            <AlertDialogDescription>
              {t("image.galleryDeleteConfirmDesc")}
              {gallery.pendingDelete ? ` (${gallery.pendingDelete.name})` : ""}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction onClick={() => void gallery.deleteGalleryItem()}>
              {t("image.galleryDelete")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      {preview && entry && pageActive && createPortal(
        <ImageViewer
          src={preview.src}
          alt="成图放大预览"
          onClose={() => gallery.showPreview(null)}
          meta={entry.meta}
          takenAt={entry.modifiedAt}
          position={{ index: preview.index, total: preview.entries.length }}
          onStep={gallery.stepPreview}
          loading={gallery.previewLoading}
          onUsePrompt={onUsePrompt}
        />,
        document.body,
      )}
    </>
  );
}
