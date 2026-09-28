import { useTranslation } from "react-i18next";
import { Brush, FolderOpen, History } from "lucide-react";
import { Button } from "@/components/ui/button";
import type { ImageGallery } from "./use-image-gallery";

export function WorkbenchHeader({ gallery }: { gallery: ImageGallery }) {
  const { t } = useTranslation();
  return (
  <div className="relative overflow-hidden rounded-2xl border border-border/70 bg-card p-4 ink-paper-bg">
    <div className="relative z-10 flex flex-wrap items-center justify-between gap-4">
      <div className="space-y-1">
        <div className="flex items-center gap-3">
          <div className="flex h-9 w-9 items-center justify-center rounded-xl border border-amber-600/30 bg-amber-500/10 text-amber-600 dark:border-amber-400/30 dark:bg-amber-400/10 dark:text-amber-400 shadow-xs">
            <Brush className="h-5 w-5" />
          </div>
          <div>
            <h1 className="font-serif text-xl font-bold tracking-wider text-foreground">墨韵丹青</h1>
          </div>
        </div>
        <p className="font-serif text-xs text-muted-foreground sm:text-sm">GPT Image 2 / 2.5 · {t("image.promptSentToProvider")}</p>
      </div>
      <div className="flex flex-wrap items-center gap-2">
        <Button
          type="button"
          size="sm"
          variant="outline"
          className="h-8 gap-1.5 font-serif text-xs"
          onClick={() => void gallery.openGallery()}
        >
          <History className="h-3.5 w-3.5" />
          {t("image.galleryTitle")}
        </Button>
        {gallery.savedGalleryDir && (
          <Button
            type="button"
            size="sm"
            variant="ghost"
            className="h-8 gap-1.5 font-serif text-xs text-muted-foreground"
            onClick={() => void gallery.openGalleryDirectory()}
          >
            <FolderOpen className="h-3.5 w-3.5" />
            {t("image.openGallery")}
          </Button>
        )}
      </div>
    </div>
  </div>
  );
}
