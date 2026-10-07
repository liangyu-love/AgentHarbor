import type { ReactNode } from "react";
import { cn } from "@/lib/utils";

// Clicks on these keep their own behaviour instead of folding the section.
const OWN_CLICK = "button, a, input, select, textarea, label, [role='switch'], [role='combobox'], [role='checkbox']";

interface FoldHeaderProps {
  open: boolean;
  onToggle: () => void;
  titleId: string;
  contentId: string;
  title: ReactNode;
  leading?: ReactNode;
  meta?: ReactNode;
  actions?: ReactNode;
  className?: string;
  titleClassName?: string;
}

// A section header that folds its content. There is deliberately no chevron: the
// title is the real button (keyboard and screen readers), and the whole bar is a
// larger mouse target.
export function FoldHeader({
  open,
  onToggle,
  titleId,
  contentId,
  title,
  leading,
  meta,
  actions,
  className,
  titleClassName,
}: FoldHeaderProps) {
  return (
    <div
      className={cn("flex flex-wrap items-center justify-between gap-3", className)}
      onClick={(event) => {
        if (!(event.target as HTMLElement).closest(OWN_CLICK)) onToggle();
      }}
    >
      <div className="flex min-w-0 flex-1 items-center gap-2.5">
        {leading}
        <h2
          id={titleId}
          className={cn("min-w-0 text-base font-semibold tracking-tight text-foreground", titleClassName)}
        >
          <button
            type="button"
            aria-expanded={open}
            aria-controls={contentId}
            onClick={onToggle}
            className="rounded-md text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          >
            {title}
          </button>
        </h2>
        {meta}
      </div>
      {actions && (
        <div className="flex shrink-0 items-center gap-2" onClick={(event) => event.stopPropagation()}>
          {actions}
        </div>
      )}
    </div>
  );
}

export function FoldBody({
  open,
  id,
  className,
  children,
}: {
  open: boolean;
  id: string;
  className?: string;
  children: ReactNode;
}) {
  if (!open) return null;
  return (
    <div
      id={id}
      className={cn(
        "motion-safe:animate-in motion-safe:fade-in-0 motion-safe:slide-in-from-top-1 motion-safe:duration-200",
        className,
      )}
    >
      {children}
    </div>
  );
}
