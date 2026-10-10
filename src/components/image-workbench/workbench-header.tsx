import { useTranslation } from "react-i18next";
import { Brush, FolderOpen, History } from "lucide-react";
import { Button } from "@/components/ui/button";
import { PageHero } from "@/components/layout/page-hero";
import type { ImageGallery } from "./use-image-gallery";

export function WorkbenchHeader({ gallery }: { gallery: ImageGallery }) {
  const { t } = useTranslation();
  return (
  <PageHero kicker="Ink Studio" watermark="墨" icon={<Brush />} title="墨韵丹青" titleClassName="font-serif tracking-wider"
    className="ink-paper-bg" description={<span className="font-serif">GPT Image 2 / 2.5 · {t("image.promptSentToProvider")}</span>}
    actions={<>
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
    </>} />
  );
}
