import { useRef, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { api, type GenerationMeta, type ImageGenerationRequest } from "@/lib/api";
import { errorKey } from "@/lib/invoke-error";
import { queryKeys } from "@/lib/query-keys";
import type { GenerationJob } from "./workbench-types";

/** Two generations run at once; the rest wait in order and can be dropped before they start. */
const MAX_ACTIVE_GENERATIONS = 2;

export function useGenerationQueue({ persistToGallery, onManualKeyUsed }: {
  persistToGallery: (images: string[], meta: GenerationMeta | undefined) => Promise<boolean>;
  onManualKeyUsed: (baseUrl: string, apiKey: string) => void;
}) {
  const queryClient = useQueryClient();
  const [generationJobs, setGenerationJobs] = useState<GenerationJob[]>([]);
  const queuedGenerations = useRef<Array<{ id: string; run: () => Promise<void> }>>([]);
  const activeGenerations = useRef(0);
  const gallerySaves = useRef(new Set<string>());
  // What each finished job saves beside its images, kept for a retried save.
  const jobRecords = useRef(new Map<string, GenerationMeta>());
  const pumpGenerations = useRef<() => void>(() => {});

  pumpGenerations.current = () => {
    while (activeGenerations.current < MAX_ACTIVE_GENERATIONS && queuedGenerations.current.length > 0) {
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
      const saved = await persistToGallery(images, jobRecords.current.get(id));
      setGenerationJobs(current => current.map(job => job.id === id
        ? { ...job, galleryStatus: saved ? "saved" : "failed" }
        : job));
      if (saved) void queryClient.invalidateQueries({ queryKey: queryKeys.imageGallery });
    } finally {
      gallerySaves.current.delete(id);
    }
  };

  const enqueue = (request: ImageGenerationRequest) => {
    const id = crypto.randomUUID();
    const job: GenerationJob = {
      id,
      prompt: request.prompt,
      model: request.model,
      size: request.size,
      quality: request.quality,
      status: activeGenerations.current < MAX_ACTIVE_GENERATIONS ? "running" : "queued",
    };
    setGenerationJobs(current => [job, ...current]);
    queuedGenerations.current.push({ id, run: async () => {
      setGenerationJobs(current => current.map(item => item.id === id ? { ...item, status: "running" } : item));
      try {
        const generated = await api.image.generate(request, id);
        jobRecords.current.set(id, {
          prompt: request.prompt,
          model: generated.model || request.model,
          size: request.size,
          quality: request.quality,
          revisedPrompts: generated.revisedPrompts,
        });
        setGenerationJobs(current => current.map(item => item.id === id
          ? { ...item, status: "completed", result: generated }
          : item));
        // A gallery failure stays visible on this canvas and can be retried.
        void saveJobToGallery(id, generated.images);
        if (!request.providerId && request.apiKey.trim()) onManualKeyUsed(request.baseUrl, request.apiKey);
      } catch (cause) {
        const key = errorKey(cause, ["image"], "image.errors.network");
        setGenerationJobs(current => current.map(item => item.id === id
          ? key === "image.errors.cancelled"
            ? { ...item, status: "cancelled" }
            : { ...item, status: "failed", error: key }
          : item));
      }
    } });
    pumpGenerations.current();
  };

  const cancelGeneration = (job: GenerationJob) => {
    if (job.status !== "running") return;
    // The job settles as "cancelled" through its own rejection; nothing to do if it already ended.
    void api.image.cancelGeneration(job.id).catch(() => undefined);
  };

  const removeCanvas = (job: GenerationJob) => {
    if (job.status === "running") return;
    if (job.status === "queued") {
      if (!queuedGenerations.current.some(queued => queued.id === job.id)) return;
      queuedGenerations.current = queuedGenerations.current.filter(queued => queued.id !== job.id);
    }
    jobRecords.current.delete(job.id);
    setGenerationJobs(current => current.filter(item => item.id !== job.id));
  };

  return { generationJobs, enqueue, cancelGeneration, removeCanvas, saveJobToGallery };
}
