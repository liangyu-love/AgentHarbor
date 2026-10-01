import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import {
  AlertTriangle,
  Brain,
  ChevronDown,
  Cpu,
  Layers,
  RefreshCw,
  Search,
  Sparkles,
  X,
} from "lucide-react";
import { api } from "@/lib/api";
import { formatInvokeError } from "@/lib/invoke-error";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Badge } from "@/components/ui/badge";
import { cn } from "@/lib/utils";
import type { CapabilityField } from "@/types/newapi";

function Origin({ field }: { field: CapabilityField<unknown> }) {
  const { t } = useTranslation();
  return (
    <div className="mt-1 flex items-center gap-1 font-mono text-[10px] text-muted-foreground/80">
      <span>{t("newapi.capabilitySource." + field.source)}</span>
      <span>·</span>
      <span>
        {field.updatedAt
          ? new Date(field.updatedAt * 1000).toLocaleTimeString()
          : t("newapi.capabilityDateUnknown")}
      </span>
    </div>
  );
}

export function ModelCapabilities() {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [search, setSearch] = useState("");
  const query = useQuery({
    queryKey: ["model-capabilities"],
    queryFn: api.routing.capabilities,
    enabled: open,
    refetchInterval: open ? 30_000 : false,
  });

  const allModels = query.data?.models ?? [];
  const models =
    allModels.filter((m) =>
      [m.providerName, m.displayName, m.upstreamModel, m.matchedModel].some((s) =>
        s.toLowerCase().includes(search.toLowerCase()),
      ),
    );

  return (
    <section
      className="rounded-2xl border border-border/70 bg-card p-4.5 backdrop-blur-md shadow-xs transition-all"
      aria-label={t("newapi.modelCapabilities")}
    >
      <div className="flex flex-wrap items-center justify-between gap-3">
        <Button
          variant="ghost"
          aria-expanded={open}
          onClick={() => setOpen(!open)}
          className="flex items-center gap-2 px-2 text-sm font-bold tracking-tight text-foreground hover:bg-muted/80 rounded-xl"
        >
          <div className="flex h-7 w-7 items-center justify-center rounded-lg bg-primary/10 text-primary">
            <Cpu className="h-4 w-4" />
          </div>
          <span>{t("newapi.modelCapabilities")}</span>
          <ChevronDown
            className={cn(
              "h-4 w-4 text-muted-foreground transition-transform duration-200",
              open && "rotate-180 text-foreground",
            )}
          />
        </Button>

        {open && query.data && (
          <span className="rounded-full bg-muted/80 px-2.5 py-0.5 font-mono text-[11px] font-semibold text-muted-foreground">
            {models.length} / {allModels.length} 模型
          </span>
        )}
      </div>

      {open && (
        <div className="mt-4 space-y-3.5 border-t border-border/50 pt-3">
          <p className="text-xs text-muted-foreground/85 leading-relaxed px-1">
            {t("newapi.capabilityRules")}
          </p>

          {/* Search & Actions Bar */}
          <div className="flex items-center gap-2">
            <div className="relative flex-1">
              <Search className="absolute left-3 top-1/2 -translate-y-1/2 h-3.5 w-3.5 text-muted-foreground" />
              <Input
                value={search}
                onChange={(e) => setSearch(e.target.value)}
                aria-label={t("newapi.capabilitySearch")}
                placeholder={t("newapi.capabilitySearch")}
                className="pl-8 pr-8 h-9 text-xs rounded-xl"
              />
              {search && (
                <button
                  type="button"
                  onClick={() => setSearch("")}
                  className="absolute right-2.5 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground"
                >
                  <X className="h-3.5 w-3.5" />
                </button>
              )}
            </div>

            <Button
              variant="outline"
              disabled={query.isFetching}
              onClick={() => void query.refetch()}
              className="h-9 gap-1.5 rounded-xl px-3 text-xs font-semibold shadow-xs"
            >
              <RefreshCw
                className={cn(
                  "h-3.5 w-3.5",
                  query.isFetching && "animate-spin text-primary",
                )}
              />
              <span>{t("common.refresh")}</span>
            </Button>
          </div>

          {query.error && (
            <div
              role="alert"
              className="rounded-xl border border-destructive/30 bg-destructive/5 p-3 text-xs font-medium text-destructive"
            >
              {formatInvokeError(query.error)}
            </div>
          )}

          {query.data?.stale && (
            <div
              role="status"
              className="flex items-start gap-2 rounded-xl border border-amber-500/30 bg-amber-500/10 p-3 text-xs font-medium text-amber-700 dark:text-amber-300"
            >
              <AlertTriangle className="h-4 w-4 shrink-0 text-amber-500 mt-0.5" />
              <span>{t("newapi.catalogStale")}</span>
            </div>
          )}

          {/* Models List */}
          <div className="max-h-[30rem] space-y-2.5 overflow-y-auto pr-0.5">
            {models.map((m) => (
              <details
                key={m.providerId + "::" + m.upstreamModel}
                className="group rounded-xl border border-border/70 bg-card p-3.5 shadow-2xs transition-all hover:border-primary/40 text-sm"
              >
                <summary className="flex cursor-pointer flex-wrap items-center justify-between gap-2 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring rounded-lg py-0.5">
                  <div className="flex flex-wrap items-center gap-1.5 font-medium">
                    <Badge variant="outline" className="text-[10px] font-semibold">
                      {m.providerName}
                    </Badge>
                    <span className="text-foreground">{m.displayName}</span>
                    <span className="text-muted-foreground/60">→</span>
                    <span className="font-mono font-bold text-primary">
                      {m.matchedModel}
                    </span>
                  </div>
                  <ChevronDown className="h-3.5 w-3.5 text-muted-foreground transition-transform duration-200 group-open:rotate-180" />
                </summary>

                {/* Expanded details */}
                <div className="mt-3.5 space-y-3 border-t border-border/40 pt-3">
                  <p className="break-words font-mono text-[11px] text-muted-foreground bg-muted/30 p-2 rounded-lg">
                    {t("newapi.upstreamModel")}:{" "}
                    <span className="font-semibold text-foreground">
                      {m.upstreamModel}
                    </span>
                  </p>

                  <div className="grid gap-3 sm:grid-cols-3">
                    {/* Reasoning levels */}
                    <div className="rounded-xl border border-border/50 bg-muted/15 p-2.5">
                      <h4 className="flex items-center gap-1.5 font-bold text-foreground text-xs">
                        <Brain className="h-3.5 w-3.5 text-primary" />
                        <span>{t("newapi.capabilityReasoning")}</span>
                      </h4>
                      <p className="mt-1 font-mono text-xs font-semibold text-foreground">
                        {m.reasoning.value.map((v) => v.effort).join(" / ") || "—"}
                      </p>
                      <Origin field={m.reasoning} />
                    </div>

                    {/* Context window */}
                    <div className="rounded-xl border border-border/50 bg-muted/15 p-2.5">
                      <h4 className="flex items-center gap-1.5 font-bold text-foreground text-xs">
                        <Layers className="h-3.5 w-3.5 text-primary" />
                        <span>{t("newapi.capabilityContext")}</span>
                      </h4>
                      <p className="mt-1 font-mono text-xs font-bold text-foreground">
                        {m.context.value?.toLocaleString() ?? "—"}{" "}
                        <span className="text-[10px] font-normal text-muted-foreground">
                          Token
                        </span>
                      </p>
                      <Origin field={m.context} />
                    </div>

                    {/* Output limit */}
                    <div className="rounded-xl border border-border/50 bg-muted/15 p-2.5">
                      <h4 className="flex items-center gap-1.5 font-bold text-foreground text-xs">
                        <Sparkles className="h-3.5 w-3.5 text-primary" />
                        <span>{t("newapi.capabilityOutput")}</span>
                      </h4>
                      <p className="mt-1 font-mono text-xs font-bold text-foreground">
                        {m.output.value?.toLocaleString() ?? "—"}{" "}
                        <span className="text-[10px] font-normal text-muted-foreground">
                          Token
                        </span>
                      </p>
                      <Origin field={m.output} />
                    </div>
                  </div>
                </div>
              </details>
            ))}
          </div>

          {!query.isLoading && !models.length && (
            <div className="rounded-xl border border-dashed border-border/70 p-8 text-center text-muted-foreground">
              <p className="text-xs font-medium">{t("newapi.capabilityEmpty")}</p>
            </div>
          )}
        </div>
      )}
    </section>
  );
}
