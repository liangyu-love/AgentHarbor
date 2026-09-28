import { cn } from "@/lib/utils";
import type { NewApiProvider } from "@/types/newapi";

export function StatusDot({ provider }: { provider: NewApiProvider }) {
  return (
    <span
      className={cn(
        "h-2 w-2 shrink-0 rounded-full",
        provider.testStatus === "ok"
          ? "bg-emerald-500"
          : provider.testStatus === "failed"
            ? "bg-red-500"
            : "bg-zinc-400 dark:bg-zinc-600",
      )}
    />
  );
}
