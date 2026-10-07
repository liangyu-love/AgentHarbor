import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { api } from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";
import { toast } from "@/hooks/use-toast";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import type { CodexSubagentSettings } from "@/types/newapi";
import type { ProviderState } from "./use-provider-state";
import { providerErrorKey } from "./relay-groups";

export function CodexSubagentSettingsFields({ store }: { store: ProviderState }) {
  const { t } = useTranslation();
  const query = useQuery({ queryKey: queryKeys.codexSubagents, queryFn: api.routing.subagents,
    refetchOnWindowFocus: false });
  const [draft, setDraft] = useState<CodexSubagentSettings | null>(null);
  const { mutationLocked, busy, run, applyChange } = store;
  const view = query.data;
  if (query.isPending) return <Skeleton className="h-20 w-full" aria-label={t("newapi.subagentLoading")} />;
  if (!view) return <div role="alert" className="flex flex-wrap items-center gap-2 text-sm text-destructive">
    <span>{t(providerErrorKey(query.error))}</span>
    <Button type="button" variant="outline" size="sm" onClick={() => void query.refetch()}>
      {t("newapi.subagentRetry")}
    </Button>
  </div>;
  const settings = draft ?? view.settings;
  const selected = view.models.find(model => model.id === settings.model);
  const efforts = settings.model ? selected?.reasoningEfforts ?? [] : view.inheritedReasoningEfforts;
  const modelUnavailable = settings.model != null && !selected;
  const effortUnavailable = settings.reasoningEffort != null && !efforts.includes(settings.reasoningEffort);
  const updateModel = (value: string) => {
    const model = value === "__inherit__" ? null : value;
    const supported = model
      ? view.models.find(candidate => candidate.id === model)?.reasoningEfforts ?? []
      : view.inheritedReasoningEfforts;
    setDraft({ model, reasoningEffort: settings.reasoningEffort && supported.includes(settings.reasoningEffort)
      ? settings.reasoningEffort : null });
  };
  const save = (next: CodexSubagentSettings | null) => void run("subagents", async () => {
    const change = await api.routing.saveSubagents(next);
    applyChange(change);
    const refreshed = await query.refetch();
    if (!refreshed.isError) setDraft(null);
    toast({ title: t(next ? "newapi.subagentSaved" : "newapi.subagentRestored"),
      description: change.applied ? t("newapi.subagentRestart") : undefined, variant: "success" });
  }, t("newapi.subagentSaveFailed"));
  return <section aria-labelledby="codex-subagents-title" className="space-y-3 border-t border-border/60 pt-4">
    <div className="flex flex-wrap items-baseline justify-between gap-2">
      <h3 id="codex-subagents-title" className="text-sm font-medium">{t("newapi.subagentDefaults")}</h3>
      {view.managed && <span className="text-xs text-muted-foreground">
        {t(view.applied ? "newapi.subagentApplied" : "newapi.subagentPending")}
      </span>}
    </div>
    <div className="grid min-w-0 gap-3 sm:grid-cols-[minmax(0,2fr)_minmax(0,1fr)]">
      <div className="min-w-0 space-y-1.5">
        <Label htmlFor="codex-subagent-model" className="text-xs text-muted-foreground">{t("newapi.subagentModel")}</Label>
        <Select value={settings.model ?? "__inherit__"} disabled={mutationLocked} onValueChange={updateModel}>
          <SelectTrigger id="codex-subagent-model" title={selected?.label ?? settings.model ?? t("newapi.subagentInheritModel")}
            className="min-w-0 gap-2 bg-background/80 [&>span]:min-w-0 [&>span]:flex-1 [&>span]:truncate [&>span]:text-left [&>svg]:shrink-0">
            <SelectValue />
          </SelectTrigger>
          <SelectContent className="max-w-[calc(100vw-2rem)]">
            <SelectItem value="__inherit__">{t("newapi.subagentInheritModel")}</SelectItem>
            {modelUnavailable && <SelectItem value={settings.model!} disabled>
              {t("newapi.subagentUnavailable", { model: settings.model })}
            </SelectItem>}
            {view.models.map(model => <SelectItem key={model.id} value={model.id} className="break-words">
              {model.label}
            </SelectItem>)}
          </SelectContent>
        </Select>
      </div>
      <div className="min-w-0 space-y-1.5">
        <Label htmlFor="codex-subagent-effort" className="text-xs text-muted-foreground">{t("newapi.subagentEffort")}</Label>
        <Select value={settings.reasoningEffort ?? "__inherit__"} disabled={mutationLocked}
          onValueChange={value => setDraft({ ...settings, reasoningEffort: value === "__inherit__" ? null : value })}>
          <SelectTrigger id="codex-subagent-effort" className="bg-background/80"><SelectValue /></SelectTrigger>
          <SelectContent>
            <SelectItem value="__inherit__">{t("newapi.subagentAutoEffort")}</SelectItem>
            {effortUnavailable && <SelectItem value={settings.reasoningEffort!} disabled>{settings.reasoningEffort}</SelectItem>}
            {efforts.map(effort => <SelectItem key={effort} value={effort}>{effort}</SelectItem>)}
          </SelectContent>
        </Select>
      </div>
    </div>
    {modelUnavailable && <p role="alert" className="text-xs text-destructive">{t("newapi.errors.subagentModel")}</p>}
    {!modelUnavailable && effortUnavailable && <p role="alert" className="text-xs text-destructive">
      {t("newapi.errors.subagentEffort")}
    </p>}
    {query.isError && <div role="alert" className="flex flex-wrap items-center gap-2 text-xs text-destructive">
      <span>{t(providerErrorKey(query.error))}</span>
      <Button type="button" variant="outline" size="sm" onClick={() => void query.refetch()}>
        {t("newapi.subagentRetry")}
      </Button>
    </div>}
    <div className="flex flex-wrap items-center justify-between gap-2">
      <span className="text-xs text-muted-foreground">{t("newapi.subagentNewOnly")}</span>
      <div className="flex flex-wrap gap-2">
        <Button type="button" variant="ghost" size="sm" disabled={mutationLocked || !view.managed}
          onClick={() => save(null)}>{t("newapi.subagentRestore")}</Button>
        <Button type="button" size="sm" disabled={mutationLocked || (!draft && view.managed) || modelUnavailable || effortUnavailable}
          onClick={() => save(settings)}>{t(busy === "subagents" ? "newapi.subagentSaving" : "newapi.save")}</Button>
      </div>
    </div>
  </section>;
}
