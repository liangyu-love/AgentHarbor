import { Check, Circle, Loader2, Terminal, X, AlertCircle, CheckCircle2 } from "lucide-react";
import { useTranslation } from "react-i18next";
import { cn } from "@/lib/utils";
import type { ProviderProbeEvent } from "@/types/newapi";

export function ProbeTimeline({ events, testing, failed }: { events: ProviderProbeEvent[]; testing: boolean; failed: boolean }) {
  const { t } = useTranslation();
  const steps = ["request", "headers", "firstByte", "complete"] as const;
  const terminal = [...events].reverse().find(event => event.stage === "complete" || event.stage === "failed");
  const status = events.find(event => event.stage === "headers")?.status;
  const request = events.find(event => event.stage === "request")?.text;
  const content = events.filter(event => event.stage === "content").map(event => event.text).join("").slice(0, 16000);
  const next = steps.findIndex(step => !events.some(event => event.stage === step));

  return (
    <div className="grid gap-4 sm:grid-cols-[210px_minmax(0,1fr)]" data-testid="probe-timeline">
      {/* Left Column: Vertical Stepper */}
      <div className="flex flex-col justify-between rounded-xl border border-border/60 bg-muted/20 p-3.5">
        <span className="mb-2 text-[11px] font-semibold uppercase tracking-wider text-muted-foreground/80">
          {t("newapi.probeStages")}
        </span>
        <ol className="space-y-1 py-1" aria-label={t("newapi.probeStages")}>
          {steps.map((step, index) => {
            const done = events.find(event => event.stage === step);
            const active = index === next;
            const broken = active && (failed || terminal?.stage === "failed");
            return (
              <li key={step} className="relative flex min-h-14 gap-3 pb-3 last:min-h-0 last:pb-0">
                {index < steps.length - 1 && (
                  <span
                    aria-hidden
                    className={cn(
                      "absolute left-[13px] top-7 h-7 w-0.5 rounded-full transition-colors duration-200 motion-reduce:transition-none",
                      done ? "bg-emerald-500/40" : "bg-border/60"
                    )}
                  />
                )}
                <span
                  className={cn(
                    "relative flex h-7 w-7 shrink-0 items-center justify-center rounded-full border-2 transition-colors duration-200 motion-reduce:transition-none shadow-sm",
                    done
                      ? "border-emerald-500/60 bg-emerald-500/10 text-emerald-600 dark:text-emerald-400"
                      : broken
                      ? "border-destructive/60 bg-destructive/10 text-destructive"
                      : active && testing
                      ? "border-primary bg-primary/15 text-primary ring-2 ring-primary/20"
                      : "border-border/70 bg-background text-muted-foreground/40"
                  )}
                >
                  {done ? (
                    <Check className="h-3.5 w-3.5 stroke-[2.5]" />
                  ) : broken ? (
                    <X className="h-3.5 w-3.5 stroke-[2.5]" />
                  ) : active && testing ? (
                    <Loader2 className="h-3.5 w-3.5 animate-spin motion-reduce:animate-none" />
                  ) : (
                    <Circle className="h-2 w-2 fill-current" />
                  )}
                </span>
                <div className="min-w-0 pt-0.5">
                  <p
                    className={cn(
                      "text-xs leading-snug font-medium transition-colors",
                      broken && "text-destructive font-semibold",
                      done && "text-foreground font-semibold",
                      active && testing && "text-primary font-semibold",
                      !done && !active && "text-muted-foreground"
                    )}
                  >
                    {t(`newapi.probe.${step}`)}
                  </p>
                  {done && (
                    <span className="mt-1 inline-flex items-center rounded border border-border/40 bg-muted/60 px-1.5 py-0.5 font-mono text-[10px] tabular-nums text-muted-foreground/90">
                      {(done.elapsedMs / 1000).toFixed(2)} s
                    </span>
                  )}
                </div>
              </li>
            );
          })}
        </ol>
      </div>

      {/* Right Column: Terminal Console */}
      <div className="flex min-w-0 flex-col overflow-hidden rounded-xl border border-zinc-800 bg-zinc-950 text-zinc-200 shadow-md">
        {/* Terminal Header */}
        <div className="flex items-center justify-between border-b border-zinc-800/80 bg-zinc-900/80 px-3.5 py-2 text-xs text-zinc-300">
          <div className="flex items-center gap-2">
            <div className="flex items-center gap-1.5" aria-hidden>
              <span className="h-2.5 w-2.5 rounded-full bg-red-500/70" />
              <span className="h-2.5 w-2.5 rounded-full bg-amber-500/70" />
              <span className="h-2.5 w-2.5 rounded-full bg-emerald-500/70" />
            </div>
            <Terminal className="ml-1.5 h-3.5 w-3.5 text-zinc-400" />
            <span className="font-mono text-xs font-semibold text-zinc-200">{t("newapi.probeResponse")}</span>
          </div>
          {status && (
            <span
              className={cn(
                "rounded-full border px-2 py-0.5 font-mono text-[11px] font-semibold",
                status >= 400
                  ? "border-red-500/30 bg-red-500/15 text-red-400"
                  : "border-emerald-500/30 bg-emerald-500/15 text-emerald-400"
              )}
            >
              HTTP {status}
            </span>
          )}
        </div>

        {/* Request Line */}
        <div className="border-b border-zinc-800/80 bg-zinc-900/50 px-3.5 py-2 font-mono text-[11px] leading-5 text-zinc-400">
          <pre className="whitespace-pre-wrap break-all font-mono text-[11px] leading-5 text-zinc-300">
            {request || t("newapi.probeWaiting")}
          </pre>
        </div>

        {/* Response Body Stream */}
        <pre
          role="log"
          aria-label={t("newapi.probeResponse")}
          className="h-36 flex-1 overflow-auto whitespace-pre-wrap break-words px-4 py-3 font-mono text-xs leading-6 text-zinc-200 selection:bg-primary/30"
        >
          {content || (testing ? t("newapi.probeReceiving") : "—")}
        </pre>

        {/* Status Footer */}
        <div
          aria-live="polite"
          className={cn(
            "flex items-center justify-between border-t border-zinc-800/80 bg-zinc-900/60 px-3.5 py-2 text-xs",
            failed || terminal?.stage === "failed" ? "text-red-400" : "text-zinc-400"
          )}
        >
          <span className="flex items-center gap-1.5 font-medium">
            {terminal?.stage === "failed" ? (
              <>
                <AlertCircle className="h-3.5 w-3.5 shrink-0" />
                <span>{t(terminal.text)}</span>
              </>
            ) : terminal?.stage === "complete" ? (
              <>
                <CheckCircle2 className="h-3.5 w-3.5 shrink-0 text-emerald-400" />
                <span className="text-emerald-400">{t("newapi.testSucceeded")}</span>
              </>
            ) : testing ? (
              <>
                <Loader2 className="h-3.5 w-3.5 shrink-0 animate-spin text-amber-400" />
                <span className="text-amber-400">{t("newapi.probeReceiving")}</span>
              </>
            ) : (
              <span>{t("newapi.probeWaiting")}</span>
            )}
          </span>
        </div>
      </div>
    </div>
  );
}
