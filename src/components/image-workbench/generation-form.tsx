import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Brush, ImagePlus, Palette } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { SecretInput } from "@/components/ui/secret-input";
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
import { PromptAssistPanel, PromptAssistToggle } from "./prompt-assist";
import { SaveAsTemplateButton } from "./prompt-template-dialog";
import { ReferenceStrip } from "./reference-strip";
import { rememberedBook } from "./use-prompt-library";
import type { ImageConnection } from "./use-image-connection";
import { SIZES, type GenerationJob, type ReferenceImage } from "./workbench-types";

/** The left scroll: connection, parameters, prompt, reference images and the generate button. */
export function GenerationForm({
  connection, prompt, setPrompt, count, setCount, references, setReferences, addReferences, removeReference, upload, dropping,
  error, canGenerate, generationJobs, onGenerate,
}: {
  connection: ImageConnection;
  prompt: string;
  setPrompt: (prompt: string) => void;
  count: number;
  setCount: (count: number) => void;
  references: ReferenceImage[];
  setReferences: (references: ReferenceImage[]) => void;
  /** Append references; returns how many were added. */
  addReferences: (references: Omit<ReferenceImage, "key">[]) => number;
  removeReference: (key: string) => void;
  upload: (files: File[]) => Promise<void>;
  /** Pictures are being dragged over the workbench. */
  dropping: boolean;
  error: string | null;
  canGenerate: boolean;
  generationJobs: GenerationJob[];
  onGenerate: () => void;
}) {
  const { t } = useTranslation();
  const [assistOpen, setAssistOpen] = useState(false);
  const [libraryKind, setLibraryKind] = useState(rememberedBook);
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
                <SecretInput
                  value={connection.apiKey}
                  onChange={connection.setApiKey}
                  onBlur={connection.saveKey}
                  hasStoredSecret={connection.keyStored}
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
            <div className="flex items-center gap-2">
              <PromptAssistToggle open={assistOpen} onToggle={() => setAssistOpen(!assistOpen)} />
              <SaveAsTemplateButton prompt={prompt} />
              <span className="font-mono text-[11px] text-muted-foreground">
                {Array.from(prompt).length}/32000 字符
              </span>
            </div>
          </div>
          <Textarea
            data-image-prompt
            value={prompt}
            onChange={event => setPrompt(event.target.value)}
            placeholder="描述画面主体、构图、光线和风格…"
            className="min-h-36 resize-y bg-background/90 font-serif leading-relaxed text-sm placeholder:text-muted-foreground/60 focus:border-[#b83b38]/50 focus:ring-1 focus:ring-[#b83b38]/30"
          />
        </div>

        {assistOpen && (
          <PromptAssistPanel prompt={prompt} references={references} onPromptChange={setPrompt} onClose={() => setAssistOpen(false)}
            onManageMaterials={() => {
              setLibraryKind("materials");
              const reduced = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
              document.querySelector("[data-prompt-library]")?.scrollIntoView({ behavior: reduced ? "auto" : "smooth", block: "start" });
            }} />
        )}

        <PromptLibrary kind={libraryKind} onKindChange={setLibraryKind} prompt={prompt} onPromptChange={setPrompt}
          references={references} onAddReferences={addReferences}
          onRemoveReference={removeReference} />

        <ReferenceStrip references={references} setReferences={setReferences} upload={upload} dropping={dropping} />

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
