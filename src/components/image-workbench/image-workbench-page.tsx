import { useCallback, useContext, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { PageActiveContext } from "@/components/layout/page-stage";
import {
  Brush,
  Check,
  Copy,
  Download,
  Feather,
  FolderOpen,
  HardDriveDownload,
  History,
  ImagePlus,
  Loader2,
  Palette,
  Pin,
  PinOff,
  RefreshCw,
  Scroll,
  Search,
  Sparkles,
  Star,
  Trash2,
  ChevronDown,
  ChevronUp,
  Upload,
  X,
  ZoomIn,
} from "lucide-react";
import { api, type GalleryItem, type ImageGenerationResponse } from "@/lib/api";
import {
  PROMPT_LIBRARY,
  loadPromptFavorites,
  promptRecipeKey,
  savePromptFavorites,
  togglePromptFavorite,
  type PromptRecipe,
} from "@/lib/image-prompts";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { Badge } from "@/components/ui/badge";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
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
import { toast } from "@/hooks/use-toast";
import { formatRelative } from "@/lib/format-time";
import { isTauriRuntime } from "@/lib/tauri-runtime";
import { cn } from "@/lib/utils";

const IMAGE_KEY = ["newapi-state"] as const;
const GALLERY_KEY = ["image-gallery"] as const;
const GALLERY_THUMBNAIL_CACHE_LIMIT = 48;

type AspectRatioOption = {
  label: string;
  name: string;
  value: string;
  desc: string;
};

const SIZES: AspectRatioOption[] = [
  { label: "1:1 方幅", name: "1024x1024", value: "1024x1024", desc: "经典正方 · 头像/画作" },
  { label: "3:2 横卷", name: "1536x1024", value: "1536x1024", desc: "宽幅山水 · 桌面壁纸" },
  { label: "2:3 立轴", name: "1024x1536", value: "1024x1536", desc: "高挑挂轴 · 移动端屏保" },
  { label: "16:9 广角", name: "1792x1024", value: "1792x1024", desc: "全景巨卷 · 影视镜头" },
  { label: "9:16 宣幅", name: "1024x1792", value: "1024x1792", desc: "通景长轴 · 竖幅插画" },
  { label: "自选尺幅", name: "auto", value: "auto", desc: "默认比例 · 智能匹配" },
];

const QUALITIES = [
  { value: "auto", label: "自然点染 (auto)", tip: "标准平衡，适宜草绘" },
  { value: "high", label: "重彩精研 (high)", tip: "高清细腻，纤毫毕现" },
  { value: "medium", label: "雅致中品 (medium)", tip: "层次分明，质感均衡" },
  { value: "low", label: "意笔写意 (low)", tip: "轻快随形，迅速成画" },
];

type InkModelComboboxProps = {
  value: string;
  options: string[];
  onChange: (value: string) => void;
};

function InkModelCombobox({ value, options, onChange }: InkModelComboboxProps) {
  const rootRef = useRef<HTMLDivElement>(null);
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");

  const filteredOptions = useMemo(() => {
    const normalized = query.trim().toLocaleLowerCase();
    if (!normalized) return options;
    return options.filter(option => option.toLocaleLowerCase().includes(normalized));
  }, [options, query]);

  useEffect(() => {
    if (!open) return;
    const closeOnOutsidePress = (event: PointerEvent) => {
      if (!rootRef.current?.contains(event.target as Node)) setOpen(false);
    };
    document.addEventListener("pointerdown", closeOnOutsidePress);
    return () => document.removeEventListener("pointerdown", closeOnOutsidePress);
  }, [open]);

  const choose = (option: string) => {
    onChange(option);
    setQuery("");
    setOpen(false);
  };

  return (
    <div ref={rootRef} className="relative">
      <Input
        value={value}
        onChange={event => {
          onChange(event.target.value);
          setQuery(event.target.value);
          setOpen(true);
        }}
        onFocus={() => {
          setQuery("");
          setOpen(true);
        }}
        onKeyDown={event => {
          if (event.key === "Escape") setOpen(false);
          if (event.key === "Enter") setOpen(false);
          if (event.key === "ArrowDown") {
            event.preventDefault();
            setOpen(true);
            requestAnimationFrame(() => {
              rootRef.current?.querySelector<HTMLButtonElement>('[role="option"]')?.focus();
            });
          }
        }}
        role="combobox"
        aria-autocomplete="list"
        aria-expanded={open}
        aria-controls="image-model-options"
        placeholder="gpt-image-2"
        className="ink-model-input pr-10 font-mono text-sm"
      />
      <button
        type="button"
        aria-label={open ? "收起模型选项" : "展开模型选项"}
        aria-expanded={open}
        onMouseDown={event => event.preventDefault()}
        onClick={() => {
          setQuery("");
          setOpen(current => !current);
        }}
        className="absolute inset-y-0 right-0 flex w-10 items-center justify-center text-[#685d50] outline-none transition-colors hover:text-[#b83b38] focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-[#b83b38]/35 dark:text-[#cbbda8] dark:hover:text-[#efaaa7]"
      >
        <ChevronDown className={cn("h-4 w-4 transition-transform duration-200", open && "rotate-180")} />
      </button>
      {open && (
        <div
          id="image-model-options"
          role="listbox"
          aria-label="可用生图模型"
          className="ink-combobox-content"
        >
          <div className="ink-combobox-heading">可用墨相</div>
          <div className="max-h-52 overflow-y-auto p-1.5 ink-combobox-scroll">
            {filteredOptions.length > 0 ? filteredOptions.map(option => (
              <button
                key={option}
                type="button"
                role="option"
                aria-selected={option === value}
                onClick={() => choose(option)}
                className="ink-combobox-item"
              >
                <span className="min-w-0 flex-1 truncate font-mono text-sm">{option}</span>
                {option === value && <Check className="h-4 w-4 shrink-0 text-[#b83b38] dark:text-[#efaaa7]" />}
              </button>
            )) : (
              <p className="px-3 py-5 text-center font-serif text-xs text-muted-foreground">
                无匹配模型，可直接使用当前输入
              </p>
            )}
          </div>
          {query.trim() && !options.some(option => option.toLocaleLowerCase() === query.trim().toLocaleLowerCase()) && (
            <div className="ink-combobox-note">按 Enter 保留自定义模型名称</div>
          )}
        </div>
      )}
    </div>
  );
}

type PromptCategory = (typeof PROMPT_LIBRARY)[number];

type SavedImageConfig = {
  baseUrl: string;
  model: string;
  size: string;
  quality: string;
  legacyApiKey?: string;
};

type GenerationJob = {
  id: string;
  prompt: string;
  model: string;
  status: "queued" | "running" | "completed" | "failed";
  result?: ImageGenerationResponse;
  error?: string;
  galleryStatus?: "saving" | "saved" | "failed";
};

function loadSavedConfig(): SavedImageConfig {
  const defaults = { baseUrl: "", model: "gpt-image-2", size: "1024x1024", quality: "auto" };
  try {
    const saved = localStorage.getItem("image-workbench-config");
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
    try { localStorage.removeItem("image-workbench-config"); } catch { /* unavailable storage */ }
  }
  return defaults;
}

function fileToDataUrl(file: File) {
  return new Promise<string>((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result));
    reader.onerror = () => reject(new Error("image.errors.input"));
    reader.readAsDataURL(file);
  });
}

function GalleryThumbnail({ item, load, onPreview, opening }: {
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

export function ImageWorkbenchPage() {
  const { t } = useTranslation();
  const pageActive = useContext(PageActiveContext);
  const state = useQuery({ queryKey: IMAGE_KEY, queryFn: api.providers.loadState, refetchOnWindowFocus: false });
  const providers = state.data?.providers ?? [];
  const imageProviders = useMemo(
    () =>
      providers.filter(
        provider =>
          provider.hasApiKey &&
          ((!!provider.model && /image|dall|gpt-image/i.test(provider.model)) ||
            provider.models.some(model => /image|dall|gpt-image/i.test(model.id))),
      ),
    [providers],
  );

  const [providerId, setProviderId] = useState<string>("");
  const savedConfig = useMemo(loadSavedConfig, []);
  const [legacyKeyMigrationPending, setLegacyKeyMigrationPending] = useState(
    () => Boolean(savedConfig.legacyApiKey),
  );
  const [providerChoiceInitialized, setProviderChoiceInitialized] = useState(
    () => Boolean(savedConfig.baseUrl || savedConfig.legacyApiKey),
  );
  const [baseUrl, setBaseUrl] = useState(savedConfig.baseUrl);
  const [apiKey, setApiKey] = useState(savedConfig.legacyApiKey ?? "");
  const [keyStored, setKeyStored] = useState(false);
  const [model, setModel] = useState(savedConfig.model);
  const [prompt, setPrompt] = useState("");
  const [size, setSize] = useState(savedConfig.size);
  const [quality, setQuality] = useState(savedConfig.quality);
  const [count, setCount] = useState(1);
  const [imageData, setImageData] = useState<string | null>(null);
  const [generationJobs, setGenerationJobs] = useState<GenerationJob[]>([]);
  const [error, setError] = useState<string | null>(null);
  const queuedGenerations = useRef<Array<{ id: string; run: () => Promise<void> }>>([]);
  const activeGenerations = useRef(0);
  const gallerySaves = useRef(new Set<string>());
  const pumpGenerations = useRef<() => void>(() => {});
  const [activeCategory, setActiveCategory] = useState<string>(PROMPT_LIBRARY[0]?.category ?? "");
  const [promptQuery, setPromptQuery] = useState("");
  const [promptFavorites, setPromptFavorites] = useState<string[]>(() => loadPromptFavorites());
  const [savedGalleryDir, setSavedGalleryDir] = useState<string | null>(null);
  const [copiedPrompt, setCopiedPrompt] = useState<string | null>(null);
  const [expandedPrompt, setExpandedPrompt] = useState<string | null>(null);
  const [libraryCollapsed, setLibraryCollapsed] = useState(false);
  const [previewSrc, setPreviewSrc] = useState<string | null>(null);
  const thumbnailCache = useRef(new Map<string, string>());
  const thumbnailLoads = useRef(new Map<string, Promise<string>>());
  const gallerySession = useRef(0);
  const previewRequest = useRef(0);
  const [galleryPreviewPath, setGalleryPreviewPath] = useState<string | null>(null);
  const [pendingDelete, setPendingDelete] = useState<GalleryItem | null>(null);
  const [galleryOpen, setGalleryOpen] = useState(false);
  const queryClient = useQueryClient();

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
  const inputRef = useRef<HTMLInputElement>(null);

  const selectedProvider = providers.find(provider => provider.id === providerId);
  const availableModels = selectedProvider?.models.filter(item => /image|dall|gpt-image/i.test(item.id)) ?? [];
  const modelOptions = Array.from(new Set([
    "gpt-image-2",
    "gpt-image-2.5",
    ...availableModels.map(item => item.id),
  ]));

  useEffect(() => {
    if (!savedConfig.legacyApiKey || !savedConfig.baseUrl) return;
    void api.image.saveKey(savedConfig.baseUrl, savedConfig.legacyApiKey)
      .then(() => {
        try {
          localStorage.setItem("image-workbench-config", JSON.stringify({
            baseUrl: savedConfig.baseUrl, model: savedConfig.model,
            size: savedConfig.size, quality: savedConfig.quality,
          }));
        } catch {
          try { localStorage.removeItem("image-workbench-config"); } catch { /* unavailable storage */ }
        }
        setLegacyKeyMigrationPending(false);
        setKeyStored(true);
        setApiKey(current => current === savedConfig.legacyApiKey ? "" : current);
      })
      .catch(() => setError("image.errors.storage"));
  }, [savedConfig]);

  useEffect(() => {
    if (!baseUrl.trim()) { setKeyStored(false); return; }
    if (!pageActive) return;
    let active = true;
    setKeyStored(false);
    const timeout = window.setTimeout(() => void api.image.keyStatus(baseUrl.trim()).then(stored => {
      if (active) setKeyStored(stored);
    }).catch(() => {
      if (active) setKeyStored(false);
    }), 350);
    return () => { active = false; window.clearTimeout(timeout); };
  }, [baseUrl, pageActive]);

  useEffect(() => {
    if (providerId || legacyKeyMigrationPending) return;
    try {
      localStorage.setItem(
        "image-workbench-config",
        JSON.stringify({ baseUrl, model, size, quality } satisfies SavedImageConfig),
      );
    } catch { /* storage may be disabled */ }
  }, [baseUrl, legacyKeyMigrationPending, model, providerId, quality, size]);

  const saveKey = () => {
    const pending = apiKey.trim();
    if (!baseUrl.trim() || !pending) return;
    void api.image.saveKey(baseUrl, pending).then(() => {
      setLegacyKeyMigrationPending(false);
      setKeyStored(true);
      setApiKey(current => current === pending ? "" : current);
    }).catch(() => setError("image.errors.storage"));
  };

  useEffect(() => {
    if (providerChoiceInitialized || !state.isSuccess) return;
    setProviderChoiceInitialized(true);
    if (baseUrl || apiKey || imageProviders.length === 0) return;
    const provider = imageProviders[0];
    setProviderId(provider.id);
    setModel(
      provider.models.find(item => /image|dall|gpt-image/i.test(item.id))?.id ||
        provider.model ||
        "gpt-image-2",
    );
  }, [apiKey, baseUrl, imageProviders, providerChoiceInitialized, state.isSuccess]);

  const chooseProvider = (id: string) => {
    setProviderChoiceInitialized(true);
    setProviderId(id);
    const provider = providers.find(item => item.id === id);
    if (provider) {
      setModel(
        provider.models.find(item => /image|dall|gpt-image/i.test(item.id))?.id ||
          (/image|dall|gpt-image/i.test(provider.model) ? provider.model : "gpt-image-2"),
      );
    }
  };

  const upload = async (file: File | undefined) => {
    if (!file) return;
    try {
      setImageData(await fileToDataUrl(file));
      setError(null);
    } catch {
      setError("image.errors.input");
    }
  };

  const filteredPromptCategories = useMemo(() => {
    const q = promptQuery.trim().toLowerCase();
    if (!q) return PROMPT_LIBRARY;
    return PROMPT_LIBRARY.map(cat => ({
      ...cat,
      recipes: cat.recipes.filter(
        recipe =>
          recipe.title.toLowerCase().includes(q) ||
          recipe.tip.toLowerCase().includes(q) ||
          recipe.prompt.toLowerCase().includes(q) ||
          cat.category.toLowerCase().includes(q),
      ),
    })).filter(cat => cat.recipes.length > 0);
  }, [promptQuery]);

  const favoriteRecipes = useMemo(() => {
    const index = new Map<string, { category: string; recipe: PromptRecipe }>();
    for (const cat of PROMPT_LIBRARY) {
      for (const recipe of cat.recipes) {
        index.set(promptRecipeKey(cat.category, recipe), { category: cat.category, recipe });
      }
    }
    return promptFavorites
      .map(key => index.get(key))
      .filter((item): item is { category: string; recipe: PromptRecipe } => Boolean(item));
  }, [promptFavorites]);

  const visibleRecipes = useMemo(() => {
    const attach = (category: string, recipe: PromptRecipe) => ({
      category,
      recipe,
      key: promptRecipeKey(category, recipe),
    });
    if (promptQuery.trim()) {
      return filteredPromptCategories.flatMap(cat => cat.recipes.map(recipe => attach(cat.category, recipe)));
    }
    const pinned = favoriteRecipes.map(item => attach(item.category, item.recipe));
    const active = filteredPromptCategories.find(c => c.category === activeCategory)?.recipes ?? [];
    const pinnedKeys = new Set(pinned.map(item => item.key));
    const rest = active
      .map(recipe => attach(activeCategory, recipe))
      .filter(item => !pinnedKeys.has(item.key));
    return [...pinned, ...rest];
  }, [filteredPromptCategories, activeCategory, promptQuery, favoriteRecipes]);

  const toggleFavorite = (category: string, recipe: PromptRecipe) => {
    const key = promptRecipeKey(category, recipe);
    const next = togglePromptFavorite(promptFavorites, key);
    setPromptFavorites(next);
    savePromptFavorites(next);
  };

  useEffect(() => {
    if (promptQuery.trim()) return;
    if (PROMPT_LIBRARY.some(c => c.category === activeCategory)) return;
    setActiveCategory(PROMPT_LIBRARY[0]?.category ?? "");
  }, [promptQuery, activeCategory]);

  const canGenerate =
    prompt.trim().length > 0 &&
    model.trim().length > 0 &&
    Boolean(providerId || (baseUrl.trim() && (apiKey.trim() || keyStored)));

  const applyPromptRecipe = (recipe: PromptRecipe) => {
    setPrompt(recipe.prompt);
    setExpandedPrompt(recipe.prompt);
  };

  const copyPromptRecipe = async (recipe: PromptRecipe) => {
    try {
      await navigator.clipboard.writeText(recipe.prompt);
      setCopiedPrompt(recipe.prompt);
      setTimeout(() => setCopiedPrompt(null), 1600);
    } catch {
      applyPromptRecipe(recipe);
    }
  };

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

  const saveImageAs = async (src: string, index: number) => {
    if (!isTauriRuntime()) {
      const link = document.createElement("a");
      link.href = src;
      link.download = `agentharbor-ink-art-${Date.now()}-${index + 1}.png`;
      link.click();
      return;
    }
    try {
      const { save } = await import("@tauri-apps/plugin-dialog");
      const path = await save({
        defaultPath: `agentharbor-ink-art-${Date.now()}-${index + 1}.png`,
        filters: [{ name: "PNG", extensions: ["png"] }],
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

  const openGallery = async () => {
    setGalleryOpen(true);
    void queryClient.invalidateQueries({ queryKey: GALLERY_KEY });
  };

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

  pumpGenerations.current = () => {
    while (activeGenerations.current < 2 && queuedGenerations.current.length > 0) {
      const next = queuedGenerations.current.shift();
      if (!next) break;
      activeGenerations.current += 1;
      void next.run().finally(() => {
        activeGenerations.current -= 1;
        pumpGenerations.current();
      });
    }
  };

  const saveJobToGallery = async (id: string, images: string[]) => {
    if (gallerySaves.current.has(id)) return;
    gallerySaves.current.add(id);
    setGenerationJobs(current => current.map(job => job.id === id ? { ...job, galleryStatus: "saving" } : job));
    try {
      const saved = await persistToGallery(images);
      setGenerationJobs(current => current.map(job => job.id === id
        ? { ...job, galleryStatus: saved ? "saved" : "failed" }
        : job));
      if (saved) void queryClient.invalidateQueries({ queryKey: GALLERY_KEY });
    } finally {
      gallerySaves.current.delete(id);
    }
  };

  const generate = () => {
    if (!prompt.trim() || !model.trim()) return;
    const id = crypto.randomUUID();
    const request = {
      providerId: providerId || null,
      baseUrl,
      apiKey,
      model: model.trim(),
      prompt: prompt.trim(),
      size,
      quality,
      count,
      imageData,
    };
    const job: GenerationJob = {
      id,
      prompt: request.prompt,
      model: request.model,
      status: activeGenerations.current < 2 ? "running" : "queued",
    };
    setGenerationJobs(current => [job, ...current]);
    queuedGenerations.current.push({ id, run: async () => {
      setGenerationJobs(current => current.map(item => item.id === id ? { ...item, status: "running" } : item));
      try {
        const generated = await api.image.generate(request);
        setGenerationJobs(current => current.map(item => item.id === id
          ? { ...item, status: "completed", result: generated }
          : item));
        // A gallery failure stays visible on this canvas and can be retried.
        void saveJobToGallery(id, generated.images);
        if (!request.providerId && request.apiKey.trim()) {
          setKeyStored(true);
          setApiKey(current => current === request.apiKey ? "" : current);
        }
      } catch (cause) {
        const key = typeof cause === "string" ? cause : cause instanceof Error ? cause.message : "image.errors.network";
        setGenerationJobs(current => current.map(item => item.id === id
          ? { ...item, status: "failed", error: key.startsWith("image.") ? key : "image.errors.network" }
          : item));
      }
    } });
    pumpGenerations.current();
  };

  const removeCanvas = (job: GenerationJob) => {
    if (job.status === "running") return;
    if (job.status === "queued") {
      if (!queuedGenerations.current.some(queued => queued.id === job.id)) return;
      queuedGenerations.current = queuedGenerations.current.filter(queued => queued.id !== job.id);
    }
    setGenerationJobs(current => current.filter(item => item.id !== job.id));
  };

  return (
    <div className="space-y-6 overflow-y-auto pb-12">
      {/* 卷首 · 题额与闲章 */}
      <div className="relative overflow-hidden rounded-2xl border border-border/70 bg-gradient-to-r from-card via-card/90 to-card p-5 shadow-sm ink-paper-bg">
        {/* 背景水墨晕染光晕 */}
        <div
          aria-hidden
          className="pointer-events-none absolute -right-10 -top-20 h-64 w-64 rounded-full bg-gradient-to-bl from-amber-500/10 via-rose-500/5 to-transparent blur-3xl"
        />
        <div className="relative z-10 flex flex-wrap items-center justify-between gap-4">
          <div className="space-y-1.5">
            <div className="flex items-center gap-3">
              <div className="flex h-9 w-9 items-center justify-center rounded-xl border border-amber-600/30 bg-amber-500/10 text-amber-600 dark:border-amber-400/30 dark:bg-amber-400/10 dark:text-amber-400 shadow-xs">
                <Brush className="h-5 w-5" />
              </div>
              <div>
                <h1 className="font-serif text-xl font-bold tracking-wider text-foreground">
                  墨韵丹青 · GPT Image 工坊
                </h1>
              </div>
              <span className="ink-seal px-2 py-0.5 text-[11px] rounded">
                丹青妙笔
              </span>
            </div>
            <p className="font-serif text-xs text-muted-foreground tracking-wide sm:text-sm">
              落笔云烟，水墨天工 · 纳万象于方寸，融丹青于数字。支持 GPT Image 2 / 2.5 及兼容图像中转。
            </p>
          </div>
          <div className="flex flex-wrap items-center gap-2">
            <Button
              type="button"
              size="sm"
              variant="outline"
              className="h-8 gap-1.5 font-serif text-xs"
              onClick={() => void openGallery()}
            >
              <History className="h-3.5 w-3.5" />
              {t("image.galleryTitle")}
            </Button>
            {savedGalleryDir && (
              <Button
                type="button"
                size="sm"
                variant="ghost"
                className="h-8 gap-1.5 font-serif text-xs text-muted-foreground"
                onClick={() => void api.openPath(savedGalleryDir).catch(() => undefined)}
              >
                <FolderOpen className="h-3.5 w-3.5" />
                {t("image.openGallery")}
              </Button>
            )}
            <span className="inline-flex items-center gap-1.5 rounded-full border border-border/80 bg-background/80 px-3 py-1 text-xs text-muted-foreground backdrop-blur-xs font-serif shadow-xs">
              <Feather className="h-3.5 w-3.5 text-amber-600 dark:text-amber-400" />
              {t("image.promptSentToProvider")}
            </span>
            <span className="inline-flex items-center gap-1.5 rounded-full border border-border/80 bg-background/80 px-3 py-1 text-xs text-muted-foreground backdrop-blur-xs font-serif shadow-xs">
              <HardDriveDownload className="h-3.5 w-3.5 text-amber-600 dark:text-amber-400" />
              成图自动存入本地画册
            </span>
          </div>
        </div>
      </div>

      <div className="grid min-w-0 gap-4 sm:gap-6 xl:grid-cols-[minmax(0,1.15fr)_minmax(320px,0.85fr)]">
        {/* 左侧：笔墨设色 · 设色法度 (创作参数) */}
        <div className="ink-scroll-frame min-w-0 p-4 transition-all duration-300 sm:p-5 sm:p-6">
          <div className="space-y-5">
            {/* 栏目头部 */}
            <div className="flex items-center justify-between border-b border-border/50 pb-3">
              <div className="flex items-center gap-2">
                <Palette className="h-4 w-4 text-[#b83b38] dark:text-[#d94f4b]" />
                <h2 className="font-serif text-base font-bold tracking-wide text-foreground">
                  笔墨设色 · 创作法度
                </h2>
              </div>
              <span className="font-serif text-xs text-muted-foreground">
                随心构思，妙契自然
              </span>
            </div>

            {/* 参数网格 */}
            <div className="grid gap-4 sm:grid-cols-2">
              <div className="space-y-1.5 sm:col-span-2">
                <Label className="font-serif text-xs font-semibold text-foreground/90">
                  中转驿站 (供应商配置)
                </Label>
                <Select
                  value={providerId || "__manual__"}
                  onValueChange={value => chooseProvider(value === "__manual__" ? "" : value)}
                >
                  <SelectTrigger className="ink-select-trigger" aria-label="中转驿站供应商">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent className="ink-select-content" position="popper">
                    <SelectItem value="__manual__" className="ink-select-item">
                      {imageProviders.length === 0
                        ? "暂无可用图像中转 · 手动填写连接凭据"
                        : "独立配置 / 手动填写连接凭据"}
                    </SelectItem>
                    {imageProviders.map(provider => (
                      <SelectItem key={provider.id} value={provider.id} className="ink-select-item">
                        {provider.name}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                {imageProviders.length === 0 && (
                  <p className="font-serif text-[11px] text-muted-foreground">
                    可在「中转站」添加带图像模型的供应商，或在下方手动填写站点与密钥。
                  </p>
                )}
              </div>

              {!providerId && (
                <>
                  <div className="space-y-1.5">
                    <Label className="font-serif text-xs font-semibold text-foreground/90">
                      站点地址
                    </Label>
                    <Input
                      value={baseUrl}
                      onChange={event => setBaseUrl(event.target.value)}
                      placeholder="https://example.com"
                      className="bg-background/90"
                    />
                  </div>
                  <div className="space-y-1.5">
                    <Label className="font-serif text-xs font-semibold text-foreground/90">
                      API Key
                    </Label>
                    <Input
                      type="password"
                      value={apiKey}
                      onChange={event => setApiKey(event.target.value)}
                      onBlur={saveKey}
                      placeholder={keyStored ? "已保存密钥；留空则继续使用" : "sk-…"}
                      className="bg-background/90 font-mono"
                    />
                  </div>
                </>
              )}

              <div className="space-y-1.5">
                <Label className="font-serif text-xs font-semibold text-foreground/90">
                  绘意墨相 (模型)
                </Label>
                <InkModelCombobox value={model} options={modelOptions} onChange={setModel} />
              </div>

              <div className="space-y-1.5">
                <Label className="font-serif text-xs font-semibold text-foreground/90">
                  出图点数 (生图张数)
                </Label>
                <Input
                  type="number"
                  min={1}
                  max={4}
                  value={count}
                  onChange={event =>
                    setCount(Math.max(1, Math.min(4, Number(event.target.value) || 1)))
                  }
                  className="bg-background/90"
                />
              </div>

              <div className="space-y-1.5">
                <Label className="font-serif text-xs font-semibold text-foreground/90">
                  尺幅规制 (长宽尺寸)
                </Label>
                <Select value={size} onValueChange={setSize}>
                  <SelectTrigger className="ink-select-trigger" aria-label="尺幅规制">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent className="ink-select-content" position="popper">
                    {SIZES.map(item => (
                      <SelectItem key={item.value} value={item.value} className="ink-select-item">
                        {item.label} ({item.value})
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>

              <div className="space-y-1.5">
                <Label className="font-serif text-xs font-semibold text-foreground/90">
                  画质品阶 (精细度)
                </Label>
                <Select value={quality} onValueChange={setQuality}>
                  <SelectTrigger className="ink-select-trigger" aria-label="画质品阶">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent className="ink-select-content" position="popper">
                    {QUALITIES.map(item => (
                      <SelectItem key={item.value} value={item.value} className="ink-select-item">
                        <span className="flex flex-col gap-0.5">
                          <span>{item.label}</span>
                          <span className="text-[10px] text-muted-foreground">{item.tip}</span>
                        </span>
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
            </div>

            {/* 提示词输入区 */}
            <div className="space-y-2">
              <div className="flex items-center justify-between">
                <Label className="font-serif text-xs font-semibold text-foreground/90">
                  画题心象 (描绘词意)
                </Label>
                <span className="font-mono text-[11px] text-muted-foreground">
                  {prompt.length}/20000 字符
                </span>
              </div>
              <Textarea
                value={prompt}
                onChange={event => setPrompt(event.target.value)}
                placeholder="在此泼墨意气，描述画面主体、意境气韵、光影透视、构图章法与画派风格…"
                className="min-h-36 resize-y bg-background/90 font-serif leading-relaxed text-sm placeholder:text-muted-foreground/60 focus:border-[#b83b38]/50 focus:ring-1 focus:ring-[#b83b38]/30"
              />
            </div>

            {/* 锦囊诗意提示词预设 */}
            <div className="space-y-2.5 rounded-xl bg-black/[0.025] p-2.5 ring-1 ring-border/50 dark:bg-white/[0.02] sm:space-y-3 sm:p-3">
              <div className="flex flex-wrap items-center justify-between gap-x-2 gap-y-1 text-xs">
                <div className="flex items-center gap-2">
                  <span className="inline-flex min-w-0 items-center gap-1.5 font-serif font-semibold text-muted-foreground">
                    <Sparkles className="h-3.5 w-3.5 shrink-0 text-[#b83b38] dark:text-[#d94f4b]" />
                    <span className="truncate">锦囊画谱 · 工业级灵感库</span>
                    {promptFavorites.length > 0 && (
                      <span className="shrink-0 rounded-full bg-amber-500/15 px-1.5 py-px text-[10px] font-normal text-amber-700 dark:text-amber-300">
                        已藏 {promptFavorites.length}
                      </span>
                    )}
                  </span>
                </div>
                <div className="flex items-center gap-2">
                  <span className="hidden font-serif text-[11px] text-muted-foreground/80 sm:inline">
                    点选填入画题 · 复制可改写
                  </span>
                  <Button
                    type="button"
                    variant="ghost"
                    size="xs"
                    className="h-6 gap-1 px-1.5 text-[11px] text-muted-foreground hover:text-foreground"
                    onClick={() => setLibraryCollapsed(!libraryCollapsed)}
                  >
                    <span>{libraryCollapsed ? "展开画谱" : "收起画谱"}</span>
                    {libraryCollapsed ? <ChevronDown className="h-3.5 w-3.5" /> : <ChevronUp className="h-3.5 w-3.5" />}
                  </Button>
                </div>
              </div>

              {!libraryCollapsed && (
                <>
                  <div className="relative">
                    <Search className="pointer-events-none absolute left-2 top-1/2 h-3 w-3 -translate-y-1/2 text-muted-foreground/70" />
                    <input
                      value={promptQuery}
                      onChange={event => setPromptQuery(event.target.value)}
                      placeholder="检索画谱：人物 / Cosplay / 海报…"
                      className="h-8 w-full min-w-0 rounded-md border border-border/60 bg-background/80 pl-7 pr-7 text-xs outline-none transition-colors focus:border-[#b83b38]/40"
                      aria-label="检索提示词画谱"
                    />
                    {promptQuery && (
                      <button
                        type="button"
                        onClick={() => setPromptQuery("")}
                        className="absolute right-2 top-1/2 -translate-y-1/2 text-muted-foreground/70 hover:text-foreground"
                        aria-label="清除检索"
                      >
                        <X className="h-3 w-3" />
                      </button>
                    )}
                  </div>
                  {!promptQuery && (
                    <div
                      role="tablist"
                      aria-label="提示词分类"
                      className="flex flex-wrap gap-1 rounded-lg bg-background/45 p-1 ring-1 ring-border/45"
                    >
                      {filteredPromptCategories.map((cat: PromptCategory) => (
                        <button
                          key={cat.category}
                          type="button"
                          role="tab"
                          aria-selected={activeCategory === cat.category}
                          onClick={() => setActiveCategory(cat.category)}
                          className={cn(
                            "inline-flex h-7 shrink-0 items-center justify-center gap-1 whitespace-nowrap rounded-md px-2.5 font-serif text-[11px] outline-none transition-[background-color,color,box-shadow] focus-visible:ring-2 focus-visible:ring-[#b83b38]/35 focus-visible:ring-offset-1",
                            activeCategory === cat.category
                              ? "bg-background font-bold text-[#b83b38] shadow-xs ring-1 ring-[#b83b38]/20 dark:bg-white/[0.06] dark:text-[#fca5a5] dark:ring-[#d94f4b]/25"
                              : "text-muted-foreground hover:bg-muted/70 hover:text-foreground",
                          )}
                        >
                          <span>{cat.category}</span>
                          <span
                            className={cn(
                              "min-w-3.5 text-center font-sans text-[9px] tabular-nums opacity-60",
                              activeCategory === cat.category && "opacity-75",
                            )}
                          >
                            {cat.recipes.length}
                          </span>
                        </button>
                      ))}
                    </div>
                  )}
                  <div className="flex max-h-96 min-h-[9rem] flex-col gap-2 overflow-y-auto overscroll-contain pr-1">
                    {visibleRecipes.length === 0 && (
                      <p className="py-6 text-center font-serif text-xs text-muted-foreground">
                        画谱中没有匹配「{promptQuery}」的条目
                      </p>
                    )}
                    {visibleRecipes.map(({ category, recipe, key }) => {
                      const expanded = expandedPrompt === recipe.prompt;
                      const filled = prompt === recipe.prompt;
                      const starred = promptFavorites.includes(key);
                      return (
                        <div
                          key={key}
                          className={cn(
                            "shrink-0 rounded-lg border border-border/60 bg-background/70 p-2 transition-all sm:p-2.5",
                            filled
                              ? "border-[#b83b38]/45 bg-[#b83b38]/[0.04] dark:border-[#d94f4b]/40 dark:bg-[#d94f4b]/[0.06]"
                              : "hover:border-[#b83b38]/35",
                          )}
                        >
                          <div className="flex items-start justify-between gap-2">
                            <div className="min-w-0">
                              <p className="flex items-center gap-1 font-serif text-xs font-semibold text-foreground/90">
                                {starred && <Star className="h-3 w-3 fill-amber-400 text-amber-500" />}
                                {recipe.title}
                                {promptQuery.trim() && (
                                  <span className="ml-1 rounded bg-muted px-1 py-px text-[9px] font-normal text-muted-foreground">
                                    {category}
                                  </span>
                                )}
                              </p>
                              <p className="mt-0.5 font-serif text-[11px] leading-snug text-muted-foreground/85">
                                {recipe.tip}
                              </p>
                            </div>
                            <div className="flex shrink-0 items-center gap-0.5">
                              <Button
                                type="button"
                                size="icon"
                                variant="ghost"
                                className="h-7 w-7 text-muted-foreground hover:text-amber-500"
                                title={starred ? "取消收藏" : "收藏并置顶"}
                                aria-label={starred ? "取消收藏" : "收藏并置顶"}
                                onClick={() => toggleFavorite(category, recipe)}
                              >
                                {starred ? (
                                  <PinOff className="h-3.5 w-3.5" />
                                ) : (
                                  <Pin className="h-3.5 w-3.5" />
                                )}
                              </Button>
                              <Button
                                type="button"
                                size="icon"
                                variant="ghost"
                                className="h-7 w-7 text-muted-foreground hover:text-[#b83b38] dark:hover:text-[#fca5a5]"
                                title={filled ? "已填入" : "填入画题"}
                                onClick={() => applyPromptRecipe(recipe)}
                              >
                                {filled ? <Check className="h-3.5 w-3.5 text-emerald-500" /> : <Feather className="h-3.5 w-3.5" />}
                              </Button>
                              <Button
                                type="button"
                                size="icon"
                                variant="ghost"
                                className="h-7 w-7 text-muted-foreground hover:text-foreground"
                                title="复制提示词"
                                onClick={() => void copyPromptRecipe(recipe)}
                              >
                                {copiedPrompt === recipe.prompt ? (
                                  <Check className="h-3.5 w-3.5 text-emerald-500" />
                                ) : (
                                  <Copy className="h-3.5 w-3.5" />
                                )}
                              </Button>
                            </div>
                          </div>
                          <button
                            type="button"
                            className="mt-1 w-full text-left"
                            onClick={() => setExpandedPrompt(expanded ? null : recipe.prompt)}
                          >
                            <p
                              className={cn(
                                "whitespace-pre-wrap break-words font-serif text-[11px] leading-relaxed text-muted-foreground transition-all",
                                expanded ? "" : "line-clamp-2",
                              )}
                            >
                              {recipe.prompt}
                            </p>
                            <span className="mt-0.5 inline-block font-serif text-[10px] text-muted-foreground/60 hover:text-foreground">
                              {expanded ? "收起" : "展开全文"}
                            </span>
                          </button>
                        </div>
                      );
                    })}
                  </div>
                </>
              )}
            </div>

            {/* 图生图参考底本 */}
            <div className="rounded-xl border border-dashed border-border/80 bg-background/50 p-3.5 transition-colors">
              <div className="flex items-center justify-between gap-3">
                <div>
                  <p className="font-serif text-xs font-semibold text-foreground/90">
                    展卷摹画 · 载入底本 (可选)
                  </p>
                  <p className="font-serif text-[11px] text-muted-foreground">
                    支持 PNG / JPG / WebP 格式，单张上限 20 MB。
                  </p>
                </div>
                <div className="flex items-center gap-2">
                  <input
                    ref={inputRef}
                    className="hidden"
                    type="file"
                    accept="image/png,image/jpeg,image/webp"
                    onChange={event => void upload(event.target.files?.[0])}
                  />
                  <Button
                    type="button"
                    size="sm"
                    variant="outline"
                    className="font-serif text-xs h-8 gap-1.5"
                    onClick={() => inputRef.current?.click()}
                  >
                    <Upload className="h-3.5 w-3.5" />
                    选入画稿
                  </Button>
                  {imageData && (
                    <Button
                      type="button"
                      size="icon"
                      variant="ghost"
                      className="h-8 w-8 text-muted-foreground hover:text-destructive"
                      onClick={() => setImageData(null)}
                    >
                      <X className="h-4 w-4" />
                    </Button>
                  )}
                </div>
              </div>
              {imageData && (
                <div className="mt-3 relative rounded-lg border p-1 bg-card/60">
                  <img
                    src={imageData}
                    alt="待摹画底本"
                    className="max-h-48 rounded object-contain mx-auto"
                  />
                </div>
              )}
            </div>

            {error && (
              <div role="alert" className="rounded-xl border border-destructive/30 bg-destructive/5 p-3 text-xs text-destructive font-serif">
                {t(error)}
              </div>
            )}

            {/* 挥毫生图核心按钮 */}
            <Button
              data-image-generate
              className="w-full h-11 font-serif text-sm tracking-widest bg-gradient-to-r from-[#8b2323] via-[#b83b38] to-[#991b1b] hover:from-[#731c1c] hover:to-[#8b2323] text-white shadow-md hover:shadow-lg transition-all duration-300 dark:from-[#b83b38] dark:to-[#d94f4b]"
              disabled={!canGenerate}
              onClick={() => void generate()}
            >
              {generationJobs.some(job => job.status === "running" || job.status === "queued") ? (
                <>
                  <ImagePlus className="mr-2 h-4 w-4" />
                  {generationJobs.filter(job => job.status === "running").length >= 2
                    ? "新建画布 · 自动排队"
                    : "新建画布 · 并行生图"}
                </>
              ) : (
                <>
                  <Brush className="mr-2 h-4 w-4 animate-pulse" />
                  挥毫泼墨 · 显像生图
                </>
              )}
            </Button>
          </div>
        </div>

        {/* 右侧：宣纸卷轴 · 成果预览 (成果画廊) */}
        <div className="ink-scroll-frame flex flex-col p-5 sm:p-6 transition-all duration-300">
          <div className="flex items-center justify-between border-b border-border/50 pb-3">
            <div className="flex items-center gap-2">
              <Scroll className="h-4 w-4 text-amber-600 dark:text-amber-400" />
              <h2 className="font-serif text-base font-bold tracking-wide text-foreground">
                丹青画卷 · 成果真迹
              </h2>
            </div>
            {generationJobs.length > 0 && (
              <Badge variant="outline" className="font-mono text-xs border-amber-600/30 text-amber-600 dark:text-amber-400">
                {generationJobs.length} 个画布
              </Badge>
            )}
          </div>

          {/* 卷轴装饰线 */}
          <div className="my-2 h-0.5 w-full bg-gradient-to-r from-transparent via-[#c5a059]/40 to-transparent rounded-full" />

          {/* 结果展陈区 */}
          {generationJobs.length === 0 && (
            <div className="flex flex-1 min-h-[460px] flex-col items-center justify-center rounded-xl border border-dashed border-border/70 p-8 text-center ink-paper-bg">
              <div className="relative mb-4 flex h-16 w-16 items-center justify-center rounded-2xl border border-amber-500/20 bg-amber-500/5 text-amber-600 dark:border-amber-400/20 dark:text-amber-400">
                <ImagePlus className="h-8 w-8 stroke-[1.5]" />
                <div
                  aria-hidden
                  className="absolute -inset-1 rounded-2xl bg-amber-500/10 blur-sm animate-ink-bloom"
                />
              </div>
              <p className="font-serif text-base font-semibold tracking-wider text-foreground">
                素胚勾勒出青花，笔锋浓转淡
              </p>
              <p className="mt-1.5 font-serif text-xs text-muted-foreground max-w-xs leading-relaxed">
                研墨停云，虚席以待。请于左席填写画题心象并挥毫生图，丹青佳作将在此徐徐展开。
              </p>
            </div>
          )}

          {generationJobs.length > 0 && (
            <div className="mt-3 grid flex-1 content-start gap-4 xl:grid-cols-2">
              {generationJobs.map(job => (
                <section key={job.id} data-generation-job-status={job.status} className="min-w-0 space-y-3 rounded-2xl border border-border/70 bg-card/80 p-3 shadow-sm dark:border-[#c5a059]/30">
                  <div className="flex items-start justify-between gap-3 border-b border-border/50 pb-2">
                    <div className="min-w-0">
                      <p className="line-clamp-2 font-serif text-sm font-semibold">{job.prompt}</p>
                      <p className="mt-1 font-mono text-[11px] text-muted-foreground">{job.model}</p>
                    </div>
                    <div className="flex shrink-0 items-center gap-1">
                      <Badge variant="outline">
                        {job.status === "queued" ? "排队中" : job.status === "running" ? "生成中" : job.status === "failed" ? "失败" : "完成"}
                      </Badge>
                      {job.status !== "running" && (
                        <button type="button" onClick={() => removeCanvas(job)}
                          aria-label={job.status === "queued" ? "取消排队" : "关闭画布"}
                          title={job.status === "queued" ? "取消排队" : "关闭画布"}
                          className="rounded p-1 text-muted-foreground hover:bg-muted hover:text-foreground">
                          <X className="h-3.5 w-3.5" />
                        </button>
                      )}
                    </div>
                  </div>
                  {job.status === "completed" && job.galleryStatus && (
                    <div className="flex items-center gap-2 text-xs text-muted-foreground">
                      {job.galleryStatus === "saving" ? t("image.gallerySaving")
                        : job.galleryStatus === "saved" ? t("image.savedToGallery")
                          : t("image.gallerySaveFailed")}
                      {job.galleryStatus === "failed" && job.result && (
                        <button type="button" className="font-semibold text-primary hover:underline"
                          onClick={() => void saveJobToGallery(job.id, job.result!.images)}>
                          {t("image.galleryRetrySave")}
                        </button>
                      )}
                    </div>
                  )}
                  {job.status === "queued" || job.status === "running" ? (
                    <div className="flex min-h-56 flex-col items-center justify-center rounded-xl border border-dashed border-amber-500/30 bg-amber-500/[0.03] p-6 text-center">
                      <Loader2 className={cn("mb-3 h-8 w-8 text-amber-600 dark:text-amber-400", job.status === "running" && "animate-spin")} />
                      <p className="font-serif text-sm font-semibold">{job.status === "queued" ? "等待画位" : "墨韵晕染，点染丹青…"}</p>
                      <p className="mt-1 text-xs text-muted-foreground">{job.status === "queued" ? "前面的画卷完成后自动开始" : "正在依题赋彩，其他画布可继续并行生成"}</p>
                    </div>
                  ) : job.status === "failed" ? (
                    <div role="alert" className="flex min-h-56 items-center justify-center rounded-xl border border-destructive/30 bg-destructive/5 p-6 text-center text-sm text-destructive">
                      {t(job.error ?? "image.errors.network")}
                    </div>
                  ) : (
                    <div className="grid gap-3">
                {job.result?.images.map((src, index) => (
                  <div
                    key={`${src.slice(0, 24)}-${index}`}
                    className="group relative space-y-3 rounded-2xl border border-border/70 bg-card p-3 shadow-sm transition-all duration-300 hover:shadow-md dark:border-[#c5a059]/30"
                  >
                    {/* 装裱画框 */}
                    <button
                      type="button"
                      className="relative block w-full overflow-hidden rounded-xl border border-border/80 bg-background/80 p-2 shadow-inner"
                      onClick={() => showPreview(src)}
                      title="点击放大预览"
                    >
                      <img
                        src={src}
                        alt={`生成成果 ${index + 1}`}
                        className="w-full rounded-lg object-contain transition-transform duration-500 group-hover:scale-[1.01]"
                      />
                      <span className="absolute bottom-3 right-3 inline-flex items-center gap-1 rounded-md bg-black/55 px-2 py-1 text-[11px] text-white opacity-0 transition-opacity group-hover:opacity-100">
                        <ZoomIn className="h-3 w-3" />
                        放大
                      </span>
                    </button>

                    {/* 图片底部信息与下载操作 */}
                    <div className="flex flex-wrap items-center justify-between gap-2 px-1">
                      <span className="font-serif text-xs font-semibold text-muted-foreground">
                        画卷之 {index + 1}
                      </span>
                      <Button
                        type="button"
                        size="sm"
                        variant="outline"
                        className="h-8 gap-1.5 border-[#b83b38]/30 bg-[#b83b38]/10 font-serif text-xs font-semibold text-[#b83b38] hover:bg-[#b83b38]/20 dark:border-[#d94f4b]/30 dark:bg-[#d94f4b]/15 dark:text-[#fca5a5]"
                        onClick={() => void saveImageAs(src, index)}
                      >
                        <Download className="h-3.5 w-3.5" />
                        {t("image.saveAs")}
                      </Button>
                    </div>

                    {job.result?.revisedPrompts[index] && (
                      <div className="rounded-lg bg-black/[0.025] p-2.5 text-xs text-muted-foreground ring-1 ring-border/40 dark:bg-white/[0.02]">
                        <p className="font-serif font-semibold text-foreground/80 mb-0.5">
                          【画意题跋 · 润色提示词】
                        </p>
                        <p className="font-serif leading-relaxed text-[11px]">
                          {job.result.revisedPrompts[index]}
                        </p>
                      </div>
                    )}
                  </div>
                ))}
                    </div>
                  )}
                </section>
              ))}
              </div>
          )}
        </div>
      </div>

      {/* 历史画册 */}
      {galleryOpen && (
        <div className="ink-scroll-frame p-5 sm:p-6">
          <div className="mb-3 flex flex-wrap items-center justify-between gap-2 border-b border-border/50 pb-3">
            <div className="flex items-center gap-2">
              <History className="h-4 w-4 text-amber-600 dark:text-amber-400" />
              <h2 className="font-serif text-base font-bold tracking-wide text-foreground">
                {t("image.galleryTitle")}
              </h2>
              <Badge variant="outline" className="font-mono text-[11px]">
                {galleryItems.length}
              </Badge>
              {galleryQuery.data?.hasMore && (
                <span className="text-xs text-muted-foreground">{t("image.galleryLimit")}</span>
              )}
            </div>
            <div className="flex items-center gap-1.5">
              <Button
                type="button"
                size="sm"
                variant="ghost"
                className="h-8 gap-1 text-xs"
                onClick={() => void queryClient.invalidateQueries({ queryKey: GALLERY_KEY })}
              >
                <RefreshCw className={cn("h-3.5 w-3.5", galleryQuery.isFetching && "animate-spin")} />
                {t("image.galleryRefresh")}
              </Button>
              {savedGalleryDir && (
                <Button
                  type="button"
                  size="sm"
                  variant="ghost"
                  className="h-8 gap-1 text-xs"
                  onClick={() => void api.openPath(savedGalleryDir).catch(() => undefined)}
                >
                  <FolderOpen className="h-3.5 w-3.5" />
                  {t("image.openGallery")}
                </Button>
              )}
              <Button
                type="button"
                size="icon"
                variant="ghost"
                className="h-8 w-8"
                onClick={closeGallery}
                aria-label="收起画册"
              >
                <X className="h-4 w-4" />
              </Button>
            </div>
          </div>

          {galleryQuery.isLoading ? (
            <div className="grid gap-2 sm:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5">
              {Array.from({ length: 5 }).map((_, index) => (
                <div
                  key={index}
                  className="aspect-square animate-pulse rounded-lg border border-border/50 bg-muted/40"
                />
              ))}
            </div>
          ) : galleryQuery.isError ? (
            <p className="py-10 text-center font-serif text-sm text-destructive">
              {t("image.galleryLoadFailed")}
            </p>
          ) : galleryItems.length === 0 ? (
            <p className="py-10 text-center font-serif text-sm text-muted-foreground">
              {t("image.galleryEmpty")}
            </p>
          ) : (
            <div className="grid gap-2 sm:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5">
              {galleryItems.map((item, index) => {
                return (
                  <div
                    key={item.path}
                    className="group relative overflow-hidden rounded-lg border border-border/60 bg-card/70"
                  >
                    <GalleryThumbnail
                      item={item}
                      load={loadGalleryThumbnail}
                      onPreview={() => void openGalleryImage(item)}
                      opening={galleryPreviewPath === item.path}
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
                        onClick={() => void saveGalleryImageAs(item, index)}
                      >
                        <Download className="h-3.5 w-3.5" />
                      </Button>
                      <Button
                        type="button"
                        size="icon"
                        variant="secondary"
                        className="h-7 w-7 bg-black/55 text-white hover:bg-destructive"
                        title={t("image.galleryDelete")}
                        onClick={() => setPendingDelete(item)}
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
      )}

      <AlertDialog open={pendingDelete !== null} onOpenChange={open => !open && setPendingDelete(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("image.galleryDeleteConfirm")}</AlertDialogTitle>
            <AlertDialogDescription>
              {t("image.galleryDeleteConfirmDesc")}
              {pendingDelete ? ` (${pendingDelete.name})` : ""}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction onClick={() => void deleteGalleryItem()}>
              {t("image.galleryDelete")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      {previewSrc && (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/75 p-6"
          role="dialog"
          aria-modal="true"
          aria-label="成图放大预览"
          onClick={() => showPreview(null)}
        >
          <button
            type="button"
            className="absolute right-5 top-5 inline-flex h-9 w-9 items-center justify-center rounded-full bg-black/50 text-white hover:bg-black/70"
            onClick={() => showPreview(null)}
            aria-label="关闭预览"
          >
            <X className="h-4 w-4" />
          </button>
          <img
            src={previewSrc}
            alt="成图放大预览"
            className="max-h-[85vh] max-w-[92vw] rounded-lg object-contain shadow-2xl"
            onClick={event => event.stopPropagation()}
          />
        </div>
      )}
    </div>
  );
}
