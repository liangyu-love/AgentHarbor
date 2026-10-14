import { useTranslation } from "react-i18next";
import { Boxes, Layers3, UsersRound } from "lucide-react";
import { cn } from "@/lib/utils";
import type { ModelSources } from "@/types/gateway";

export const SOURCE_KINDS = ["relay", "account", "aggregate"] as const;
export type SourceKind = (typeof SOURCE_KINDS)[number];
const ICON = { relay: Boxes, account: UsersRound, aggregate: Layers3 } as const;

/**
 * The kinds of model a consumer is offered — relays, shared account sources
 * and aggregates — as three switches that combine freely.
 */
export function ModelSourceSwitches({ value, counts, disabled, label, onChange }: {
  value: ModelSources;
  counts?: Partial<Record<SourceKind, number>>;
  disabled?: boolean;
  label: string;
  onChange: (next: ModelSources) => void;
}) {
  const { t } = useTranslation();
  return (
    <div role="group" aria-label={label} className="flex flex-wrap gap-1.5" data-model-sources>
      {SOURCE_KINDS.map(kind => {
        const on = value[kind];
        const Icon = ICON[kind];
        return (
          <button key={kind} type="button" aria-pressed={on} data-model-source={kind} disabled={disabled}
            onClick={() => onChange({ ...value, [kind]: !on })}
            className={cn("inline-flex items-center gap-1.5 rounded-full border px-2.5 py-1 text-xs font-medium transition-colors",
              "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50",
              on ? "border-primary/40 bg-primary/10 text-foreground hover:border-primary/60"
                : "border-dashed border-border text-muted-foreground hover:border-primary/40")}>
            <Icon aria-hidden className={cn("h-3.5 w-3.5", on && "text-primary")} />
            <span className={cn(!on && "line-through decoration-muted-foreground/50")}>{t(`gateway.kind.${kind}`)}</span>
            {counts?.[kind] != null && <span className="font-mono text-[10px] tabular-nums text-muted-foreground">{counts[kind]}</span>}
          </button>
        );
      })}
    </div>
  );
}
