import type { CSSProperties } from "react";
import { useTranslation } from "react-i18next";
import { Check, Copy, Pencil, Trash2 } from "lucide-react";
import type { AggregateLive, AggregateView } from "@/types/aggregation";
import type { NewApiProvider } from "@/types/newapi";
import { cn } from "@/lib/utils";
import { compactTokens } from "./model-resolution-fields";
import { relayGroupKey, relayHue } from "./relay-groups";
import { memberState, tiersOf, type MemberState } from "./aggregation-model";

const stateTone: Record<MemberState, string> = {
  hit: "border-primary/45 bg-primary/[0.07]",
  cooling: "border-amber-500/40 bg-amber-500/[0.07]",
  probe: "border-amber-500/30 bg-transparent",
  standby: "border-border/70 bg-card",
  off: "border-dashed border-border/80 bg-transparent opacity-60",
  down: "border-destructive/30 bg-destructive/[0.04]",
};
const dotTone: Record<MemberState, string> = {
  hit: "bg-primary agg-dot-live",
  cooling: "bg-amber-500",
  probe: "bg-amber-500/60",
  standby: "bg-muted-foreground/35",
  off: "bg-muted-foreground/25",
  down: "bg-destructive",
};

// One aggregate on the routing board: who it is, then its members tier by tier
// with what each is doing right now.
export function AggregateRouteRow({ model, providers, live, busy, copied, onCopy, onEdit, onDelete }: {
  model: AggregateView;
  providers: NewApiProvider[];
  live: AggregateLive | undefined;
  busy: boolean;
  copied: boolean;
  onCopy: () => void;
  onEdit: () => void;
  onDelete: () => void;
}) {
  const { t } = useTranslation();
  const tiers = tiersOf(model.members);
  const weighted = model.strategy === "weightedRoundRobin";
  const facts = [
    t(weighted ? "aggregation.weightedRoundRobin" : "aggregation.roundRobin"),
    t(model.affinity === "statelessRotation" ? "aggregation.statelessRotation" : "aggregation.session"),
    model.firstTokenTimeoutSecs ? t("aggregation.firstTokenShort", { seconds: model.firstTokenTimeoutSecs }) : null,
    model.projection ? t("aggregation.contextShort", { tokens: compactTokens(model.projection.contextWindow) }) : null,
  ].filter((fact): fact is string => !!fact);

  return (
    <li className="agg-row" data-aggregate-row={model.id} data-available={model.available}>
      <div className="min-w-0 space-y-2">
        <div className="flex min-w-0 items-center gap-2">
          <span aria-hidden className={cn("h-2 w-2 shrink-0 rounded-full", model.available ? "bg-emerald-500" : "bg-destructive")} />
          <h3 className="min-w-0 break-words font-display text-[17px] font-semibold leading-tight tracking-tight text-foreground">
            {model.displayName || model.publicModelId}
          </h3>
          <span className="sr-only">{t(model.available ? "aggregation.available" : "aggregation.unavailable")}</span>
        </div>
        <div className="flex min-w-0 items-center gap-1">
          <span className="min-w-0 break-all font-mono text-xs text-muted-foreground">{model.publicModelId}</span>
          <button
            type="button"
            onClick={onCopy}
            disabled={busy}
            title={t("aggregation.copyPublicId")}
            aria-label={t("aggregation.copyPublicId")}
            className="flex h-7 w-7 shrink-0 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-50"
          >
            {copied ? <Check className="h-3 w-3 text-emerald-500" /> : <Copy className="h-3 w-3" />}
          </button>
        </div>
        <p className="text-[11.5px] leading-relaxed text-muted-foreground">{facts.join(" · ")}</p>
        {model.reason && (
          <p className="text-xs text-destructive">
            {t(`aggregation.reasons.${model.reason}`, { defaultValue: t("aggregation.unavailable") })}
          </p>
        )}
      </div>

      <ol className="agg-lanes" aria-label={t("aggregation.members")}>
        {tiers.map((priority, index) => (
          <li key={priority} className="agg-lane" data-tier={index}>
            <span className="agg-lane-label">
              {index === 0 ? t("aggregation.tierPrimary") : tiers.length === 2 ? t("aggregation.tierBackupOnly") : t("aggregation.tierBackup", { index })}
            </span>
            <ul className="flex min-w-0 flex-wrap gap-1.5">
              {model.members.filter((member) => member.priority === priority).map((member) => {
                const { state, status, runtime } = memberState(model, member, live);
                const name = status?.providerName || member.providerId;
                const provider = providers.find((entry) => entry.id === member.providerId);
                // The same hue as the relay's card on the relays page.
                const hue = relayHue(provider ? relayGroupKey(provider.baseUrl) : name);
                return (
                  <li
                    key={member.id}
                    data-route-member={member.id}
                    data-member-state={state}
                    title={state === "down" ? t(`aggregation.reasons.${status?.reason}`, { defaultValue: t("aggregation.unavailable") }) : undefined}
                    className={cn("agg-chip", stateTone[state])}
                  >
                    <span aria-hidden className="agg-chip-mark" style={{ "--relay-hue": hue } as CSSProperties}>
                      {name.slice(0, 1).toUpperCase()}
                    </span>
                    <span className="min-w-0">
                      <span className="block truncate text-[12.5px] font-semibold leading-tight text-foreground">{name}</span>
                      <span className={cn("block truncate font-mono text-[11px] leading-tight text-muted-foreground", state === "down" && "line-through")}>
                        {member.actualModelId}
                      </span>
                    </span>
                    {weighted && <span className="agg-chip-weight">×{member.weight}</span>}
                    <span className="agg-chip-state">
                      <span aria-hidden className={cn("h-1.5 w-1.5 shrink-0 rounded-full", dotTone[state])} />
                      {state === "cooling"
                        ? t("aggregation.live.cooling", { seconds: Math.ceil((runtime?.coolingMs ?? 0) / 1000) })
                        : t(`aggregation.live.${state}`)}
                      {!!runtime?.inFlight && <span className="tabular-nums">· {runtime.inFlight}</span>}
                    </span>
                  </li>
                );
              })}
            </ul>
          </li>
        ))}
      </ol>

      <div className="agg-row-actions">
        <button type="button" className="agg-icon-button" disabled={busy} onClick={onEdit}
          aria-label={t("common.edit")} title={t("common.edit")}>
          <Pencil className="h-3.5 w-3.5" />
        </button>
        <button type="button" className="agg-icon-button hover:!text-destructive" disabled={busy} onClick={onDelete}
          aria-label={t("common.delete")} title={t("common.delete")}>
          <Trash2 className="h-3.5 w-3.5" />
        </button>
      </div>
    </li>
  );
}
