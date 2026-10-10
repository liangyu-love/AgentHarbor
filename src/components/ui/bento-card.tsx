import { cn } from "@/lib/utils";

interface BentoCardProps extends React.HTMLAttributes<HTMLDivElement> {
  children: React.ReactNode;
  compact?: boolean;
}

export function BentoCard({
  className,
  children,
  compact,
  ...props
}: BentoCardProps) {
  return (
    <div
      className={cn(
        // A hairline instead of a border box, so cards inside sections stay quiet.
        "relative overflow-hidden rounded-[22px] bg-card shadow-[0_0_0_1px_hsl(var(--border)/0.85)]",
        "transition-shadow duration-200 ease-out hover:shadow-[0_0_0_1px_hsl(var(--border)),0_10px_26px_-22px_hsl(var(--foreground)/0.35)] motion-reduce:transition-none",
        compact ? "p-4" : "p-6",
        className,
      )}
      {...props}
    >
      <div className="relative z-10 flex h-full flex-col">{children}</div>
    </div>
  );
}
