import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useQuery } from "@tanstack/react-query";
import {
  Brush,
  Download,
  Feather,
  ImagePlus,
  Loader2,
  Palette,
  Scroll,
  Upload,
  X,
} from "lucide-react";
import { api, type ImageGenerationResponse } from "@/lib/api";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { Badge } from "@/components/ui/badge";
import { cn } from "@/lib/utils";

const IMAGE_KEY = ["newapi-state"] as const;

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

const POETIC_CATEGORIES = [
  {
    category: "水墨山水",
    prompts: [
      "远山如黛，千峰积雪，孤舟蓑笠翁独钓寒江，大写意泼墨与留白，宋画气韵，清冷幽远，超高清宣纸肌理",
      "烟雨江南，徽派古村落，白墙黛瓦，乌篷船划破碧波涟漪，淡彩水墨，细雨蒙蒙，水气氤氲，水墨国画",
    ],
  },
  {
    category: "工笔重彩",
    prompts: [
      "宋代院体工笔花鸟，一枝红杏青苔湿，双雀争鸣，线条劲细如铁线描，石青石绿矿物设色，真丝绢本古朴沉静",
      "敦煌壁画飞天仙子，沥粉贴金，飘带翻飞于祥云彩霞间，斑驳矿彩质感，神圣庄严而灵动，千年壁画风韵",
    ],
  },
  {
    category: "赛博国风",
    prompts: [
      "新中式赛博朋克，云雾缥缈的飞檐古道观与全息霓虹相融，御剑行者独立于摩天飞檐，雨夜青石板反光，水墨与数码交融",
      "国风机械青龙，盘旋于雷云紫霄之上，鎏金龙鳞与青铜机甲交错，云气吞吐，极具东方神话工业科幻感",
    ],
  },
  {
    category: "文房禅意",
    prompts: [
      "中式禅意极简摄影，古拙粗陶茶盏，一缕清烟袅袅升起，一瓣山茶花落在湿润青石板，静谧工作室柔光，高贵沉静",
      "案头文房四宝，徽墨歙砚，羊毫狼毫笔锋毕现，一滴浓墨正在白宣纸上晕染扩散，微距特写，水墨艺术质感",
    ],
  },
];

type SavedImageConfig = {
  baseUrl: string;
  model: string;
  size: string;
  quality: string;
  legacyApiKey?: string;
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

export function ImageWorkbenchPage() {
  const { t } = useTranslation();
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
  const [baseUrl, setBaseUrl] = useState(savedConfig.baseUrl);
  const [apiKey, setApiKey] = useState(savedConfig.legacyApiKey ?? "");
  const [keyStored, setKeyStored] = useState(false);
  const [model, setModel] = useState(savedConfig.model);
  const [prompt, setPrompt] = useState("");
  const [size, setSize] = useState(savedConfig.size);
  const [quality, setQuality] = useState(savedConfig.quality);
  const [count, setCount] = useState(1);
  const [imageData, setImageData] = useState<string | null>(null);
  const [result, setResult] = useState<ImageGenerationResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [activeCategory, setActiveCategory] = useState<string>("水墨山水");
  const inputRef = useRef<HTMLInputElement>(null);

  const selectedProvider = providers.find(provider => provider.id === providerId);
  const availableModels = selectedProvider?.models.filter(item => /image|dall|gpt-image/i.test(item.id)) ?? [];

  useEffect(() => {
    // Scrub the legacy plaintext value before sending it to protected storage.
    try {
      localStorage.setItem("image-workbench-config", JSON.stringify({
        baseUrl: savedConfig.baseUrl, model: savedConfig.model,
        size: savedConfig.size, quality: savedConfig.quality,
      }));
    } catch {
      try { localStorage.removeItem("image-workbench-config"); } catch { /* unavailable storage */ }
    }
    if (!savedConfig.legacyApiKey || !savedConfig.baseUrl) return;
    void api.image.saveKey(savedConfig.baseUrl, savedConfig.legacyApiKey)
      .then(() => {
        setKeyStored(true);
        setApiKey(current => current === savedConfig.legacyApiKey ? "" : current);
      })
      .catch(() => setError("image.errors.storage"));
  }, [savedConfig]);

  useEffect(() => {
    if (!baseUrl.trim()) { setKeyStored(false); return; }
    let active = true;
    void api.image.keyStatus(baseUrl).then(stored => {
      if (active) setKeyStored(stored);
    }).catch(() => {
      if (active) setKeyStored(false);
    });
    return () => { active = false; };
  }, [baseUrl]);

  useEffect(() => {
    if (providerId) return;
    localStorage.setItem(
      "image-workbench-config",
      JSON.stringify({ baseUrl, model, size, quality } satisfies SavedImageConfig),
    );
  }, [baseUrl, model, providerId, quality, size]);

  const saveKey = () => {
    const pending = apiKey.trim();
    if (!baseUrl.trim() || !pending) return;
    void api.image.saveKey(baseUrl, pending).then(() => {
      setKeyStored(true);
      setApiKey(current => current === pending ? "" : current);
    }).catch(() => setError("image.errors.storage"));
  };

  useEffect(() => {
    if (providerId || baseUrl || apiKey || imageProviders.length === 0) return;
    const provider = imageProviders[0];
    setProviderId(provider.id);
    setModel(
      provider.models.find(item => /image|dall|gpt-image/i.test(item.id))?.id ||
        provider.model ||
        "gpt-image-2",
    );
  }, [apiKey, baseUrl, imageProviders, providerId]);

  const chooseProvider = (id: string) => {
    setProviderId(id);
    const provider = providers.find(item => item.id === id);
    if (provider) {
      setModel(
        provider.models.find(item => /image|dall|gpt-image/i.test(item.id))?.id ||
          (/image|dall|gpt-image/i.test(provider.model) ? provider.model : "gpt-image-2"),
      );
      setBaseUrl("");
      setApiKey("");
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

  const generate = async () => {
    if (busy || !prompt.trim() || !model.trim()) return;
    setBusy(true);
    setError(null);
    try {
      const generated = await api.image.generate({
          providerId: providerId || null,
          baseUrl,
          apiKey,
          model: model.trim(),
          prompt: prompt.trim(),
          size,
          quality,
          count,
          imageData,
        });
      setResult(generated);
      if (!providerId && apiKey.trim()) {
        setKeyStored(true);
        setApiKey("");
      }
    } catch (cause) {
      const key =
        typeof cause === "string" ? cause : cause instanceof Error ? cause.message : "image.errors.network";
      setError(key.startsWith("image.") ? key : "image.errors.network");
    } finally {
      setBusy(false);
    }
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
          <div className="flex items-center gap-2">
            <span className="inline-flex items-center gap-1.5 rounded-full border border-border/80 bg-background/80 px-3 py-1 text-xs text-muted-foreground backdrop-blur-xs font-serif shadow-xs">
              <Feather className="h-3.5 w-3.5 text-amber-600 dark:text-amber-400" />
              本地私享调用 · 绝不外泄提示词
            </span>
          </div>
        </div>
      </div>

      <div className="grid gap-6 xl:grid-cols-[minmax(0,1.15fr)_minmax(360px,0.85fr)]">
        {/* 左侧：笔墨设色 · 设色法度 (创作参数) */}
        <div className="ink-scroll-frame p-5 sm:p-6 transition-all duration-300">
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
                <select
                  className="h-9 w-full rounded-lg border border-input bg-background/90 px-3 text-sm transition-colors focus:border-primary focus:ring-1 focus:ring-primary/20"
                  value={providerId}
                  onChange={event => chooseProvider(event.target.value)}
                >
                  <option value="">独立配置 / 手动填写连接凭据</option>
                  {imageProviders.map(provider => (
                    <option key={provider.id} value={provider.id}>
                      {provider.name} · {provider.model}
                    </option>
                  ))}
                </select>
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
                <Input
                  list="image-model-list"
                  value={model}
                  onChange={event => setModel(event.target.value)}
                  placeholder="gpt-image-2"
                  className="bg-background/90 font-mono text-sm"
                />
                <datalist id="image-model-list">
                  <option value="gpt-image-2" />
                  <option value="gpt-image-2.5" />
                  {availableModels.map(item => (
                    <option key={item.id} value={item.id} />
                  ))}
                </datalist>
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
                <select
                  className="h-9 w-full rounded-lg border border-input bg-background/90 px-3 text-sm transition-colors focus:border-primary focus:ring-1 focus:ring-primary/20"
                  value={size}
                  onChange={event => setSize(event.target.value)}
                >
                  {SIZES.map(item => (
                    <option key={item.value} value={item.value}>
                      {item.label} ({item.value})
                    </option>
                  ))}
                </select>
              </div>

              <div className="space-y-1.5">
                <Label className="font-serif text-xs font-semibold text-foreground/90">
                  画质品阶 (精细度)
                </Label>
                <select
                  className="h-9 w-full rounded-lg border border-input bg-background/90 px-3 text-sm transition-colors focus:border-primary focus:ring-1 focus:ring-primary/20"
                  value={quality}
                  onChange={event => setQuality(event.target.value)}
                >
                  {QUALITIES.map(item => (
                    <option key={item.value} value={item.value}>
                      {item.label}
                    </option>
                  ))}
                </select>
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
            <div className="space-y-2 rounded-xl bg-black/[0.025] p-3 ring-1 ring-border/50 dark:bg-white/[0.02]">
              <div className="flex items-center justify-between text-xs">
                <span className="font-serif font-semibold text-muted-foreground">
                  锦囊画谱 · 灵感点题
                </span>
                <div className="flex gap-1">
                  {POETIC_CATEGORIES.map(cat => (
                    <button
                      key={cat.category}
                      type="button"
                      onClick={() => setActiveCategory(cat.category)}
                      className={cn(
                        "rounded-md px-2 py-0.5 font-serif text-[11px] transition-all",
                        activeCategory === cat.category
                          ? "bg-[#b83b38]/10 text-[#b83b38] dark:bg-[#d94f4b]/15 dark:text-[#fca5a5] font-bold"
                          : "text-muted-foreground hover:bg-muted/60 hover:text-foreground",
                      )}
                    >
                      {cat.category}
                    </button>
                  ))}
                </div>
              </div>
              <div className="flex flex-col gap-1.5 pt-1">
                {POETIC_CATEGORIES.find(c => c.category === activeCategory)?.prompts.map(item => (
                  <button
                    key={item}
                    type="button"
                    className="group flex items-start gap-1.5 rounded-lg border border-border/60 bg-background/70 p-2 text-left text-xs transition-all hover:border-[#b83b38]/40 hover:bg-[#b83b38]/5"
                    onClick={() => setPrompt(item)}
                  >
                    <Feather className="mt-0.5 h-3 w-3 shrink-0 text-muted-foreground group-hover:text-[#b83b38]" />
                    <span className="line-clamp-2 font-serif text-muted-foreground group-hover:text-foreground">
                      {item}
                    </span>
                  </button>
                ))}
              </div>
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
              className="w-full h-11 font-serif text-sm tracking-widest bg-gradient-to-r from-[#8b2323] via-[#b83b38] to-[#991b1b] hover:from-[#731c1c] hover:to-[#8b2323] text-white shadow-md hover:shadow-lg transition-all duration-300 dark:from-[#b83b38] dark:to-[#d94f4b]"
              disabled={busy || !prompt.trim() || !model.trim()}
              onClick={() => void generate()}
            >
              {busy ? (
                <>
                  <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                  丹青点染中… 研墨赋彩
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
            {result && (
              <Badge variant="outline" className="font-mono text-xs border-amber-600/30 text-amber-600 dark:text-amber-400">
                {result.model}
              </Badge>
            )}
          </div>

          {/* 卷轴装饰线 */}
          <div className="my-2 h-0.5 w-full bg-gradient-to-r from-transparent via-[#c5a059]/40 to-transparent rounded-full" />

          {/* 结果展陈区 */}
          {!result && !busy && (
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

          {busy && (
            <div className="flex flex-1 min-h-[460px] flex-col items-center justify-center rounded-xl border border-dashed border-amber-500/30 p-8 text-center ink-paper-bg">
              <div className="relative mb-5 flex h-16 w-16 items-center justify-center rounded-2xl border border-amber-500/30 bg-amber-500/10 text-amber-600 dark:text-amber-400">
                <Loader2 className="h-8 w-8 animate-spin" />
              </div>
              <p className="font-serif text-base font-bold tracking-widest text-foreground animate-pulse">
                墨韵晕染，点染丹青…
              </p>
              <p className="mt-1.5 font-serif text-xs text-muted-foreground">
                大匠无名，正在依题赋彩构筑神工，稍待片刻
              </p>
            </div>
          )}

          {result && !busy && (
            <div className="mt-3 flex-1 space-y-4">
              <div className="grid gap-4 sm:grid-cols-1">
                {result.images.map((src, index) => (
                  <div
                    key={`${src.slice(0, 24)}-${index}`}
                    className="group relative space-y-3 rounded-2xl border border-border/70 bg-card p-3 shadow-sm transition-all duration-300 hover:shadow-md dark:border-[#c5a059]/30"
                  >
                    {/* 装裱画框 */}
                    <div className="relative overflow-hidden rounded-xl border border-border/80 bg-background/80 p-2 shadow-inner">
                      <img
                        src={src}
                        alt={`生成成果 ${index + 1}`}
                        className="w-full rounded-lg object-contain transition-transform duration-500 group-hover:scale-[1.01]"
                      />
                    </div>

                    {/* 图片底部信息与下载操作 */}
                    <div className="flex flex-wrap items-center justify-between gap-2 px-1">
                      <span className="font-serif text-xs font-semibold text-muted-foreground">
                        画卷之 {index + 1}
                      </span>
                      <a
                        href={src}
                        download={`aimami-ink-art-${Date.now()}-${index + 1}.png`}
                        className="inline-flex items-center gap-1.5 rounded-lg border border-[#b83b38]/30 bg-[#b83b38]/10 px-3 py-1 font-serif text-xs font-semibold text-[#b83b38] transition-all hover:bg-[#b83b38]/20 dark:border-[#d94f4b]/30 dark:bg-[#d94f4b]/15 dark:text-[#fca5a5]"
                      >
                        <Download className="h-3.5 w-3.5" />
                        保存画卷真迹
                      </a>
                    </div>

                    {result.revisedPrompts[index] && (
                      <div className="rounded-lg bg-black/[0.025] p-2.5 text-xs text-muted-foreground ring-1 ring-border/40 dark:bg-white/[0.02]">
                        <p className="font-serif font-semibold text-foreground/80 mb-0.5">
                          【画意题跋 · 润色提示词】
                        </p>
                        <p className="font-serif leading-relaxed text-[11px]">
                          {result.revisedPrompts[index]}
                        </p>
                      </div>
                    )}
                  </div>
                ))}
              </div>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
