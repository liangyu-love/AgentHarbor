import type { ElementType, ReactNode } from "react";
import { cn } from "@/lib/utils";

// The opening of a page, in the poster language shared by every page: an
// English kicker, a display title and the page's primary actions. Optional
// children render as a strip under a hairline.
export function PageHero({ kicker, icon, title, titleAs: Title = "h1", titleId, titleClassName, badge,
  description, actions, children, className }: {
  kicker: string;
  icon?: ReactNode;
  title: ReactNode;
  titleAs?: ElementType;
  titleId?: string;
  titleClassName?: string;
  badge?: ReactNode;
  description?: ReactNode;
  actions?: ReactNode;
  children?: ReactNode;
  className?: string;
}) {
  return (
    <section className={cn("route-surface route-rise relative overflow-hidden rounded-3xl border border-border/80 bg-card shadow-sm", className)}>
      <div className="relative flex flex-col gap-4 p-5 sm:flex-row sm:items-center sm:justify-between sm:p-6">
        <div className="flex min-w-0 items-center gap-3.5">
          {icon && (
            <span className="flex h-11 w-11 shrink-0 items-center justify-center rounded-2xl bg-foreground text-background [&_svg]:h-5 [&_svg]:w-5">
              {icon}
            </span>
          )}
          <div className="min-w-0">
            <p className="poster-kicker text-[10px] text-primary">{kicker}</p>
            <div className="flex flex-wrap items-center gap-2">
              <Title id={titleId} className={cn("poster-title text-2xl", titleClassName)}>{title}</Title>
              {badge}
            </div>
            {description && <p className="mt-0.5 text-[13px] text-muted-foreground">{description}</p>}
          </div>
        </div>
        {actions && <div className="flex flex-wrap items-center gap-2 sm:shrink-0">{actions}</div>}
      </div>
      {children && <div className="relative border-t border-border/60 px-5 py-4 sm:px-6">{children}</div>}
    </section>
  );
}
