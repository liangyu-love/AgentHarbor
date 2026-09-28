import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import { Download, FolderOpen, History, RefreshCw, Trash2, X } from "lucide-react";
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
import { formatRelative } from "@/lib/format-time";
import { cn } from "@/lib/utils";
import { GalleryThumbnail } from "./gallery-thumbnail";
import type { ImageGallery } from "./use-image-gallery";

/** The history gallery section, shown once the user opens it. */
export function GalleryPanel({ gallery }: { gallery: ImageGallery }) {
  const { t } = useTranslation();
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

      {gallery.galleryQuery.isLoading ? (
        <div className="grid gap-2 sm:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5">
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
      ) : (
        <div className="grid gap-2 sm:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5">
          {gallery.galleryItems.map((item, index) => {
            return (
              <div
                key={item.path}
                className="group relative overflow-hidden rounded-lg border border-border/60 bg-card/70"
              >
                <GalleryThumbnail
                  item={item}
                  load={gallery.loadGalleryThumbnail}
                  onPreview={() => void gallery.openGalleryImage(item)}
                  opening={gallery.galleryPreviewPath === item.path}
                />
                <div className="absolute inset-x-0 bottom-0 flex items-center justify-between gap-1 bg-gradient-to-t from-black/70 to-transparent px-2 pb-1.5 pt-6 text-[10px] text-white">
                  <span className="truncate font-mono" title={item.name}>
                    {item.name}
                  </span>
                  <span className="shrink-0 opacity-90">{formatRelative(item.modifiedAt)}</span>
                </div>
                <div className="absolute right-1 top-1 flex gap-1 opacity-0 transition-opacity group-hover:opacity-100">
                  <Button
                    type="button"
                    size="icon"
                    variant="secondary"
                    className="h-7 w-7 bg-black/55 text-white hover:bg-black/75"
                    title={t("image.saveAs")}
                    onClick={() => void gallery.saveGalleryImageAs(item, index)}
                  >
                    <Download className="h-3.5 w-3.5" />
                  </Button>
                  <Button
                    type="button"
                    size="icon"
                    variant="secondary"
                    className="h-7 w-7 bg-black/55 text-white hover:bg-destructive"
                    title={t("image.galleryDelete")}
                    onClick={() => gallery.setPendingDelete(item)}
                  >
                    <Trash2 className="h-3.5 w-3.5" />
                  </Button>
                </div>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}

/** Delete confirmation and the full-size preview overlay. */
export function GalleryOverlays({ gallery, pageActive }: { gallery: ImageGallery; pageActive: boolean }) {
  const { t } = useTranslation();
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

      {gallery.previewSrc && pageActive && createPortal(
        <div
          className="fixed inset-0 z-[200] flex items-center justify-center bg-black/80 p-4 sm:p-6 backdrop-blur-sm animate-in fade-in-0 duration-200"
          role="dialog"
          aria-modal="true"
          aria-label="成图放大预览"
          onKeyDown={event => { if (event.key === "Escape") gallery.showPreview(null); }}
          onClick={() => gallery.showPreview(null)}
        >
          <button
            type="button"
            autoFocus
            className="absolute right-5 top-5 inline-flex h-9 w-9 items-center justify-center rounded-full bg-black/60 text-white/80 hover:text-white hover:bg-black/80 transition-all hover:scale-105 active:scale-95"
            onClick={() => gallery.showPreview(null)}
            aria-label="关闭预览"
          >
            <X className="h-4 w-4" />
          </button>
          <img
            src={gallery.previewSrc}
            alt="成图放大预览"
            className="max-h-[88vh] max-w-[92vw] rounded-xl object-contain shadow-2xl transition-transform duration-200 animate-in zoom-in-95 duration-200"
            onClick={event => event.stopPropagation()}
          />
        </div>, document.body
      )}
    </>
  );
}
