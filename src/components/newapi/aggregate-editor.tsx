import { useState, type DragEvent, type ReactNode } from "react";
import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { ArrowDown, ArrowUp, GripVertical, Plus, Save, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import type { AggregateMember, AggregateModel } from "@/types/aggregation";
import type { CompressionPolicy, ModelRef, NewApiProvider } from "@/types/newapi";
import { api } from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";
import { cn } from "@/lib/utils";
import { modelsOf, shiftedPriority, tiersOf } from "./aggregation-model";

const FIRST_TOKEN_CHOICES = [0, 15, 30, 45, 60, 90, 120, 180, 300];

// A choice between a few options, each explained in one line.
function Segmented<T extends string>({ id, label, value, options, onChange }: {
  id: string;
  label: string;
  value: T;
  options: { value: T; label: string; hint: string }[];
  onChange: (value: T) => void;
}) {
  return (
    <div className="min-w-0 space-y-1.5">
      <p id={`${id}-label`} className="text-xs font-medium text-muted-foreground">{label}</p>
      <div id={id} role="radiogroup" aria-labelledby={`${id}-label`} className="agg-segmented">
        {options.map((option) => (
          <button
            key={option.value}
            type="button"
            role="radio"
            aria-checked={value === option.value}
            onClick={() => onChange(option.value)}
            className="agg-segment"
          >
            <span className="block text-[13px] font-semibold">{option.label}</span>
            <span className="block text-[11px] leading-snug text-muted-foreground">{option.hint}</span>
          </button>
        ))}
      </div>
    </div>
  );
}

function Block({ title, hint, actions, children }: { title: string; hint?: string; actions?: ReactNode; children: ReactNode }) {
  return (
    <div className="space-y-3">
      <div className="flex flex-wrap items-end justify-between gap-2">
        <div className="min-w-0">
          <h4 className="poster-kicker text-[10px] text-muted-foreground">{title}</h4>
          {hint && <p className="mt-0.5 text-xs text-muted-foreground">{hint}</p>}
        </div>
        {actions}
      </div>
      {children}
    </div>
  );
}

// Creates or edits an aggregate. Members are arranged in tiers: the primary
// tier serves while any of it can, each backup tier only when all above fail.
export function AggregateEditor({ draft, update, providers, busy, suggestions, references, onSave, onCancel }: {
  draft: AggregateModel;
  update: (patch: Partial<AggregateModel>) => void;
  providers: NewApiProvider[];
  busy: boolean;
  suggestions: string[];
  references: ModelRef[];
  onSave: () => void;
  onCancel: () => void;
}) {
  const { t } = useTranslation();
  // The gateway's shared aggregate prefix, which an empty field keeps.
  const gateway = useQuery({ queryKey: queryKeys.gateway, queryFn: api.gateway.state, refetchOnWindowFocus: false });
  const sharedPrefix = gateway.data?.aggregatePrefix ?? "pool";
  const [dragged, setDragged] = useState<string | null>(null);
  const [dropTier, setDropTier] = useState<number | "new" | null>(null);
  const tiers = tiersOf(draft.members);
  const weighted = draft.strategy === "weightedRoundRobin";

  const memberUpdate = (id: string, patch: Partial<AggregateMember>) =>
    update({ members: draft.members.map((member) => (member.id === id ? { ...member, ...patch } : member)) });

  const addMember = () => {
    const provider = providers.find((entry) => modelsOf(entry).length > 0);
    update({
      members: [...draft.members, {
        id: crypto.randomUUID(),
        providerId: provider?.id ?? "",
        actualModelId: modelsOf(provider)[0] ?? "",
        enabled: true,
        priority: tiers[0] ?? 0,
        weight: 1,
      }],
    });
  };

  const dropOn = (target: number | "new") => (event: DragEvent) => {
    event.preventDefault();
    setDropTier(null);
    const id = event.dataTransfer.getData("text/plain") || dragged;
    setDragged(null);
    if (!id) return;
    memberUpdate(id, { priority: target === "new" ? (tiers[tiers.length - 1] ?? 0) + 1 : target });
  };
  const dragOver = (target: number | "new") => (event: DragEvent) => {
    if (!dragged) return;
    event.preventDefault();
    event.dataTransfer.dropEffect = "move";
    setDropTier(target);
  };

  const memberRow = (member: AggregateMember) => {
    const index = draft.members.indexOf(member);
    const provider = providers.find((entry) => entry.id === member.providerId);
    const providerModels = modelsOf(provider);
    const up = shiftedPriority(draft.members, member, -1);
    const down = shiftedPriority(draft.members, member, 1);
    return (
      <div
        key={member.id}
        data-aggregate-member
        draggable={!busy}
        onDragStart={(event) => {
          event.dataTransfer.setData("text/plain", member.id);
          event.dataTransfer.effectAllowed = "move";
          setDragged(member.id);
        }}
        onDragEnd={() => { setDragged(null); setDropTier(null); }}
        className={cn("agg-member", dragged === member.id && "opacity-50", !member.enabled && "agg-member-off")}
      >
        <GripVertical aria-hidden className="agg-member-grip h-4 w-4 shrink-0 text-muted-foreground/60" />
        <div className="min-w-0 space-y-1">
          <Label htmlFor={`member-provider-${member.id}`} className="sr-only">{t("aggregation.provider")}</Label>
          <Select
            value={member.providerId}
            onValueChange={(value) => {
              const next = providers.find((entry) => entry.id === value);
              memberUpdate(member.id, { providerId: value, actualModelId: modelsOf(next)[0] ?? "" });
            }}
          >
            <SelectTrigger id={`member-provider-${member.id}`} className="h-9 bg-card text-xs">
              <SelectValue placeholder={t("aggregation.selectProvider")} />
            </SelectTrigger>
            <SelectContent>
              {!provider && member.providerId && (
                <SelectItem value={member.providerId} disabled>
                  {member.providerId} · {t("aggregation.unavailable")}
                </SelectItem>
              )}
              {providers.map((entry) => (
                <SelectItem key={entry.id} value={entry.id}>{entry.name}</SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
        <div className="min-w-0 space-y-1">
          <Label htmlFor={`member-model-${member.id}`} className="sr-only">{t("aggregation.actualId")}</Label>
          <Select value={member.actualModelId} onValueChange={(value) => memberUpdate(member.id, { actualModelId: value })}>
            <SelectTrigger id={`member-model-${member.id}`} className="h-9 bg-card font-mono text-xs">
              <SelectValue placeholder={t("aggregation.selectModel")} />
            </SelectTrigger>
            <SelectContent>
              {member.actualModelId && !providerModels.includes(member.actualModelId) && (
                <SelectItem value={member.actualModelId} disabled>
                  {member.actualModelId} · {t("aggregation.unavailable")}
                </SelectItem>
              )}
              {providerModels.map((id) => (
                <SelectItem key={id} value={id}>{id}</SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
        {weighted && (
          <label className="agg-weight" title={t("aggregation.weightHint")}>
            <span className="text-[11px] text-muted-foreground">{t("aggregation.weight")}</span>
            <Input
              id={`member-weight-${member.id}`}
              type="number"
              min={1}
              max={1000}
              value={member.weight}
              className="h-9 w-16 bg-card px-2 font-mono text-xs"
              onChange={(event) => memberUpdate(member.id, { weight: Number(event.target.value) })}
            />
          </label>
        )}
        <div className="agg-member-tools">
          <Switch
            checked={member.enabled}
            aria-label={t("aggregation.memberEnabledFor", { index: index + 1 })}
            title={t("aggregation.memberEnabled")}
            onCheckedChange={(enabled) => memberUpdate(member.id, { enabled })}
          />
          <button type="button" className="agg-icon-button" disabled={up === null}
            aria-label={t("aggregation.moveUp", { index: index + 1 })} title={t("aggregation.moveUpHint")}
            onClick={() => up !== null && memberUpdate(member.id, { priority: up })}>
            <ArrowUp className="h-3.5 w-3.5" />
          </button>
          <button type="button" className="agg-icon-button" disabled={down === null}
            aria-label={t("aggregation.moveDown", { index: index + 1 })} title={t("aggregation.moveDownHint")}
            onClick={() => down !== null && memberUpdate(member.id, { priority: down })}>
            <ArrowDown className="h-3.5 w-3.5" />
          </button>
          <button type="button" className="agg-icon-button hover:!text-destructive"
            aria-label={t("aggregation.removeMember", { index: index + 1 })}
            onClick={() => update({ members: draft.members.filter((entry) => entry.id !== member.id) })}>
            <Trash2 className="h-3.5 w-3.5" />
          </button>
        </div>
      </div>
    );
  };

  return (
    <form
      className="agg-editor"
      data-aggregate-editor
      onSubmit={(event) => {
        event.preventDefault();
        onSave();
      }}
    >
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="min-w-0">
          <p className="poster-kicker text-[10px] text-primary">{draft.id ? "Edit pool" : "New pool"}</p>
          <h3 className="font-display text-lg font-semibold tracking-tight">
            {draft.id ? t("aggregation.editTitle") : t("aggregation.add")}
          </h3>
        </div>
        <label className="flex items-center gap-2.5 text-xs font-medium">
          {t("aggregation.enabled")}
          <Switch
            checked={draft.enabled}
            disabled={busy}
            aria-label={t("aggregation.enabled")}
            onCheckedChange={(enabled) => update({ enabled })}
          />
        </label>
      </div>

      <fieldset disabled={busy} className="min-w-0 space-y-6">
        <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-[minmax(0,1.3fr)_minmax(0,1.3fr)_minmax(0,1fr)]">
          <div className="space-y-1.5">
            <Label htmlFor="aggregate-public-id" className="text-xs font-medium">{t("aggregation.publicId")}</Label>
            <Input
              id="aggregate-public-id"
              list="aggregate-official-models"
              value={draft.publicModelId}
              disabled={!!draft.id}
              placeholder={t("aggregation.publicIdExample")}
              className="bg-card font-mono text-sm"
              onChange={(event) => {
                const matching = references.filter((reference) => reference.modelId === event.target.value);
                const providerSet = new Set(matching.map((reference) => reference.providerId));
                update({ publicModelId: event.target.value, canonicalRef: providerSet.size === 1 ? matching[0] : null });
              }}
              maxLength={256}
              required
            />
            <datalist id="aggregate-official-models">
              {suggestions.map((id) => <option key={id} value={id} />)}
            </datalist>
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="aggregate-display-name" className="text-xs font-medium">{t("aggregation.displayName")}</Label>
            <Input
              id="aggregate-display-name"
              value={draft.displayName}
              placeholder={t("aggregation.displayNameExample")}
              className="bg-card text-sm"
              onChange={(event) => update({ displayName: event.target.value })}
              maxLength={200}
            />
          </div>
          <div className="space-y-1.5 sm:col-span-2 xl:col-span-1">
            <Label htmlFor="aggregate-gateway-prefix" className="text-xs font-medium">{t("aggregation.gatewayPrefix")}</Label>
            <Input
              id="aggregate-gateway-prefix"
              value={draft.gatewayPrefix ?? ""}
              placeholder={sharedPrefix}
              className="bg-card font-mono text-sm"
              aria-describedby="aggregate-gateway-prefix-hint"
              onChange={(event) => update({ gatewayPrefix: event.target.value.toLowerCase() || null })}
              maxLength={40}
              spellCheck={false}
            />
            <p id="aggregate-gateway-prefix-hint" className="truncate text-[11px] text-muted-foreground" data-gateway-id-preview>
              {t("aggregation.gatewayPrefixHint")}{" "}
              <code className="font-mono text-foreground/80">{`${draft.gatewayPrefix?.trim() || sharedPrefix}/${draft.publicModelId || "…"}`}</code>
            </p>
          </div>
        </div>

        <Block
          title={t("aggregation.members")}
          hint={t("aggregation.tiersHint")}
          actions={
            <Button type="button" variant="outline" size="sm" className="h-8 gap-1 rounded-lg text-xs"
              disabled={draft.members.length >= 64 || providers.length === 0} onClick={addMember}>
              <Plus className="h-3.5 w-3.5" />
              {t("aggregation.addMember")}
            </Button>
          }
        >
          <div className="space-y-2">
            {tiers.map((priority, index) => (
              <div
                key={priority}
                data-tier-lane={index}
                onDragOver={dragOver(priority)}
                onDragLeave={() => setDropTier(null)}
                onDrop={dropOn(priority)}
                className={cn("agg-tier", dropTier === priority && "agg-tier-drop")}
              >
                <div className="agg-tier-head">
                  <span className="agg-tier-index">{index + 1}</span>
                  <span className="text-[13px] font-semibold">
                    {index === 0 ? t("aggregation.tierPrimary") : tiers.length === 2 ? t("aggregation.tierBackupOnly") : t("aggregation.tierBackup", { index })}
                  </span>
                  <span className="text-[11px] text-muted-foreground">
                    {t(index === 0 ? "aggregation.tierPrimaryHint" : "aggregation.tierBackupHint")}
                  </span>
                </div>
                <div className="space-y-1.5">
                  {draft.members.filter((member) => member.priority === priority).map(memberRow)}
                </div>
              </div>
            ))}
            {dragged && (
              <div
                onDragOver={dragOver("new")}
                onDragLeave={() => setDropTier(null)}
                onDrop={dropOn("new")}
                className={cn("agg-tier-new", dropTier === "new" && "agg-tier-drop")}
              >
                {t("aggregation.dropNewTier")}
              </div>
            )}
            {draft.members.length === 0 && (
              <div className="rounded-xl border border-dashed border-border/80 p-6 text-center text-xs text-muted-foreground">
                {t(providers.length ? "aggregation.addMembersHint" : "aggregation.noProviders")}
              </div>
            )}
          </div>
        </Block>

        <div className="grid gap-6 xl:grid-cols-2">
          <Block title={t("aggregation.scheduling")}>
            <div className="grid gap-3 sm:grid-cols-2">
              <Segmented
                id="aggregate-strategy"
                label={t("aggregation.strategy")}
                value={draft.strategy}
                onChange={(strategy) => update({ strategy })}
                options={[
                  { value: "roundRobin", label: t("aggregation.roundRobin"), hint: t("aggregation.roundRobinHint") },
                  { value: "weightedRoundRobin", label: t("aggregation.weightedRoundRobin"), hint: t("aggregation.weightedRoundRobinHint") },
                ]}
              />
              <Segmented
                id="aggregate-affinity"
                label={t("aggregation.affinity")}
                value={draft.affinity}
                onChange={(affinity) => update({ affinity })}
                options={[
                  { value: "session", label: t("aggregation.session"), hint: t("aggregation.sessionHint") },
                  { value: "statelessRotation", label: t("aggregation.statelessRotation"), hint: t("aggregation.statelessRotationHint") },
                ]}
              />
            </div>
          </Block>

          <Block title={t("aggregation.failover")}>
            <div className="grid gap-3 sm:grid-cols-3">
              <div className="min-w-0 space-y-1.5">
                <Label htmlFor="aggregate-failure-threshold" className="text-xs font-medium text-muted-foreground">
                  {t("aggregation.failureThreshold")}
                </Label>
                <Select
                  value={String(draft.retryPolicy.failureThreshold ?? 3)}
                  onValueChange={(value) => update({ retryPolicy: { ...draft.retryPolicy, failureThreshold: Number(value) } })}
                >
                  <SelectTrigger id="aggregate-failure-threshold" className="bg-card"><SelectValue /></SelectTrigger>
                  <SelectContent>
                    {[2, 3, 4, 5, 6, 7, 8, 9, 10].map((value) => (
                      <SelectItem key={value} value={String(value)}>{value}</SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
              <div className="min-w-0 space-y-1.5">
                <Label htmlFor="aggregate-attempts" className="text-xs font-medium text-muted-foreground">
                  {t("aggregation.retryAttempts")}
                </Label>
                <Select
                  value={String(draft.retryPolicy.maxAttempts)}
                  onValueChange={(value) => update({ retryPolicy: { ...draft.retryPolicy, maxAttempts: Number(value) } })}
                >
                  <SelectTrigger id="aggregate-attempts" className="bg-card"><SelectValue /></SelectTrigger>
                  <SelectContent>
                    <SelectItem value="1">1</SelectItem>
                    <SelectItem value="2">2</SelectItem>
                  </SelectContent>
                </Select>
              </div>
              <div className="min-w-0 space-y-1.5">
                <Label htmlFor="aggregate-first-token" className="text-xs font-medium text-muted-foreground">
                  {t("aggregation.firstToken")}
                </Label>
                <Select
                  value={String(draft.firstTokenTimeoutSecs ?? 0)}
                  onValueChange={(value) => update({ firstTokenTimeoutSecs: Number(value) })}
                >
                  <SelectTrigger id="aggregate-first-token" className="bg-card"><SelectValue /></SelectTrigger>
                  <SelectContent>
                    {FIRST_TOKEN_CHOICES.map((seconds) => (
                      <SelectItem key={seconds} value={String(seconds)}>
                        {seconds ? t("aggregation.seconds", { seconds }) : t("aggregation.firstTokenOff")}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
            </div>
            <p className={cn("text-[11.5px] leading-relaxed",
              draft.firstTokenTimeoutSecs && draft.retryPolicy.maxAttempts < 2 ? "text-amber-700 dark:text-amber-300" : "text-muted-foreground")}>
              {draft.firstTokenTimeoutSecs && draft.retryPolicy.maxAttempts < 2
                ? t("aggregation.firstTokenNeedsRetry")
                : t("aggregation.firstTokenHint")}
            </p>
          </Block>
        </div>

        <Block title={t("aggregation.compression")}>
          <div className="flex min-w-0 max-w-md gap-2">
            <Select
              value={draft.compression.mode}
              onValueChange={(value) => {
                const mode = value as CompressionPolicy["mode"];
                update({
                  compression: mode === "percent" ? { mode, percent: 90 } : mode === "tokens" ? { mode, tokens: 100_000 } : { mode },
                });
              }}
            >
              <SelectTrigger id="aggregate-compression" className="min-w-0 flex-1 bg-card"><SelectValue /></SelectTrigger>
              <SelectContent>
                {(["inherit", "percent", "tokens", "clientDefault"] as const).map((mode) => (
                  <SelectItem key={mode} value={mode}>{t("newapi.compression." + mode)}</SelectItem>
                ))}
              </SelectContent>
            </Select>
            {(draft.compression.mode === "percent" || draft.compression.mode === "tokens") && (
              <Input
                type="number"
                min={1}
                max={draft.compression.mode === "percent" ? 99 : 10_000_000}
                className="w-28 shrink-0 bg-card font-mono text-xs"
                aria-label={t("newapi.compression.modelLevel")}
                value={draft.compression.mode === "percent" ? draft.compression.percent : draft.compression.tokens}
                onChange={(event) =>
                  update({
                    compression: draft.compression.mode === "percent"
                      ? { mode: "percent", percent: Number(event.target.value) }
                      : { mode: "tokens", tokens: Number(event.target.value) },
                  })
                }
              />
            )}
          </div>
        </Block>
      </fieldset>

      <div className="flex items-center justify-end gap-2.5 border-t border-border/60 pt-4">
        <Button type="button" variant="outline" disabled={busy} onClick={onCancel}>{t("common.cancel")}</Button>
        <Button type="submit" disabled={busy} className="gap-1.5 shadow-sm">
          <Save className="h-4 w-4" />
          {t("common.save")}
        </Button>
      </div>
    </form>
  );
}
