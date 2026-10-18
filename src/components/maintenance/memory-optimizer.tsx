import { useContext, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { AlertCircle, CheckCircle2, Info, Loader2, MemoryStick, ShieldCheck } from "lucide-react";
import { api } from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";
import { errorMessage, formatInvokeError } from "@/lib/invoke-error";
import { cn } from "@/lib/utils";
import { BentoCard } from "@/components/ui/bento-card";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { CountUp } from "@/components/ui/count-up";
import { PageActiveContext } from "@/components/layout/page-stage";
import type { MemoryArea, MemoryOptimizeOutcome, MemorySnapshot } from "@/types";

const PREFIX = "maintenance.memory.";

/** In run order. Purging the whole standby list or writing out the modified list can stall the system briefly, so those are opt-in. */
const AREAS: { id: MemoryArea; stalls?: true }[] = [
  { id: "workingSets" },
  { id: "systemFileCache" },
  { id: "modifiedFileCache" },
  { id: "modifiedList", stalls: true },
  { id: "standbyList", stalls: true },
  { id: "lowPriorityStandbyList" },
  { id: "registryCache" },
  { id: "combinePages" },
];
const DEFAULT_AREAS = AREAS.filter(area => !area.stalls).map(area => area.id);

/** The page lists, heaviest first, as Resource Monitor draws them. */
const SEGMENTS = [
  { key: "inUse", tone: "bg-primary" },
  { key: "modified", tone: "bg-primary/60" },
  { key: "standby", tone: "bg-primary/25" },
  { key: "free", tone: "bg-muted-foreground/15" },
] as const;

const MIB = 1024 ** 2;

function formatMemory(bytes: number): string {
  const gib = bytes / 1024 ** 3;
  return gib >= 1 ? `${gib.toFixed(1)} GB` : `${Math.round(bytes / MIB)} MB`;
}

interface Result {
  tone: "success" | "error" | "info";
  message: string;
}

/** Releases working sets and standby memory, the way PCL2 and Mem Reduct do. Windows only. */
export function MemoryOptimizer() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [result, setResult] = useState<Result | null>(null);
  // The lists move constantly; follow them only while the page is on screen.
  const pageActive = useContext(PageActiveContext);
  const status = useQuery({ queryKey: queryKeys.memoryStatus, queryFn: api.memory.status, refetchInterval: pageActive ? 3000 : false, retry: false });
  const areas = useQuery({ queryKey: queryKeys.memoryAreas, queryFn: api.memory.areas, retry: false });
  const chosen = areas.data ?? DEFAULT_AREAS;

  const saveAreas = useMutation({
    mutationFn: api.memory.setAreas,
    onMutate: next => queryClient.setQueryData(queryKeys.memoryAreas, next),
    onSuccess: saved => queryClient.setQueryData(queryKeys.memoryAreas, saved),
    onError: error => {
      void queryClient.invalidateQueries({ queryKey: queryKeys.memoryAreas });
      setResult({ tone: "error", message: formatInvokeError(error) });
    },
  });

  const describe = (outcome: MemoryOptimizeOutcome): Result => {
    const { before, after } = outcome;
    const released = Math.max(0, before.inUse - after.inUse);
    const freed = Math.max(0, after.free - before.free);
    const available = formatMemory(after.standby + after.free);
    let message = released < MIB && freed < MIB
      ? t(PREFIX + "alreadyLean", { available })
      : t(PREFIX + "optimized", { released: formatMemory(released), freed: formatMemory(freed), available });
    if (outcome.failed.length > 0) {
      message += " " + t(PREFIX + "partlyFailed", { areas: outcome.failed.map(area => t(`${PREFIX}areas.${area}.label`)).join(t(PREFIX + "listSeparator")) });
    }
    return { tone: outcome.failed.length === chosen.length ? "error" : "success", message };
  };

  const optimize = useMutation({
    mutationFn: () => api.memory.optimize(chosen),
    onMutate: () => setResult(null),
    onSuccess: outcome => {
      queryClient.setQueryData(queryKeys.memoryStatus, (current: typeof status.data) => current && { ...current, memory: outcome.after });
      setResult(describe(outcome));
    },
    onError: error => setResult(errorMessage(error) === PREFIX + "errors.cancelled"
      ? { tone: "info", message: t(PREFIX + "errors.cancelled") }
      : { tone: "error", message: formatInvokeError(error) }),
  });

  // A run the tray started: show its outcome here as well.
  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    api.memory.onOptimized(outcome => {
      queryClient.setQueryData(queryKeys.memoryStatus, (current: typeof status.data) => current && { ...current, memory: outcome.after });
      setResult(describe(outcome));
    }).then(stop => (disposed ? stop() : (unlisten = stop))).catch(() => undefined);
    return () => {
      disposed = true;
      unlisten?.();
    };
    // describe reads the latest translation and areas through the closure; re-subscribing on every change is not needed.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [queryClient]);

  if (!status.data?.supported) return null;
  const { elevated, memory } = status.data;
  const busy = optimize.isPending;

  const toggle = (area: MemoryArea, on: boolean) => {
    const next = on ? [...chosen, area] : chosen.filter(id => id !== area);
    saveAreas.mutate(AREAS.map(item => item.id).filter(id => next.includes(id)));
  };

  return (
    <BentoCard className="p-5 sm:col-span-2" data-memory-optimizer={busy ? "running" : "idle"}>
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="flex items-center gap-3">
          <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl bg-primary/10 text-primary ring-1 ring-inset ring-primary/15">
            <MemoryStick className="h-4 w-4" />
          </div>
          <span className="text-[15px] font-semibold tracking-tight">{t(PREFIX + "title")}</span>
        </div>
        <Button size="sm" onClick={() => optimize.mutate()} disabled={busy || chosen.length === 0} className="shrink-0">
          {busy ? <Loader2 className="mr-1.5 h-3.5 w-3.5 animate-spin" /> : !elevated && <ShieldCheck className="mr-1.5 h-3.5 w-3.5" />}
          {busy ? t(PREFIX + (elevated ? "running" : "waitingConsent")) : t(PREFIX + "action")}
        </Button>
      </div>
      <p className="mt-3 text-xs leading-relaxed text-muted-foreground">
        {t(PREFIX + "description")} {t(PREFIX + (elevated ? "elevated" : "needsAdmin"))}
      </p>

      {memory && <MemoryMeter memory={memory} />}

      <div className="mt-5">
        <div className="flex items-center justify-between gap-3">
          <p className="poster-kicker text-[10px] text-muted-foreground" aria-hidden>Release</p>
          <button type="button" className="text-xs text-muted-foreground underline-offset-4 hover:text-foreground hover:underline disabled:opacity-50"
            disabled={busy || DEFAULT_AREAS.join() === chosen.join()} onClick={() => saveAreas.mutate(DEFAULT_AREAS)}>
            {t(PREFIX + "restoreDefaults")}
          </button>
        </div>
        <div className="mt-2 grid gap-x-6 gap-y-1 sm:grid-cols-2" role="group" aria-label={t(PREFIX + "areasTitle")}>
          {AREAS.map(({ id, stalls }) => (
            <label key={id} className="flex cursor-pointer items-start gap-3 rounded-xl px-2 py-2 transition-colors hover:bg-muted/50" data-memory-area={id}>
              <Checkbox className="mt-0.5" checked={chosen.includes(id)} disabled={busy} onCheckedChange={on => toggle(id, on === true)} />
              <span className="min-w-0">
                <span className="flex flex-wrap items-center gap-x-2 text-[13px] font-medium">
                  {t(`${PREFIX}areas.${id}.label`)}
                  {stalls && <span className="text-[11px] font-normal text-amber-700 dark:text-amber-300">{t(PREFIX + "mayStall")}</span>}
                </span>
                <span className="mt-0.5 block text-xs leading-relaxed text-muted-foreground">{t(`${PREFIX}areas.${id}.hint`)}</span>
              </span>
            </label>
          ))}
        </div>
      </div>

      {result && (
        <div role="status" className={cn(
          "mt-4 flex items-start gap-2 rounded-xl border px-3 py-2 text-xs",
          result.tone === "success" && "border-emerald-500/20 bg-emerald-500/5 text-emerald-700 dark:text-emerald-400",
          result.tone === "error" && "border-destructive/20 bg-destructive/5 text-destructive",
          result.tone === "info" && "border-border bg-muted/40 text-muted-foreground",
        )}>
          {result.tone === "success" ? <CheckCircle2 className="mt-0.5 h-3.5 w-3.5 shrink-0" />
            : result.tone === "error" ? <AlertCircle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
            : <Info className="mt-0.5 h-3.5 w-3.5 shrink-0" />}
          <span className="leading-relaxed">{result.message}</span>
        </div>
      )}
    </BentoCard>
  );
}

function MemoryMeter({ memory }: { memory: MemorySnapshot }) {
  const { t } = useTranslation();
  const used = memory.inUse + memory.modified;
  const percent = memory.total > 0 ? (used / memory.total) * 100 : 0;
  return (
    <div className="harbor-well mt-4 p-4" data-memory-meter>
      <div className="flex flex-wrap items-end justify-between gap-x-4 gap-y-1">
        <div className="flex items-baseline gap-1">
          <CountUp value={percent} className="poster-number text-4xl font-semibold leading-none tracking-tight" />
          <span className="text-sm font-medium text-muted-foreground">%</span>
        </div>
        <p className="text-xs text-muted-foreground">
          {t(PREFIX + "usedOfTotal", { used: formatMemory(used), total: formatMemory(memory.total) })}
        </p>
      </div>
      <div className="mt-3 flex h-2.5 overflow-hidden rounded-full bg-muted-foreground/15" aria-hidden>
        {SEGMENTS.map(({ key, tone }) => (
          <div key={key} className={cn("h-full transition-[width] duration-700 ease-out motion-reduce:transition-none", tone)}
            style={{ width: `${memory.total > 0 ? (memory[key] / memory.total) * 100 : 0}%` }} />
        ))}
      </div>
      <dl className="mt-3 grid grid-cols-2 gap-x-4 gap-y-1.5 text-xs sm:grid-cols-4">
        {SEGMENTS.map(({ key, tone }) => (
          <div key={key} className="flex items-center gap-2">
            <span className={cn("h-2 w-2 shrink-0 rounded-full", tone)} aria-hidden />
            <dt className="text-muted-foreground">{t(`${PREFIX}lists.${key}`)}</dt>
            <dd className="ml-auto font-medium tabular-nums sm:ml-0">{formatMemory(memory[key])}</dd>
          </div>
        ))}
      </dl>
    </div>
  );
}
