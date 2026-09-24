import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useQuery } from "@tanstack/react-query";
import { ImagePlus, Loader2, Sparkles, Upload, X } from "lucide-react";
import { api, type ImageGenerationResponse } from "@/lib/api";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { Badge } from "@/components/ui/badge";
import { AuroraCard } from "@/components/ui/aurora-card";

const IMAGE_KEY = ["newapi-state"] as const;
const SIZES = ["auto", "1024x1024", "1536x1024", "1024x1536", "1792x1024", "1024x1792"];
const QUALITY = ["auto", "low", "medium", "high"];
const builtInPrompts = [
  "产品摄影：一件极简白色陶瓷杯，柔和的工作室光线，米白色背景，电商主图，高清细节",
  "电影感概念艺术：雨夜霓虹街道，一位穿风衣的人撑伞走过，湿润路面反光，广角构图",
  "编辑一张照片：保留主体与脸部特征，把背景替换为自然的樱花公园，光线与透视保持一致",
];

type SavedImageConfig = {
  baseUrl: string;
  apiKey: string;
  model: string;
  size: string;
  quality: string;
};

function loadSavedConfig(): SavedImageConfig {
  try {
    const saved = localStorage.getItem("image-workbench-config");
    if (saved) return { baseUrl: "", apiKey: "", model: "gpt-image-2", size: "1024x1024", quality: "auto", ...JSON.parse(saved) as Partial<SavedImageConfig> };
  } catch {
    // Ignore malformed or unavailable browser storage.
  }
  return { baseUrl: "", apiKey: "", model: "gpt-image-2", size: "1024x1024", quality: "auto" };
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
  const imageProviders = useMemo(() => providers.filter(provider => provider.hasApiKey && (!!provider.model && /image|dall|gpt-image/i.test(provider.model) || provider.models.some(model => /image|dall|gpt-image/i.test(model.id)))), [providers]);
  const [providerId, setProviderId] = useState<string>("");
  const savedConfig = useMemo(loadSavedConfig, []);
  const [baseUrl, setBaseUrl] = useState(savedConfig.baseUrl);
  const [apiKey, setApiKey] = useState(savedConfig.apiKey);
  const [model, setModel] = useState(savedConfig.model);
  const [prompt, setPrompt] = useState("");
  const [size, setSize] = useState(savedConfig.size);
  const [quality, setQuality] = useState(savedConfig.quality);
  const [count, setCount] = useState(1);
  const [imageData, setImageData] = useState<string | null>(null);
  const [result, setResult] = useState<ImageGenerationResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);
  const selectedProvider = providers.find(provider => provider.id === providerId);
  const availableModels = selectedProvider?.models.filter(item => /image|dall|gpt-image/i.test(item.id)) ?? [];

  useEffect(() => {
    if (providerId) return;
    localStorage.setItem("image-workbench-config", JSON.stringify({ baseUrl, apiKey, model, size, quality } satisfies SavedImageConfig));
  }, [apiKey, baseUrl, model, providerId, quality, size]);

  useEffect(() => {
    if (providerId || baseUrl || apiKey || imageProviders.length === 0) return;
    const provider = imageProviders[0];
    setProviderId(provider.id);
    setModel(provider.models.find(item => /image|dall|gpt-image/i.test(item.id))?.id || provider.model || "gpt-image-2");
  }, [apiKey, baseUrl, imageProviders, providerId]);

  const chooseProvider = (id: string) => {
    setProviderId(id);
    const provider = providers.find(item => item.id === id);
    if (provider) {
      setModel(provider.models.find(item => /image|dall|gpt-image/i.test(item.id))?.id || (/image|dall|gpt-image/i.test(provider.model) ? provider.model : "gpt-image-2"));
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
      setResult(await api.image.generate({
        providerId: providerId || null,
        baseUrl,
        apiKey,
        model: model.trim(),
        prompt: prompt.trim(),
        size,
        quality,
        count,
        imageData,
      }));
    } catch (cause) {
      const key = typeof cause === "string" ? cause : cause instanceof Error ? cause.message : "image.errors.network";
      setError(key.startsWith("image.") ? key : "image.errors.network");
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="space-y-6 overflow-y-auto pb-8">
      <div className="flex flex-wrap items-start justify-between gap-4">
        <div>
          <div className="flex items-center gap-2"><ImagePlus className="h-5 w-5 text-primary" /><h1 className="text-lg font-semibold">GPT Image 工作台</h1></div>
          <p className="mt-1 text-sm text-muted-foreground">支持 GPT Image 2 / 2.5 及兼容 OpenAI Images API 的中转，生成与图生图共用一套配置。</p>
        </div>
        <Badge variant="secondary">本地调用 · 不上传到第三方提示词站</Badge>
      </div>

      <div className="grid gap-4 xl:grid-cols-[minmax(0,1fr)_minmax(320px,420px)]">
        <AuroraCard color="purple" className="p-5">
          <div className="space-y-4">
            <div className="flex items-center gap-2"><Sparkles className="h-4 w-4 text-primary" /><h2 className="font-semibold">创作参数</h2></div>
            <div className="grid gap-3 sm:grid-cols-2">
              <div className="space-y-1.5 sm:col-span-2"><Label>中转卡片</Label><select className="h-9 w-full rounded-md border bg-background px-3 text-sm" value={providerId} onChange={event => chooseProvider(event.target.value)}><option value="">独立配置 / 手动填写</option>{imageProviders.map(provider => <option key={provider.id} value={provider.id}>{provider.name} · {provider.model}</option>)}</select></div>
              {!providerId && <><div className="space-y-1.5"><Label>站点地址</Label><Input value={baseUrl} onChange={event => setBaseUrl(event.target.value)} placeholder="https://example.com" /></div><div className="space-y-1.5"><Label>API Key</Label><Input type="password" value={apiKey} onChange={event => setApiKey(event.target.value)} placeholder="sk-…" /></div></>}
              <div className="space-y-1.5"><Label>模型</Label><Input list="image-model-list" value={model} onChange={event => setModel(event.target.value)} placeholder="gpt-image-2" /><datalist id="image-model-list"><option value="gpt-image-2" /><option value="gpt-image-2.5" />{availableModels.map(item => <option key={item.id} value={item.id} />)}</datalist></div>
              <div className="space-y-1.5"><Label>数量</Label><Input type="number" min={1} max={4} value={count} onChange={event => setCount(Math.max(1, Math.min(4, Number(event.target.value) || 1)))} /></div>
              <div className="space-y-1.5"><Label>尺寸</Label><select className="h-9 w-full rounded-md border bg-background px-3 text-sm" value={size} onChange={event => setSize(event.target.value)}>{SIZES.map(value => <option key={value}>{value}</option>)}</select></div>
              <div className="space-y-1.5"><Label>质量</Label><select className="h-9 w-full rounded-md border bg-background px-3 text-sm" value={quality} onChange={event => setQuality(event.target.value)}>{QUALITY.map(value => <option key={value}>{value}</option>)}</select></div>
            </div>
            <div className="space-y-1.5"><div className="flex items-center justify-between"><Label>提示词</Label><span className="text-[11px] text-muted-foreground">{prompt.length}/20000</span></div><Textarea value={prompt} onChange={event => setPrompt(event.target.value)} placeholder="描述画面、主体、风格、构图与文字要求…" className="min-h-36 resize-y" /></div>
            <div className="flex flex-wrap gap-2">{builtInPrompts.map(item => <button key={item} type="button" className="rounded-full border px-2.5 py-1 text-[11px] text-muted-foreground hover:bg-muted" onClick={() => setPrompt(item)}>{item.slice(0, 22)}…</button>)}</div>
            <div className="rounded-xl border border-dashed p-3"><div className="flex items-center justify-between gap-3"><div><p className="text-sm font-medium">图生图输入（可选）</p><p className="text-xs text-muted-foreground">PNG / JPG / WebP，单张不超过 20 MB。</p></div><div className="flex items-center gap-2"><input ref={inputRef} className="hidden" type="file" accept="image/png,image/jpeg,image/webp" onChange={event => void upload(event.target.files?.[0])} /><Button type="button" size="sm" variant="outline" onClick={() => inputRef.current?.click()}><Upload className="mr-1.5 h-3.5 w-3.5" />上传</Button>{imageData && <Button type="button" size="icon" variant="ghost" onClick={() => setImageData(null)}><X className="h-4 w-4" /></Button>}</div></div>{imageData && <img src={imageData} alt="待编辑图片" className="mt-3 max-h-44 rounded-lg object-contain" />}</div>
            {error && <p role="alert" className="text-sm text-destructive">{t(error)}</p>}
            <Button className="w-full" disabled={busy || !prompt.trim() || !model.trim()} onClick={() => void generate()}>{busy ? <Loader2 className="mr-2 h-4 w-4 animate-spin" /> : <Sparkles className="mr-2 h-4 w-4" />}生成图片</Button>
          </div>
        </AuroraCard>

        <AuroraCard color="blue" className="p-5">
          <div className="flex items-center justify-between gap-2"><h2 className="font-semibold">结果预览</h2>{result && <Badge variant="secondary">{result.model}</Badge>}</div>
          {!result ? <div className="flex min-h-[420px] items-center justify-center rounded-xl border border-dashed text-center text-sm text-muted-foreground">生成结果会显示在这里</div> : <div className="mt-4 grid gap-3 sm:grid-cols-2">{result.images.map((src, index) => <div key={`${src.slice(0, 24)}-${index}`} className="space-y-1.5"><img src={src} alt={`生成结果 ${index + 1}`} className="w-full rounded-xl border object-contain" /><a className="inline-flex text-xs text-primary hover:underline" href={src} download={`aimami-image-${index + 1}.png`}>下载图片</a>{result.revisedPrompts[index] && <p className="text-[11px] text-muted-foreground">{result.revisedPrompts[index]}</p>}</div>)}</div>}
        </AuroraCard>
      </div>
    </div>
  );
}
