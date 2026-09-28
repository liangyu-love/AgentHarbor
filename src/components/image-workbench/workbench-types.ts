import type { ImageGenerationResponse } from "@/lib/api";
import { storageKeys } from "@/lib/storage-keys";

export const GALLERY_THUMBNAIL_CACHE_LIMIT = 48;

type AspectRatioOption = {
  label: string;
  name: string;
  value: string;
  desc: string;
};

export const SIZES: AspectRatioOption[] = [
  { label: "1:1 方幅", name: "1024x1024", value: "1024x1024", desc: "经典正方 · 头像/画作" },
  { label: "3:2 横卷", name: "1536x1024", value: "1536x1024", desc: "宽幅山水 · 桌面壁纸" },
  { label: "2:3 立轴", name: "1024x1536", value: "1024x1536", desc: "高挑挂轴 · 移动端屏保" },
  { label: "16:9 广角", name: "1792x1024", value: "1792x1024", desc: "全景巨卷 · 影视镜头" },
  { label: "9:16 宣幅", name: "1024x1792", value: "1024x1792", desc: "通景长轴 · 竖幅插画" },
  { label: "自选尺幅", name: "auto", value: "auto", desc: "默认比例 · 智能匹配" },
];

export const QUALITIES = [
  { value: "auto", label: "自动", tip: "由模型选择" },
  { value: "high", label: "高", tip: "更多画面细节" },
  { value: "medium", label: "中", tip: "平衡细节与速度" },
  { value: "low", label: "低", tip: "优先生成速度" },
];

export type SavedImageConfig = {
  baseUrl: string;
  model: string;
  size: string;
  quality: string;
  legacyApiKey?: string;
};

export type GenerationJob = {
  id: string;
  prompt: string;
  model: string;
  status: "queued" | "running" | "completed" | "failed" | "cancelled";
  result?: ImageGenerationResponse;
  error?: string;
  galleryStatus?: "saving" | "saved" | "failed";
};

export function loadSavedConfig(): SavedImageConfig {
  const defaults = { baseUrl: "", model: "gpt-image-2", size: "1024x1024", quality: "auto" };
  try {
    const saved = localStorage.getItem(storageKeys.imageWorkbenchConfig);
    if (saved) {
      const parsed = JSON.parse(saved) as Partial<SavedImageConfig> & { apiKey?: string };
      const config = {
        baseUrl: typeof parsed.baseUrl === "string" ? parsed.baseUrl : defaults.baseUrl,
        model: typeof parsed.model === "string" ? parsed.model : defaults.model,
        size: typeof parsed.size === "string" ? parsed.size : defaults.size,
        quality: typeof parsed.quality === "string" ? parsed.quality : defaults.quality,
      };
      return { ...config, legacyApiKey: parsed.apiKey };
    }
  } catch {
    try { localStorage.removeItem(storageKeys.imageWorkbenchConfig); } catch { /* unavailable storage */ }
  }
  return defaults;
}

export function fileToDataUrl(file: File) {
  return new Promise<string>((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result));
    reader.onerror = () => reject(new Error("image.errors.input"));
    reader.readAsDataURL(file);
  });
}
