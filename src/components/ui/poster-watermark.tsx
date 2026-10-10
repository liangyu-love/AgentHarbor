import { cn } from "@/lib/utils";

/** A decorative relief word behind a hero; its echo and sheen layers repeat the text via `data-text`. */
export function PosterWatermark({ text, className }: { text: string; className?: string }) {
  return <span aria-hidden className={cn("poster-watermark", className)} data-text={text}>{text}</span>;
}
