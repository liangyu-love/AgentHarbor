import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { api, type GalleryItem } from "@/lib/api";
import { toast } from "@/hooks/use-toast";
import { queryKeys } from "@/lib/query-keys";
import { isTauriRuntime } from "@/lib/tauri-runtime";
import { GALLERY_THUMBNAIL_CACHE_LIMIT } from "./workbench-types";

const GALLERY_KEY = queryKeys.imageGallery;

/** Local gallery: listing, thumbnails, previews, save-as, delete and its directory. */
export function useImageGallery(pageActive: boolean) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [savedGalleryDir, setSavedGalleryDir] = useState<string | null>(null);
  const [previewSrc, setPreviewSrc] = useState<string | null>(null);
  const thumbnailCache = useRef(new Map<string, string>());
  const thumbnailLoads = useRef(new Map<string, Promise<string>>());
  const gallerySession = useRef(0);
  const previewRequest = useRef(0);
  const [galleryPreviewPath, setGalleryPreviewPath] = useState<string | null>(null);
  const [pendingDelete, setPendingDelete] = useState<GalleryItem | null>(null);
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

  const loadGalleryThumbnail = useCallback((item: GalleryItem) => {
    const key = JSON.stringify([item.path, item.modifiedAt, item.size]);
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
    const request = api.image.loadGalleryThumbnail(item.path).then(src => {
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

  const persistToGallery = async (images: string[]): Promise<boolean> => {
    if (!isTauriRuntime() || images.length === 0) return false;
    try {
      const saved = await api.image.saveToGallery(images);
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

  const chooseGalleryDirectory = async () => {
    if (!isTauriRuntime()) return;
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const selected = await open({ directory: true, multiple: false });
      if (typeof selected !== "string") return;
      const directory = await api.image.setGalleryDirectory(selected);
      setSavedGalleryDir(directory);
      await queryClient.invalidateQueries({ queryKey: GALLERY_KEY });
    } catch (cause) {
      const key = typeof cause === "string" ? cause : cause instanceof Error ? cause.message : "image.errors.save";
      toast({ title: t("image.saveFailed"), description: t(key.startsWith("image.") ? key : "image.errors.save"), variant: "destructive" });
    }
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

  const showPreview = (src: string | null) => {
    previewRequest.current += 1;
    setGalleryPreviewPath(null);
    setPreviewSrc(src);
  };

  const closeGallery = () => {
    gallerySession.current += 1;
    thumbnailCache.current.clear();
    thumbnailLoads.current.clear();
    showPreview(null);
    setGalleryOpen(false);
  };

  const openGalleryImage = async (item: GalleryItem) => {
    const request = ++previewRequest.current;
    setGalleryPreviewPath(item.path);
    setPreviewSrc(null);
    try {
      const src = await api.image.loadGalleryImage(item.path);
      if (request === previewRequest.current) setPreviewSrc(src);
    } catch {
      if (request === previewRequest.current) {
        toast({ title: t("common.error"), description: t("image.errors.notFound"), variant: "destructive" });
      }
    } finally {
      if (request === previewRequest.current) setGalleryPreviewPath(null);
    }
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
    previewSrc, galleryPreviewPath, pendingDelete, setPendingDelete,
    loadGalleryThumbnail, persistToGallery, openGalleryDirectory, chooseGalleryDirectory,
    saveImageAs, openGallery, refreshGallery, showPreview, closeGallery, openGalleryImage,
    saveGalleryImageAs, deleteGalleryItem,
  };
}

export type ImageGallery = ReturnType<typeof useImageGallery>;
