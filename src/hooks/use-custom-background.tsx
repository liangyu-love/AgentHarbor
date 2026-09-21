import React, {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import { toast } from "@/hooks/use-toast";

export interface WallpaperPreset {
  id: string;
  name: string;
  gradient: string;
  previewClass?: string;
}

export const WALLPAPER_PRESETS: WallpaperPreset[] = [
  {
    id: "aurora",
    name: "极光微风",
    gradient:
      "radial-gradient(at 15% 20%, rgba(59, 130, 246, 0.55) 0px, transparent 55%), radial-gradient(at 85% 15%, rgba(139, 92, 246, 0.5) 0px, transparent 50%), radial-gradient(at 50% 85%, rgba(16, 185, 129, 0.45) 0px, transparent 55%), radial-gradient(at 80% 80%, rgba(6, 182, 212, 0.4) 0px, transparent 50%), #0d131f",
  },
  {
    id: "cyber",
    name: "霓虹幻夜",
    gradient:
      "radial-gradient(at 0% 100%, rgba(236, 72, 153, 0.5) 0px, transparent 50%), radial-gradient(at 100% 0%, rgba(59, 130, 246, 0.55) 0px, transparent 50%), radial-gradient(at 50% 50%, rgba(99, 102, 241, 0.45) 0px, transparent 55%), radial-gradient(at 10% 20%, rgba(168, 85, 247, 0.4) 0px, transparent 50%), #0a0e1a",
  },
  {
    id: "sunset",
    name: "暮光晚霞",
    gradient:
      "radial-gradient(at 20% 80%, rgba(249, 115, 22, 0.5) 0px, transparent 50%), radial-gradient(at 80% 20%, rgba(168, 85, 247, 0.5) 0px, transparent 50%), radial-gradient(at 50% 10%, rgba(236, 72, 153, 0.4) 0px, transparent 50%), #160c22",
  },
  {
    id: "ocean",
    name: "深邃蔚蓝",
    gradient:
      "radial-gradient(at 20% 30%, rgba(14, 165, 233, 0.55) 0px, transparent 50%), radial-gradient(at 80% 70%, rgba(59, 130, 246, 0.45) 0px, transparent 50%), radial-gradient(at 50% 90%, rgba(99, 102, 241, 0.4) 0px, transparent 55%), #081120",
  },
  {
    id: "slate",
    name: "素雅岩灰",
    gradient:
      "radial-gradient(at 30% 30%, rgba(148, 163, 184, 0.3) 0px, transparent 60%), radial-gradient(at 70% 70%, rgba(71, 85, 105, 0.35) 0px, transparent 60%), #0f172a",
  },
];

const STORAGE_SETTINGS_KEY = "agentharbor_wallpaper_settings_v1";
const DB_NAME = "agentharbor_wallpaper_db";
const STORE_NAME = "assets";
const ACTIVE_WALLPAPER_KEY = "active_custom_wallpaper";

interface StoredSettings {
  enabled: boolean;
  opacity: number;
  blur: number;
  overlayDim: number;
  presetId: string | null;
  hasCustomUpload: boolean;
}

const DEFAULT_SETTINGS: StoredSettings = {
  enabled: false,
  opacity: 0.45,
  blur: 8,
  overlayDim: 0.2,
  presetId: "aurora",
  hasCustomUpload: false,
};

function getStoredSettings(): StoredSettings {
  try {
    const raw = localStorage.getItem(STORAGE_SETTINGS_KEY);
    if (!raw) return DEFAULT_SETTINGS;
    return { ...DEFAULT_SETTINGS, ...JSON.parse(raw) };
  } catch {
    return DEFAULT_SETTINGS;
  }
}

function openDb(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const request = indexedDB.open(DB_NAME, 1);
    request.onupgradeneeded = () => {
      const db = request.result;
      if (!db.objectStoreNames.contains(STORE_NAME)) {
        db.createObjectStore(STORE_NAME);
      }
    };
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
}

async function saveCustomBlob(blob: Blob): Promise<void> {
  const db = await openDb();
  return new Promise((resolve, reject) => {
    const tx = db.transaction(STORE_NAME, "readwrite");
    const store = tx.objectStore(STORE_NAME);
    const req = store.put(blob, ACTIVE_WALLPAPER_KEY);
    req.onsuccess = () => resolve();
    req.onerror = () => reject(req.error);
  });
}

async function loadCustomBlob(): Promise<Blob | null> {
  try {
    const db = await openDb();
    return new Promise((resolve, reject) => {
      const tx = db.transaction(STORE_NAME, "readonly");
      const store = tx.objectStore(STORE_NAME);
      const req = store.get(ACTIVE_WALLPAPER_KEY);
      req.onsuccess = () => resolve((req.result as Blob) || null);
      req.onerror = () => reject(req.error);
    });
  } catch {
    return null;
  }
}

async function deleteCustomBlob(): Promise<void> {
  try {
    const db = await openDb();
    return new Promise((resolve, reject) => {
      const tx = db.transaction(STORE_NAME, "readwrite");
      const store = tx.objectStore(STORE_NAME);
      const req = store.delete(ACTIVE_WALLPAPER_KEY);
      req.onsuccess = () => resolve();
      req.onerror = () => reject(req.error);
    });
  } catch {
    // Ignore
  }
}

interface CustomBackgroundContextType {
  enabled: boolean;
  opacity: number;
  blur: number;
  overlayDim: number;
  presetId: string | null;
  hasCustomUpload: boolean;
  customImageUrl: string | null;
  activeBackgroundStyle: React.CSSProperties | null;
  setEnabled: (enabled: boolean) => void;
  setOpacity: (opacity: number) => void;
  setBlur: (blur: number) => void;
  setOverlayDim: (dim: number) => void;
  selectPreset: (presetId: string) => void;
  uploadCustomImage: (file: File) => Promise<void>;
  clearCustomImage: () => Promise<void>;
  resetToDefault: () => Promise<void>;
}

const CustomBackgroundContext = createContext<CustomBackgroundContextType | null>(null);

export function CustomBackgroundProvider({ children }: { children: ReactNode }) {
  const [settings, setSettings] = useState<StoredSettings>(getStoredSettings);
  const [customImageUrl, setCustomImageUrl] = useState<string | null>(null);

  // Synchronize settings to localStorage
  useEffect(() => {
    try {
      localStorage.setItem(STORAGE_SETTINGS_KEY, JSON.stringify(settings));
    } catch {
      // Ignore
    }
  }, [settings]);

  // Load custom image from IndexedDB on initial mount
  useEffect(() => {
    let active = true;
    let urlToRevoke: string | null = null;

    loadCustomBlob()
      .then((blob) => {
        if (!active || !blob) return;
        const objectUrl = URL.createObjectURL(blob);
        urlToRevoke = objectUrl;
        setCustomImageUrl(objectUrl);
      })
      .catch(() => {
        // ignore
      });

    return () => {
      active = false;
      if (urlToRevoke) URL.revokeObjectURL(urlToRevoke);
    };
  }, []);

  // Compute active background CSS style
  const activeBackgroundStyle = useMemo<React.CSSProperties | null>(() => {
    if (!settings.enabled) return null;

    if (settings.hasCustomUpload && customImageUrl) {
      return {
        backgroundImage: `url(${customImageUrl})`,
        backgroundSize: "cover",
        backgroundPosition: "center",
        backgroundRepeat: "no-repeat",
      };
    }

    if (settings.presetId) {
      const preset = WALLPAPER_PRESETS.find((p) => p.id === settings.presetId);
      if (preset) {
        return {
          background: preset.gradient,
          backgroundSize: "cover",
          backgroundPosition: "center",
        };
      }
    }

    return null;
  }, [settings.enabled, settings.hasCustomUpload, customImageUrl, settings.presetId]);

  const hasActiveBackground = Boolean(settings.enabled && activeBackgroundStyle);

  // Set global HTML data attribute for adaptive glassmorphism
  useEffect(() => {
    if (hasActiveBackground) {
      document.documentElement.dataset.hasCustomBg = "true";
    } else {
      delete document.documentElement.dataset.hasCustomBg;
    }
  }, [hasActiveBackground]);

  const setEnabled = useCallback((enabled: boolean) => {
    setSettings((prev) => ({ ...prev, enabled }));
  }, []);

  const setOpacity = useCallback((opacity: number) => {
    setSettings((prev) => ({ ...prev, opacity }));
  }, []);

  const setBlur = useCallback((blur: number) => {
    setSettings((prev) => ({ ...prev, blur }));
  }, []);

  const setOverlayDim = useCallback((overlayDim: number) => {
    setSettings((prev) => ({ ...prev, overlayDim }));
  }, []);

  const selectPreset = useCallback((presetId: string) => {
    setSettings((prev) => ({
      ...prev,
      presetId,
      hasCustomUpload: false,
      enabled: true,
    }));
  }, []);

  const uploadCustomImage = useCallback(
    async (file: File) => {
      if (!file.type.startsWith("image/")) {
        toast({
          title: "文件格式不支持",
          description: "请选择常见的图片格式（PNG、JPG、WebP 等）",
          variant: "destructive",
        });
        return;
      }

      try {
        await saveCustomBlob(file);
        if (customImageUrl) {
          URL.revokeObjectURL(customImageUrl);
        }
        const newUrl = URL.createObjectURL(file);
        setCustomImageUrl(newUrl);
        setSettings((prev) => ({
          ...prev,
          hasCustomUpload: true,
          presetId: null,
          enabled: true,
        }));
        toast({
          title: "壁纸已更新",
          description: "自定义壁纸已应用并持久保存",
          variant: "success",
        });
      } catch (err) {
        toast({
          title: "保存壁纸失败",
          description: String(err),
          variant: "destructive",
        });
      }
    },
    [customImageUrl],
  );

  const clearCustomImage = useCallback(async () => {
    try {
      await deleteCustomBlob();
      if (customImageUrl) {
        URL.revokeObjectURL(customImageUrl);
      }
      setCustomImageUrl(null);
      setSettings((prev) => ({
        ...prev,
        hasCustomUpload: false,
        presetId: "aurora",
        enabled: false,
      }));
      toast({
        title: "壁纸已清除",
        description: "已恢复默认背景",
        variant: "default",
      });
    } catch {
      // ignore
    }
  }, [customImageUrl]);

  const resetToDefault = useCallback(async () => {
    try {
      await deleteCustomBlob();
      if (customImageUrl) {
        URL.revokeObjectURL(customImageUrl);
      }
      setCustomImageUrl(null);
      setSettings(DEFAULT_SETTINGS);
      toast({
        title: "已重置为默认设置",
        variant: "default",
      });
    } catch {
      // ignore
    }
  }, [customImageUrl]);

  return (
    <CustomBackgroundContext.Provider
      value={{
        enabled: settings.enabled,
        opacity: settings.opacity,
        blur: settings.blur,
        overlayDim: settings.overlayDim,
        presetId: settings.presetId,
        hasCustomUpload: settings.hasCustomUpload,
        customImageUrl,
        activeBackgroundStyle,
        setEnabled,
        setOpacity,
        setBlur,
        setOverlayDim,
        selectPreset,
        uploadCustomImage,
        clearCustomImage,
        resetToDefault,
      }}
    >
      {children}
    </CustomBackgroundContext.Provider>
  );
}

export function useCustomBackground() {
  const context = useContext(CustomBackgroundContext);
  if (!context) {
    throw new Error("useCustomBackground must be used within CustomBackgroundProvider");
  }
  return context;
}

export function CustomBackgroundLayer() {
  const { enabled, activeBackgroundStyle, opacity, blur, overlayDim } = useCustomBackground();

  if (!enabled || !activeBackgroundStyle) return null;

  return (
    <div
      aria-hidden="true"
      className="pointer-events-none fixed inset-0 z-0 overflow-hidden select-none"
    >
      <div
        className="absolute inset-0 transition-all duration-300 ease-out will-change-[opacity,filter,transform]"
        style={{
          ...activeBackgroundStyle,
          opacity,
          filter: `blur(${blur}px)`,
          transform: blur > 0 ? "scale(1.05)" : "none",
        }}
      />
      {overlayDim > 0 && (
        <div
          className="absolute inset-0 bg-background transition-opacity duration-300"
          style={{ opacity: overlayDim }}
        />
      )}
    </div>
  );
}
