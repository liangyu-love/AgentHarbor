import { useRef } from "react";
import { useTranslation } from "react-i18next";
import { Brush, ImagePlus, Palette, Upload, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { InkModelCombobox } from "./ink-model-combobox";
import { PromptLibrary } from "./prompt-library";
import type { ImageConnection } from "./use-image-connection";
import { SIZES, type GenerationJob } from "./workbench-types";

/** The left scroll: connection, parameters, prompt, reference image and the generate button. */
export function GenerationForm({
  connection, prompt, setPrompt, count, setCount, imageData, setImageData, upload,
  error, canGenerate, generationJobs, onGenerate,
}: {
  connection: ImageConnection;
  prompt: string;
  setPrompt: (prompt: string) => void;
  count: number;
  setCount: (count: number) => void;
  imageData: string | null;
  setImageData: (data: string | null) => void;
  upload: (file: File | undefined) => Promise<void>;
  error: string | null;
  canGenerate: boolean;
  generationJobs: GenerationJob[];
  onGenerate: () => void;
}) {
  const { t } = useTranslation();
  const inputRef = useRef<HTMLInputElement>(null);
  const selectedQuality = connection.qualityOptions.find(item => item.value === connection.quality);
  return (
    <div className="ink-scroll-frame min-w-0 p-4 transition-all duration-300 sm:p-5 sm:p-6">
      <div className="space-y-5">
        {/* 栏目头部 */}
        <div className="flex items-center justify-between border-b border-border/50 pb-3">
          <div className="flex items-center gap-2">
            <Palette className="h-4 w-4 text-[#b83b38] dark:text-[#d94f4b]" />
            <h2 className="font-serif text-base font-bold tracking-wide text-foreground">
              笔墨设色
            </h2>
          </div>
        </div>

        {/* 参数网格 */}
        <div className="grid gap-4 sm:grid-cols-2">
          <div className="space-y-1.5 sm:col-span-2">
            <Label className="font-serif text-xs font-semibold text-foreground/90">
              图像供应商
            </Label>
            <Select
              value={connection.providerId || "__manual__"}
              onValueChange={value => connection.chooseProvider(value === "__manual__" ? "" : value)}
            >
              <SelectTrigger className="ink-select-trigger" aria-label="图像供应商">
                <SelectValue />
              </SelectTrigger>
              <SelectContent className="ink-select-content" position="popper">
                <SelectItem value="__manual__" className="ink-select-item">
                  {connection.imageProviders.length === 0
                    ? "暂无可用图像中转 · 手动填写连接凭据"
                    : "独立配置 / 手动填写连接凭据"}
                </SelectItem>
                {connection.imageProviders.map(provider => (
                  <SelectItem key={provider.id} value={provider.id} className="ink-select-item">
                    {provider.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            {connection.imageProviders.length === 0 && (
              <p className="font-serif text-[11px] text-muted-foreground">
                可在「中转站」添加带图像模型的供应商，或在下方手动填写站点与密钥。
              </p>
            )}
          </div>

          {!connection.providerId && (
            <>
              <div className="space-y-1.5">
                <Label className="font-serif text-xs font-semibold text-foreground/90">
                  站点地址
                </Label>
                <Input
                  value={connection.baseUrl}
                  onChange={event => connection.setBaseUrl(event.target.value)}
                  placeholder="https://example.com"
                  className="bg-background/90"
                  list={connection.keySites.length > 0 ? "image-key-sites" : undefined}
                />
                {connection.keySites.length > 0 && (
                  <datalist id="image-key-sites">
                    {connection.keySites.map(site => <option key={site} value={site} />)}
                  </datalist>
                )}
              </div>
              <div className="space-y-1.5">
                <Label className="font-serif text-xs font-semibold text-foreground/90">
                  API Key
                </Label>
                <Input
                  type="password"
                  value={connection.apiKey}
                  onChange={event => connection.setApiKey(event.target.value)}
                  onBlur={connection.saveKey}
                  placeholder={connection.keyStored ? "已保存密钥；留空则继续使用" : "sk-…"}
                  className="bg-background/90 font-mono"
                />
                {connection.keyStored && (
                  <button type="button" onClick={connection.forgetKey}
                    className="text-[11px] text-muted-foreground hover:text-destructive hover:underline">
                    忘记此站点的密钥
                  </button>
                )}
              </div>
            </>
          )}

          <div className="space-y-1.5">
            <Label className="font-serif text-xs font-semibold text-foreground/90">
              模型
            </Label>
            <InkModelCombobox value={connection.model} options={connection.modelOptions} onChange={connection.setModel} />
          </div>

          <div className="space-y-1.5">
            <Label className="font-serif text-xs font-semibold text-foreground/90">
              生成张数
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
              画幅尺寸
            </Label>
            <Select value={connection.size} onValueChange={connection.setSize}>
              <SelectTrigger className="ink-select-trigger" aria-label="画幅尺寸">
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
              画质
            </Label>
            <Select value={connection.quality} onValueChange={connection.setQuality}>
              <SelectTrigger className="ink-select-trigger items-center" aria-label="画质">
                <SelectValue>{selectedQuality?.label ?? connection.quality}</SelectValue>
              </SelectTrigger>
              <SelectContent className="ink-select-content" position="popper">
                {connection.qualityOptions.map(item => (
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
              提示词
            </Label>
            <span className="font-mono text-[11px] text-muted-foreground">
              {Array.from(prompt).length}/32000 字符
            </span>
          </div>
          <Textarea
            value={prompt}
            onChange={event => setPrompt(event.target.value)}
            placeholder="描述画面主体、构图、光线和风格…"
            className="min-h-36 resize-y bg-background/90 font-serif leading-relaxed text-sm placeholder:text-muted-foreground/60 focus:border-[#b83b38]/50 focus:ring-1 focus:ring-[#b83b38]/30"
          />
        </div>

        <PromptLibrary prompt={prompt} onPromptChange={setPrompt} />

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
                  aria-label="移除画稿"
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
          className="ink-generate-button h-11 w-full font-serif text-sm tracking-wider text-white"
          disabled={!canGenerate}
          onClick={onGenerate}
        >
          {generationJobs.some(job => job.status === "running" || job.status === "queued") ? (
            <>
              <ImagePlus className="mr-2 h-4 w-4" />
              {generationJobs.filter(job => job.status === "running").length >= 2
                ? "新建画布（排队）"
                : "新建画布"}
            </>
          ) : (
            <>
              <Brush className="mr-2 h-4 w-4" />
              开始生成
            </>
          )}
        </Button>
      </div>
    </div>
  );
}
