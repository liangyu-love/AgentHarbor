import { cn } from "@/lib/utils";

export function BentoInnerPanel({
  className,
  children,
  ...props
}: React.HTMLAttributes<HTMLDivElement>) {
  return (
    <div
      className={cn(
        "rounded-2xl p-4",
        "bg-muted/50 dark:bg-muted/35",
        className,
      )}
      {...props}
    >
      {children}
    </div>
  );
}
