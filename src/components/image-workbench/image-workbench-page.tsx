import { useContext, useState } from "react";
import { PageActiveContext } from "@/components/layout/page-stage";
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
          onRetrySave={queue.saveJobToGallery}
          onPreview={gallery.showPreview}
          onSaveAs={gallery.saveImageAs}
        />
      </div>

      {/* 历史画册 */}
      {gallery.galleryOpen && <GalleryPanel gallery={gallery} />}

      <GalleryOverlays gallery={gallery} pageActive={pageActive} />
    </div>
  );
}
