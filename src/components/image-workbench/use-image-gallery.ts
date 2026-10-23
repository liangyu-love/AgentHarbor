import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { api, type GalleryItem, type GalleryMeta, type GenerationMeta } from "@/lib/api";
import { toast } from "@/hooks/use-toast";
import { queryKeys } from "@/lib/query-keys";
import { isTauriRuntime } from "@/lib/tauri-runtime";
import { GALLERY_THUMBNAIL_CACHE_LIMIT } from "./workbench-types";

const GALLERY_KEY = queryKeys.imageGallery;

/** One image the preview can show: a canvas image already in memory, or a gallery file. */
export type PreviewEntry = {
  key: string;
  src?: string;
  path?: string;
  meta?: GalleryMeta | null;
  /** Milliseconds, for gallery files. */
  modifiedAt?: number;
};

export type PreviewState = { entries: PreviewEntry[]; index: number; src: string };

/** Local gallery: listing, thumbnails, previews, save-as, delete and its directory. */
export function useImageGallery(pageActive: boolean) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [savedGalleryDir, setSavedGalleryDir] = useState<string | null>(null);
  const [preview, setPreview] = useState<PreviewState | null>(null);
  // A neighbour is loading while the current image stays on screen.
  const [previewLoading, setPreviewLoading] = useState(false);
  const thumbnailCache = useRef(new Map<string, string>());
  const thumbnailLoads = useRef(new Map<string, Promise<string>>());
  const gallerySession = useRef(0);
  const previewRequest = useRef(0);
  const [galleryPreviewPath, setGalleryPreviewPath] = useState<string | null>(null);
  const [pendingDelete, setPendingDelete] = useState<GalleryItem | null>(null);
  // A folder the image materials already use, waiting for the user to accept sharing it.
  const [sharedGalleryDir, setSharedGalleryDir] = useState<string | null>(null);
  const [galleryOpen, setGalleryOpen] = useState(false);
  const galleryRef = useRef<HTMLDivElement>(null);

  const galleryQuery = useQuery({
    queryKey: GALLERY_KEY,
    queryFn: () => api.image.listGallery(),
    enabled: isTauriRuntime() && galleryOpen,
    staleTime: 5_000,
  });
  const galleryItems = useMemo(() => galleryQuery.data?.items ?? [], [galleryQuery.data]);

  useEffect(() => {
    if (!galleryOpen || !isTauriRuntime()) return;
    if (galleryQuery.data?.directory) {
      setSavedGalleryDir(galleryQuery.data.directory);
    }
  }, [galleryOpen, galleryQuery.data?.directory]);

  useEffect(() => () => {
    gallerySession.current += 1;
    previewRequest.current += 1;
    thumbnailCache.current.clear();
    thumbnailLoads.current.clear();
  }, [galleryOpen]);

  /** A cached downscale of `item`; `edge` asks for a larger one than the grid's. */
  const loadGalleryThumbnail = useCallback((item: GalleryItem, edge?: number) => {
    const key = JSON.stringify([item.path, item.modifiedAt, item.size, edge ?? null]);
    const cache = thumbnailCache.current;
    const cached = cache.get(key);
    if (cached) {
      cache.delete(key);
      cache.set(key, cached);
      return Promise.resolve(cached);
    }
    const pending = thumbnailLoads.current.get(key);
    if (pending) return pending;
    const session = gallerySession.current;
    const request = api.image.loadGalleryThumbnail(item.path, edge).then(src => {
      if (session === gallerySession.current) {
        cache.set(key, src);
        while (cache.size > GALLERY_THUMBNAIL_CACHE_LIMIT) {
          cache.delete(cache.keys().next().value!);
        }
      }
      return src;
    }).finally(() => {
      if (thumbnailLoads.current.get(key) === request) thumbnailLoads.current.delete(key);
    });
    thumbnailLoads.current.set(key, request);
    return request;
  }, []);

  const persistToGallery = async (images: string[], meta?: GenerationMeta): Promise<boolean> => {
    if (!isTauriRuntime() || images.length === 0) return false;
    try {
      const saved = await api.image.saveToGallery(images, meta);
      setSavedGalleryDir(saved.directory);
      toast({
        title: t("image.savedToGallery"),
        description: saved.directory,
        variant: "success",
      });
      return true;
    } catch (cause) {
      const key =
        typeof cause === "string" ? cause : cause instanceof Error ? cause.message : "image.errors.save";
      toast({
        title: t("image.saveFailed"),
        description: t(key.startsWith("image.") ? key : "image.errors.save"),
        variant: "destructive",
      });
      return false;
    }
  };

  const openGalleryDirectory = async () => {
    const directory = savedGalleryDir;
    if (!directory) return;
    try {
      await api.openPath(directory);
    } catch {
      toast({ title: t("image.openGalleryFailed"), description: t("image.openGalleryFailedDetail"), variant: "destructive" });
    }
  };

  /** Moves the gallery to `selected`; the image materials' folder waits for the user's consent. */
  const applyGalleryDirectory = async (selected: string, allowShared: boolean) => {
    try {
      const directory = await api.image.setGalleryDirectory(selected, allowShared);
      setSavedGalleryDir(directory);
      await queryClient.invalidateQueries({ queryKey: GALLERY_KEY });
    } catch (cause) {
      const key = typeof cause === "string" ? cause : cause instanceof Error ? cause.message : "image.errors.save";
      if (key === "image.errors.galleryShared") {
        setSharedGalleryDir(selected);
        return;
      }
      toast({ title: t("image.saveFailed"), description: t(key.startsWith("image.") ? key : "image.errors.save"), variant: "destructive" });
    }
  };

  const chooseGalleryDirectory = async () => {
    if (!isTauriRuntime()) return;
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const selected = await open({ directory: true, multiple: false });
      if (typeof selected === "string") await applyGalleryDirectory(selected, false);
    } catch {
      toast({ title: t("image.saveFailed"), description: t("image.errors.save"), variant: "destructive" });
    }
  };

  const confirmSharedGalleryDir = () => {
    const selected = sharedGalleryDir;
    setSharedGalleryDir(null);
    if (selected) void applyGalleryDirectory(selected, true);
  };

  const saveImageAs = async (src: string, index: number) => {
    const mime = src.slice(5, src.indexOf(";")).toLowerCase();
    const extension = mime === "image/jpeg" || mime === "image/jpg" ? "jpg"
      : mime === "image/webp" ? "webp" : mime === "image/gif" ? "gif" : "png";
    const filename = `agentharbor-ink-art-${Date.now()}-${index + 1}.${extension}`;
    if (!isTauriRuntime()) {
      const link = document.createElement("a");
      link.href = src;
      link.download = filename;
      link.click();
      return;
    }
    try {
      const { save } = await import("@tauri-apps/plugin-dialog");
      const path = await save({
        defaultPath: filename,
        filters: [{ name: extension.toUpperCase(), extensions: [extension] }],
      });
      if (typeof path !== "string") return;
      await api.image.saveFile(path, src);
      toast({ title: t("image.saveLocal"), description: path, variant: "success" });
    } catch (cause) {
      const key =
        typeof cause === "string" ? cause : cause instanceof Error ? cause.message : "image.errors.save";
      toast({
        title: t("image.saveFailed"),
        description: t(key.startsWith("image.") ? key : "image.errors.save"),
        variant: "destructive",
      });
    }
  };

  const scrollToGallery = useCallback(() => {
    const gallery = galleryRef.current;
    const stage = gallery?.closest<HTMLElement>("[data-page-scroll-container]");
    if (!gallery || !stage) return;
    const top = stage.scrollTop + gallery.getBoundingClientRect().top - stage.getBoundingClientRect().top - 24;
    stage.scrollTo({ top, behavior: window.matchMedia("(prefers-reduced-motion: reduce)").matches ? "auto" : "smooth" });
  }, []);

  useEffect(() => {
    if (!galleryOpen || !pageActive) return;
    const frame = requestAnimationFrame(scrollToGallery);
    return () => cancelAnimationFrame(frame);
  }, [galleryOpen, pageActive, scrollToGallery]);

  const openGallery = async () => {
    setGalleryOpen(true);
    void queryClient.invalidateQueries({ queryKey: GALLERY_KEY });
    if (galleryOpen) scrollToGallery();
  };

  const refreshGallery = () => void queryClient.invalidateQueries({ queryKey: GALLERY_KEY });

  /** Show entry `index`; a gallery file opens once its original has loaded. */
  const showEntry = async (entries: PreviewEntry[], index: number) => {
    const entry = entries[index];
    if (!entry) return;
    const request = ++previewRequest.current;
    if (entry.src) {
      setGalleryPreviewPath(null);
      setPreviewLoading(false);
      setPreview({ entries, index, src: entry.src });
      return;
    }
    if (!entry.path) return;
    setGalleryPreviewPath(entry.path);
    setPreviewLoading(true);
    try {
      const src = await api.image.loadGalleryImage(entry.path);
      if (request === previewRequest.current) setPreview({ entries, index, src });
    } catch {
      if (request === previewRequest.current) {
        toast({ title: t("common.error"), description: t("image.errors.notFound"), variant: "destructive" });
      }
    } finally {
      if (request === previewRequest.current) {
        setGalleryPreviewPath(null);
        setPreviewLoading(false);
      }
    }
  };

  /** Open the preview on `entries[index]`, or close it with null. */
  const showPreview = (entries: PreviewEntry[] | null, index = 0) => {
    if (!entries) {
      previewRequest.current += 1;
      setGalleryPreviewPath(null);
      setPreviewLoading(false);
      setPreview(null);
      return;
    }
    void showEntry(entries, index);
  };

  /** Step to a neighbour of the image on screen. */
  const stepPreview = (step: number) => {
    if (!preview || previewLoading) return;
    const next = preview.index + step;
    if (next >= 0 && next < preview.entries.length) void showEntry(preview.entries, next);
  };

  const closeGallery = () => {
    gallerySession.current += 1;
    thumbnailCache.current.clear();
    thumbnailLoads.current.clear();
    showPreview(null);
    setGalleryOpen(false);
  };

  /** Open a gallery image; the viewer then steps through `items` (what the grid shows). */
  const openGalleryImage = (item: GalleryItem, items: GalleryItem[]) => {
    setPreview(null);
    const entries = items.map(entry => ({ key: entry.path, path: entry.path, meta: entry.meta, modifiedAt: entry.modifiedAt }));
    void showEntry(entries, Math.max(0, items.indexOf(item)));
  };

  const saveGalleryImageAs = async (item: GalleryItem, index: number) => {
    try {
      await saveImageAs(await api.image.loadGalleryImage(item.path), index);
    } catch {
      toast({ title: t("image.saveFailed"), description: t("image.errors.notFound"), variant: "destructive" });
    }
  };

  const deleteGalleryItem = async () => {
    const item = pendingDelete;
    if (!item) return;
    try {
      await api.image.deleteGalleryImage(item.path);
      gallerySession.current += 1;
      thumbnailCache.current.clear();
      thumbnailLoads.current.clear();
      void queryClient.invalidateQueries({ queryKey: GALLERY_KEY });
      toast({ title: t("image.galleryDeleted"), description: item.name, variant: "success" });
    } catch {
      toast({ title: t("image.saveFailed"), description: item.name, variant: "destructive" });
    } finally {
      setPendingDelete(null);
    }
  };

  return {
    galleryOpen, galleryRef, galleryQuery, galleryItems, savedGalleryDir,
    preview, previewLoading, stepPreview, galleryPreviewPath, pendingDelete, setPendingDelete,
    loadGalleryThumbnail, persistToGallery, openGalleryDirectory, chooseGalleryDirectory,
    sharedGalleryDir, confirmSharedGalleryDir, cancelSharedGalleryDir: () => setSharedGalleryDir(null),
    saveImageAs, openGallery, refreshGallery, showPreview, closeGallery, openGalleryImage,
    saveGalleryImageAs, deleteGalleryItem,
  };
}

export type ImageGallery = ReturnType<typeof useImageGallery>;
