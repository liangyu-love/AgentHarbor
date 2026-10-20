import { useEffect, useRef, useState } from "react";
import { Upload, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { usePointerSort } from "./use-pointer-sort";
import { MAX_REFERENCES, type ReferenceImage } from "./workbench-types";

const moved = <T,>(items: T[], from: number, to: number) => {
  const next = [...items];
  const [item] = next.splice(from, 1);
  next.splice(to, 0, item);
  return next;
};

/** The image-to-image references, in the order the prompt names them: 图1, 图2… */
export function ReferenceStrip({ references, setReferences, upload }: {
  references: ReferenceImage[];
  setReferences: (references: ReferenceImage[]) => void;
  upload: (files: File[]) => Promise<void>;
}) {
  const inputRef = useRef<HTMLInputElement>(null);
  // A newly added reference glows briefly, so a load from the canvas, gallery or materials is seen.
  const [fresh, setFresh] = useState(false);
  const count = useRef(references.length);
  useEffect(() => {
    const grew = references.length > count.current;
    count.current = references.length;
    if (!grew) return;
    setFresh(true);
    const timer = window.setTimeout(() => setFresh(false), 1400);
    return () => window.clearTimeout(timer);
  }, [references.length]);
  const sort = usePointerSort({ onMove: (from, to) => setReferences(moved(references, from, to)) });
  const single = references.length === 1;

  return (
    <div data-image-reference data-fresh={fresh || undefined} className={cn(
      "rounded-xl border bg-background/50 p-3.5 transition-[border-color,background-color,box-shadow] duration-500 motion-reduce:transition-none",
      references.length > 0 ? "border-solid border-border" : "border-dashed border-border/80",
      fresh && "border-[#b83b38]/55 bg-[#b83b38]/[0.04] shadow-[0_0_0_3px_rgba(184,59,56,0.12)] dark:border-[#d94f4b]/55",
    )}>
      <div className="flex items-center justify-between gap-3">
        <div className="min-w-0">
          <p className="font-serif text-xs font-semibold text-foreground/90">
            展卷摹画 · 参考图 (可选)
            {references.length > 0 && (
              <span className="ml-1.5 font-sans text-[10px] font-normal tabular-nums text-muted-foreground">
                {references.length}/{MAX_REFERENCES}
              </span>
            )}
          </p>
          <p className="font-serif text-[11px] text-muted-foreground">
            {references.length > 1
              ? "按顺序称作图1、图2…，提示词里可以这样指代；拖动排序。"
              : `PNG / JPG / WebP，单张 ≤ 20 MB，最多 ${MAX_REFERENCES} 张；成图可点「用作底本」，或从画谱「图片」页选素材。`}
          </p>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          <input
            ref={inputRef}
            className="hidden"
            type="file"
            multiple
            accept="image/png,image/jpeg,image/webp"
            onChange={event => {
              void upload(Array.from(event.target.files ?? []));
              event.target.value = "";
            }}
          />
          <Button
            type="button"
            size="sm"
            variant="outline"
            className="h-8 gap-1.5 font-serif text-xs"
            disabled={references.length >= MAX_REFERENCES}
            onClick={() => inputRef.current?.click()}
          >
            <Upload className="h-3.5 w-3.5" />
            选入画稿
          </Button>
          {references.length > 1 && (
            <Button type="button" size="sm" variant="ghost" className="h-8 px-2 font-serif text-xs text-muted-foreground hover:text-destructive"
              onClick={() => setReferences([])}>
              清空
            </Button>
          )}
        </div>
      </div>
      {references.length > 0 && (
        <div className={cn("mt-3 flex flex-wrap gap-2", sort.drag && "select-none")}>
          {references.map((item, index) => {
            const label = `图${index + 1}`;
            const dragging = sort.drag?.from === index;
            const dropTarget = sort.drag?.over === index && !dragging;
            return (
              <div
                key={item.key}
                ref={sort.itemRef(index)}
                data-reference={label}
                className={cn(
                  "group/reference relative rounded-lg border bg-card/60 p-1 transition-[opacity,box-shadow]",
                  single ? "w-full" : "w-[5.5rem]",
                  dragging && "opacity-50",
                  dropTarget && "ring-2 ring-[#b83b38]/40",
                )}
              >
                <button
                  type="button"
                  aria-label={`${label}${item.name ? `：${item.name}` : ""}，拖动或 Alt + ←→ 排序`}
                  title={references.length > 1 ? "拖动或 Alt + ←→ 排序" : undefined}
                  className={cn(
                    "block w-full touch-none rounded outline-none focus-visible:ring-2 focus-visible:ring-[#b83b38]/35",
                    references.length > 1 && "cursor-grab active:cursor-grabbing",
                  )}
                  {...sort.handleProps(index, references.length)}
                >
                  <img
                    src={item.src}
                    alt={item.name ? `参考图 ${label}：${item.name}` : `参考图 ${label}`}
                    draggable={false}
                    className={cn("mx-auto rounded", single ? "max-h-48 object-contain" : "h-20 w-full object-cover")}
                  />
                </button>
                <span className="pointer-events-none absolute left-1.5 top-1.5 rounded bg-black/55 px-1 font-sans text-[10px] font-semibold text-white">
                  {label}
                </span>
                {item.name && (
                  <p className="mt-0.5 truncate px-0.5 text-center font-serif text-[10px] text-muted-foreground" title={item.note || item.name}>
                    {item.name}
                  </p>
                )}
                <button
                  type="button"
                  aria-label={`移除${label}`}
                  title={`移除${label}`}
                  onClick={() => setReferences(references.filter(other => other.key !== item.key))}
                  className="absolute right-1 top-1 rounded-full bg-background/90 p-0.5 text-muted-foreground shadow-sm ring-1 ring-border/60 hover:text-destructive"
                >
                  <X className="h-3.5 w-3.5" />
                </button>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
