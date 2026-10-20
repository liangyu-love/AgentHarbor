import { useContext, useState } from "react";
import { PageActiveContext } from "@/components/layout/page-stage";
import { toast } from "@/hooks/use-toast";
import { api, type GalleryItem } from "@/lib/api";
import { GalleryOverlays, GalleryPanel } from "./gallery-panel";
import { GenerationCanvas } from "./generation-canvas";
import { GenerationForm } from "./generation-form";
import { useGenerationQueue } from "./use-generation-queue";
import { useImageConnection } from "./use-image-connection";
import { useImageGallery } from "./use-image-gallery";
import { WorkbenchHeader } from "./workbench-header";
import { fileToDataUrl } from "./workbench-types";

export function ImageWorkbenchPage() {
  const pageActive = useContext(PageActiveContext);
  const [error, setError] = useState<string | null>(null);
  const connection = useImageConnection(pageActive, setError);
  const gallery = useImageGallery(pageActive);
  const queue = useGenerationQueue({
    persistToGallery: gallery.persistToGallery,
    onManualKeyUsed: connection.markKeyUsed,
  });
  const [prompt, setPrompt] = useState("");
  const [count, setCount] = useState(1);
  const [imageData, setImageData] = useState<string | null>(null);

  const upload = async (file: File | undefined) => {
    if (!file) return;
    try {
      setImageData(await fileToDataUrl(file));
      setError(null);
    } catch {
      setError("image.errors.input");
    }
  };

  const canGenerate =
    prompt.trim().length > 0 &&
    connection.model.trim().length > 0 &&
    connection.hasCredentials;

  /** Put a saved prompt back in the prompt box; from the page, also bring the box into view. */
  const fillPrompt = (text: string, reveal: boolean) => {
    setPrompt(text);
    if (!reveal) return;
    toast({ title: "已填入提示词", variant: "success" });
    const reduced = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    document.querySelector("[data-image-prompt]")?.scrollIntoView({ behavior: reduced ? "auto" : "smooth", block: "center" });
  };

  /** Make a finished image the next generation's reference, and bring the reference into view. */
  const takeAsReference = (src: string) => {
    setImageData(src);
    setError(null);
    toast({ title: "已载入为底本", description: "下次生成将以这张图做图生图", variant: "success" });
    const reduced = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    document.querySelector("[data-image-reference]")?.scrollIntoView({ behavior: reduced ? "auto" : "smooth", block: "center" });
  };

  const takeGalleryAsReference = async (item: GalleryItem) => {
    try {
      takeAsReference(await api.image.loadGalleryImage(item.path));
    } catch {
      toast({ title: "未能载入底本", description: item.name, variant: "destructive" });
    }
  };

  const generate = () => {
    if (!prompt.trim() || !connection.model.trim()) return;
    queue.enqueue({
      providerId: connection.providerId || null,
      baseUrl: connection.baseUrl,
      apiKey: connection.apiKey,
      model: connection.model.trim(),
      prompt: prompt.trim(),
      size: connection.size,
      quality: connection.quality,
      count,
      imageData,
    });
  };

  return (
    <div className="space-y-6 pb-12">
      {/* 卷首 */}
      <WorkbenchHeader gallery={gallery} />

      <div className="grid min-w-0 gap-4 sm:gap-6 xl:grid-cols-[minmax(0,1.15fr)_minmax(320px,0.85fr)]">
        {/* 创作参数 */}
        <GenerationForm
          connection={connection}
          prompt={prompt}
          setPrompt={setPrompt}
          count={count}
          setCount={setCount}
          imageData={imageData}
          setImageData={setImageData}
          upload={upload}
          error={error}
          canGenerate={canGenerate}
          generationJobs={queue.generationJobs}
          onGenerate={generate}
        />

        {/* 右侧：宣纸卷轴 · 成果预览 (成果画廊) */}
        <GenerationCanvas
          generationJobs={queue.generationJobs}
          onCancel={queue.cancelGeneration}
          onRemove={queue.removeCanvas}
          onRegenerate={queue.regenerate}
          onRetrySave={queue.saveJobToGallery}
          onPreview={gallery.showPreview}
          onSaveAs={gallery.saveImageAs}
          onUsePrompt={text => fillPrompt(text, true)}
          onUseAsReference={takeAsReference}
        />
      </div>

      {/* 历史画册 */}
      {gallery.galleryOpen && (
        <GalleryPanel gallery={gallery} onUsePrompt={text => fillPrompt(text, true)} onUseAsReference={item => void takeGalleryAsReference(item)} />
      )}

      <GalleryOverlays gallery={gallery} pageActive={pageActive} onUsePrompt={text => fillPrompt(text, false)}
        onUseAsReference={src => { gallery.showPreview(null); takeAsReference(src); }} />
    </div>
  );
}
