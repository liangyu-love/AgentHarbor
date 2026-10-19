import { useTranslation } from "react-i18next";
import { Download, Feather, ImagePlus, ImageUp, Loader2, Scroll, Square, X, ZoomIn } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import type { PreviewEntry } from "./use-image-gallery";
import { qualityLabel, sizeLabel, type GenerationJob } from "./workbench-types";

/** A finished job's images as the viewer steps through them, each with what made it. */
const previewEntries = (job: GenerationJob): PreviewEntry[] =>
  (job.result?.images ?? []).map((src, index) => ({
    key: `${job.id}-${index}`,
    src,
    meta: {
      prompt: job.prompt,
      revisedPrompt: job.result?.revisedPrompts[index] || null,
      model: job.result?.model || job.model,
      size: job.size,
      quality: job.quality,
    },
  }));

/** The result scroll: one canvas per generation job, with its status and images. */
export function GenerationCanvas({ generationJobs, onCancel, onRemove, onRetrySave, onPreview, onSaveAs, onUsePrompt, onUseAsReference }: {
  generationJobs: GenerationJob[];
  onCancel: (job: GenerationJob) => void;
  onRemove: (job: GenerationJob) => void;
  onRetrySave: (id: string, images: string[]) => Promise<void>;
  onPreview: (entries: PreviewEntry[], index: number) => void;
  onSaveAs: (src: string, index: number) => Promise<void>;
  onUsePrompt: (prompt: string) => void;
  /** Load an image as the next generation's reference. */
  onUseAsReference: (src: string) => void;
}) {
  const { t } = useTranslation();
  return (
    <div className="ink-scroll-frame flex flex-col p-5 sm:p-6 transition-all duration-300">
      <div className="flex items-center justify-between border-b border-border/50 pb-3">
        <div className="flex items-center gap-2">
          <Scroll className="h-4 w-4 text-amber-600 dark:text-amber-400" />
          <h2 className="font-serif text-base font-bold tracking-wide text-foreground">
            丹青画卷
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
            画布尚空
          </p>
          <p className="mt-1.5 font-serif text-xs text-muted-foreground max-w-xs leading-relaxed">
            输入提示词并开始生成，图片会显示在这里。
          </p>
        </div>
      )}

      {generationJobs.length > 0 && (
        <div className="ink-jobs-grid mt-3 flex-1 content-start gap-4">
          {generationJobs.map(job => (
            <section key={job.id} data-generation-job-status={job.status} className="min-w-0 space-y-3 rounded-2xl border border-border/70 bg-card/80 p-3 dark:border-[#c5a059]/30">
              <div className="flex items-start justify-between gap-3 border-b border-border/50 pb-2">
                <div className="min-w-0">
                  <p className="line-clamp-2 font-serif text-sm font-semibold" title={job.prompt}>{job.prompt}</p>
                  <p className="mt-1 flex flex-wrap items-center gap-x-1.5 text-[11px] text-muted-foreground">
                    <span className="font-mono">{job.model}</span>
                    <span aria-hidden>·</span>
                    <span>{sizeLabel(job.size)}</span>
                    <span aria-hidden>·</span>
                    <span>画质 {qualityLabel(job.quality)}</span>
                  </p>
                </div>
                <div className="flex shrink-0 items-center gap-1">
                  <button type="button" onClick={() => onUsePrompt(job.prompt)}
                    aria-label="复用这条提示词" title="复用这条提示词"
                    className="rounded p-1 text-muted-foreground hover:bg-muted hover:text-[#b83b38] dark:hover:text-[#fca5a5]">
                    <Feather className="h-3.5 w-3.5" />
                  </button>
                  <Badge variant="outline">
                    {job.status === "queued" ? "排队中" : job.status === "running" ? "生成中" : job.status === "failed" ? "失败" : job.status === "cancelled" ? "已取消" : "完成"}
                  </Badge>
                  {job.status === "running" && (
                    <button type="button" onClick={() => onCancel(job)}
                      aria-label="取消生成" title="取消生成"
                      className="rounded p-1 text-muted-foreground hover:bg-muted hover:text-destructive">
                      <Square className="h-3.5 w-3.5" />
                    </button>
                  )}
                  {job.status !== "running" && (
                    <button type="button" onClick={() => onRemove(job)}
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
                      onClick={() => void onRetrySave(job.id, job.result!.images)}>
                      {t("image.galleryRetrySave")}
                    </button>
                  )}
                </div>
              )}
              {job.status === "queued" || job.status === "running" ? (
                <div className="flex min-h-56 flex-col items-center justify-center rounded-xl border border-dashed border-amber-500/30 bg-amber-500/[0.03] p-6 text-center">
                  <Loader2 className={cn("mb-3 h-8 w-8 text-amber-600 dark:text-amber-400", job.status === "running" && "animate-spin")} />
                  <p className="font-serif text-sm font-semibold">{job.status === "queued" ? "排队中" : "正在生成"}</p>
                  <p className="mt-1 text-xs text-muted-foreground">{job.status === "queued" ? "前一个任务完成后自动开始" : "可以继续创建其他画布，或点右上角停止"}</p>
                </div>
              ) : job.status === "cancelled" ? (
                <div className="flex min-h-56 items-center justify-center rounded-xl border border-dashed border-border/70 p-6 text-center text-sm text-muted-foreground">
                  已取消生成
                </div>
              ) : job.status === "failed" ? (
                <div role="alert" className="flex min-h-56 items-center justify-center rounded-xl border border-destructive/30 bg-destructive/5 p-6 text-center text-sm text-destructive">
                  {t(job.error ?? "image.errors.network")}
                </div>
              ) : (
                <div className="ink-result-images gap-3">
            {job.result?.images.map((src, index) => (
              <div
                key={`${src.slice(0, 24)}-${index}`}
                className="group relative min-w-0 space-y-2"
              >
                {/* 装裱画框 */}
                <button
                  type="button"
                  className="relative block w-full overflow-hidden rounded-xl border border-border/80 bg-background/80 p-1.5 transition-[border-color,box-shadow] duration-200 hover:border-[#b83b38]/35 hover:shadow-sm"
                  onClick={() => onPreview(previewEntries(job), index)}
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
                <div className="flex flex-wrap items-center justify-between gap-2 px-0.5">
                  <span className="font-serif text-xs font-semibold text-muted-foreground">
                    图片 {index + 1}
                  </span>
                  <div className="flex flex-wrap items-center gap-2">
                    <Button
                      type="button"
                      size="sm"
                      variant="outline"
                      className="h-8 gap-1.5 border-border/80 bg-background/70 font-serif text-xs font-semibold text-foreground/80 hover:border-[#b83b38]/35 hover:bg-[#b83b38]/[0.06] hover:text-[#b83b38] dark:hover:border-[#d94f4b]/40 dark:hover:bg-[#d94f4b]/10 dark:hover:text-[#fca5a5]"
                      onClick={() => onUseAsReference(src)}
                      title="以这张图做图生图"
                    >
                      <ImageUp className="h-3.5 w-3.5" />
                      用作底本
                    </Button>
                    <Button
                      type="button"
                      size="sm"
                      variant="outline"
                      className="h-8 gap-1.5 border-[#b83b38]/30 bg-[#b83b38]/10 font-serif text-xs font-semibold text-[#b83b38] hover:bg-[#b83b38]/20 dark:border-[#d94f4b]/30 dark:bg-[#d94f4b]/15 dark:text-[#fca5a5]"
                      onClick={() => void onSaveAs(src, index)}
                    >
                      <Download className="h-3.5 w-3.5" />
                      {t("image.saveAs")}
                    </Button>
                  </div>
                </div>

                {job.result?.revisedPrompts[index] && job.result.revisedPrompts[index] !== job.prompt && (
                  <details className="rounded-lg bg-black/[0.025] px-2.5 py-2 text-xs text-muted-foreground ring-1 ring-border/40 dark:bg-white/[0.02]">
                    <summary className="cursor-pointer select-none font-serif font-semibold text-foreground/80">
                      模型修订后的提示词
                    </summary>
                    <p className="mt-1 font-serif text-[11px] leading-relaxed">
                      {job.result.revisedPrompts[index]}
                    </p>
                  </details>
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
  );
}
