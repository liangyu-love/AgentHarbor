import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Brush, ImagePlus, Palette } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { cn } from "@/lib/utils";
import { ConnectionPanel, useConnectionPanel } from "./connection-panel";
import { PromptLibrary } from "./prompt-library";
import { PromptAssistPanel, PromptAssistToggle } from "./prompt-assist";
import { SaveAsTemplateButton } from "./prompt-template-dialog";
import { ReferenceStrip } from "./reference-strip";
import { rememberedBook } from "./use-prompt-library";
import type { ImageConnection } from "./use-image-connection";
import { SIZES, type GenerationJob, type ReferenceImage } from "./workbench-types";

const COUNTS = [1, 2, 3, 4];

/**
 * The left scroll: a folding connection, then what is written and picked for each image
 * (prompt, polishing, references, the 画谱), with the parameters and the generate button
 * in a dock that stays at the bottom of the view while the scroll is longer than it.
 */
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
  const panel = useConnectionPanel(connection);
  const selectedQuality = connection.qualityOptions.find(item => item.value === connection.quality);
  const connected = connection.hasCredentials && connection.model.trim().length > 0;
  const busy = generationJobs.some(job => job.status === "running" || job.status === "queued");

  const revealConnection = () => {
    panel.setOpen(true);
    const reduced = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    document.querySelector("[data-image-connection]")?.scrollIntoView({ behavior: reduced ? "auto" : "smooth", block: "start" });
  };

  return (
    <div className="ink-scroll-frame min-w-0 p-4 transition-all duration-300 sm:p-6">
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

        <ConnectionPanel connection={connection} open={panel.open} onOpenChange={panel.setOpen} />

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
            onKeyDown={event => {
              if (event.key !== "Enter" || !(event.ctrlKey || event.metaKey) || event.nativeEvent.isComposing) return;
              event.preventDefault();
              if (canGenerate) onGenerate();
            }}
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

        <ReferenceStrip references={references} setReferences={setReferences} upload={upload} dropping={dropping} />

        <PromptLibrary kind={libraryKind} onKindChange={setLibraryKind} prompt={prompt} onPromptChange={setPrompt}
          references={references} onAddReferences={addReferences}
          onRemoveReference={removeReference} />
      </div>

      {/* 吸底：参数与挥毫生图 */}
      <div data-image-dock
        className="sticky bottom-0 z-20 -mx-4 -mb-4 mt-5 space-y-2 rounded-b-2xl border-t border-border/50 bg-[#fbf9f5]/95 px-4 pb-4 pt-3 backdrop-blur-md dark:bg-[#0f1013]/95 sm:-mx-6 sm:-mb-6 sm:px-6 sm:pb-5">
        {error && (
          <div role="alert" className="rounded-lg border border-destructive/30 bg-destructive/5 px-3 py-2 font-serif text-xs text-destructive">
            {t(error)}
          </div>
        )}
        <div className="flex flex-wrap items-center gap-2">
          <div role="radiogroup" aria-label="生成张数"
            className="flex h-9 shrink-0 items-center rounded-[0.65rem] bg-black/[0.04] p-0.5 ring-1 ring-border/50 dark:bg-white/[0.04]">
            {COUNTS.map(value => (
              <button key={value} type="button" role="radio" aria-checked={count === value} aria-label={`${value} 张`}
                onClick={() => setCount(value)}
                className={cn(
                  "h-8 w-8 rounded-[0.5rem] font-serif text-xs tabular-nums outline-none transition-colors focus-visible:ring-2 focus-visible:ring-[#b83b38]/35",
                  count === value
                    ? "bg-background font-bold text-[#b83b38] shadow-xs dark:bg-white/[0.08] dark:text-[#fca5a5]"
                    : "text-muted-foreground hover:text-foreground",
                )}>
                {value}
              </button>
            ))}
          </div>
          <Select value={connection.size} onValueChange={connection.setSize}>
            <SelectTrigger className="ink-select-trigger !h-9 !w-auto min-w-0 max-w-[13rem] flex-1 basis-36" aria-label="画幅尺寸">
              <SelectValue />
            </SelectTrigger>
            <SelectContent className="ink-select-content" position="popper" side="top">
              {SIZES.map(item => (
                <SelectItem key={item.value} value={item.value} className="ink-select-item">
                  {item.label} ({item.value})
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <Select value={connection.quality} onValueChange={connection.setQuality}>
            <SelectTrigger className="ink-select-trigger !h-9 !w-auto min-w-0 max-w-[8rem] flex-1 basis-20 items-center" aria-label="画质">
              <SelectValue>{selectedQuality?.label ?? connection.quality}</SelectValue>
            </SelectTrigger>
            <SelectContent className="ink-select-content" position="popper" side="top">
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

          {/* 挥毫生图核心按钮 */}
          <Button
            data-image-generate
            title="Ctrl + Enter 也可以生成"
            className="ink-generate-button ml-auto h-10 min-w-[9.5rem] flex-1 font-serif text-sm tracking-wider text-white sm:flex-none"
            disabled={!canGenerate}
            onClick={onGenerate}
          >
            {busy ? (
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
        {!canGenerate && (
          <p className="font-serif text-[11px] text-muted-foreground">
            {!connected ? (
              <>
                连接还没配好 ·{" "}
                <button type="button" onClick={revealConnection} className="text-[#b83b38] hover:underline dark:text-[#fca5a5]">
                  去配置
                </button>
              </>
            ) : "写下提示词后即可生成 · Ctrl + Enter 快速生成"}
          </p>
        )}
      </div>
    </div>
  );
}
