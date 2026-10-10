import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Plus, Sparkles } from "lucide-react";
import type { MergeSuggestion } from "./aggregation-model";

const SHOWN = 6;

// Models several relays serve under one name, offered as ready-made aggregates.
export function AggregateSuggestions({ suggestions, disabled, onCreate }: {
  suggestions: MergeSuggestion[];
  disabled: boolean;
  onCreate: (suggestion: MergeSuggestion) => void;
}) {
  const { t } = useTranslation();
  const [all, setAll] = useState(false);
  if (suggestions.length === 0) return null;
  const shown = all ? suggestions : suggestions.slice(0, SHOWN);
  return (
    <div className="agg-suggest" data-merge-suggestions>
      <div className="flex min-w-0 items-center gap-2">
        <Sparkles aria-hidden className="h-4 w-4 shrink-0 text-primary" />
        <p className="min-w-0 text-[12.5px] text-muted-foreground">
          <span className="font-semibold text-foreground">{t("aggregation.suggest.title", { count: suggestions.length })}</span>
          {" "}{t("aggregation.suggest.hint")}
        </p>
      </div>
      <div className="flex flex-wrap gap-1.5">
        {shown.map((suggestion) => (
          <button
            key={suggestion.key}
            type="button"
            disabled={disabled}
            data-merge-suggestion={suggestion.key}
            onClick={() => onCreate(suggestion)}
            title={suggestion.members.map((member) => `${member.providerName} / ${member.modelId}`).join("\n")}
            aria-label={t("aggregation.suggest.create", { model: suggestion.publicModelId, count: suggestion.members.length })}
            className="agg-suggest-chip group"
          >
            <span className="truncate font-mono text-[12px] text-foreground">{suggestion.publicModelId}</span>
            <span className="shrink-0 text-[11px] text-muted-foreground">{t("aggregation.suggest.relays", { count: suggestion.members.length })}</span>
            <Plus aria-hidden className="h-3.5 w-3.5 shrink-0 text-primary transition-transform group-hover:rotate-90 motion-reduce:transition-none" />
          </button>
        ))}
        {suggestions.length > SHOWN && (
          <button type="button" onClick={() => setAll(!all)} className="agg-suggest-chip text-[12px] text-muted-foreground">
            {all ? t("aggregation.suggest.less") : t("aggregation.suggest.more", { count: suggestions.length - SHOWN })}
          </button>
        )}
      </div>
    </div>
  );
}
