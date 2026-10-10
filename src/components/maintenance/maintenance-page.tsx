import { useState, useCallback } from "react";
import { flushSync } from "react-dom";
import { useTranslation } from "react-i18next";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api";
import { cn } from "@/lib/utils";
import { BentoCard } from "@/components/ui/bento-card";
import { Button } from "@/components/ui/button";
import {
  AlertDialog,
  AlertDialogContent,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogCancel,
  AlertDialogAction,
} from "@/components/ui/alert-dialog";
import {
  Stethoscope,
  Trash2,
  RotateCw,
  RotateCcw,
  Loader2,
  CheckCircle2,
  AlertCircle,
  Wrench,
} from "lucide-react";
import { PageHero } from "@/components/layout/page-hero";
import { formatInvokeError } from "@/lib/invoke-error";

const MIN_FEEDBACK_MS = 800;

interface ActionResult {
  type: "success" | "error";
  message: string;
}

export function MaintenancePage() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [results, setResults] = useState<Record<string, ActionResult>>({});
  const [runningKeys, setRunningKeys] = useState<Record<string, boolean>>({});
  const [restartConfirmOpen, setRestartConfirmOpen] = useState(false);
  const [cleanConfirmOpen, setCleanConfirmOpen] = useState(false);

  const setActionResult = (key: string, result: ActionResult) => {
    setResults((prev) => ({ ...prev, [key]: result }));
  };

  const diagnoseMutation = useMutation({
    mutationFn: () => api.diagnose(),
    onSuccess: (res) => {
      const d = res.data;
      setActionResult("diagnose", {
        type: "success",
        message: t("maintenance.diagnoseResult", {
          os: d.platform.os,
          arch: d.platform.arch,
          version: d.coreVersion,
          count: d.registryState.accountCount,
        }),
      });
    },
    onError: (err) => setActionResult("diagnose", { type: "error", message: formatInvokeError(err) }),
  });

  const cleanMutation = useMutation({
    mutationFn: () => api.clean(),
    onSuccess: (res) => {
      queryClient.invalidateQueries();
      const d = res.data;
      setActionResult("clean", {
        type: "success",
        message: t("maintenance.cleanResult", {
          authBackups: d.authBackupsRemoved,
          registryBackups: d.registryBackupsRemoved,
          staleEntries: d.staleEntriesRemoved,
        }),
      });
    },
    onError: (err) => setActionResult("clean", { type: "error", message: formatInvokeError(err) }),
  });

  const rebuildMutation = useMutation({
    mutationFn: () => api.rebuildRegistry(),
    onSuccess: (res) => {
      queryClient.invalidateQueries();
      setActionResult("rebuild", {
        type: "success",
        message: t("maintenance.rebuildResult", { count: res.data.accountCount }),
      });
    },
    onError: (err) => setActionResult("rebuild", { type: "error", message: formatInvokeError(err) }),
  });

  const restartMutation = useMutation({
    mutationFn: () => api.targets.restart("codex"),
    onSuccess: () => setActionResult("restart", { type: "success", message: t("maintenance.codexRestarted") }),
    onError: (err) => setActionResult("restart", { type: "error", message: formatInvokeError(err) }),
  });

  const runAction = useCallback(async (key: string, mutateAsync: () => Promise<unknown>) => {
    if (runningKeys[key]) return;
    flushSync(() => setRunningKeys((prev) => ({ ...prev, [key]: true })));
    await new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));

    const startedAt = Date.now();
    try {
      await mutateAsync();
    } catch {
      // Each mutation displays its specific error via onError.
    } finally {
      const elapsed = Date.now() - startedAt;
      if (elapsed < MIN_FEEDBACK_MS) {
        await new Promise((r) => setTimeout(r, MIN_FEEDBACK_MS - elapsed));
      }
      setRunningKeys((prev) => ({ ...prev, [key]: false }));
    }
  }, [runningKeys]);

  const handleRestartClick = () => {
    setRestartConfirmOpen(true);
  };

  const handleRestartConfirm = () => {
    setRestartConfirmOpen(false);
    runAction("restart", () => restartMutation.mutateAsync());
  };

  const actions: {
    key: string;
    icon: typeof Stethoscope;
    iconTile: string;
    label: string;
    description: string;
    actionLabel: string;
    loadingLabel: string;
    onAction: () => void;
    variant?: "destructive";
  }[] = [
    {
      key: "diagnose",
      icon: Stethoscope,
      iconTile: "bg-primary/10 text-primary",
      label: t("maintenance.diagnose"),
      description: t("maintenance.diagnoseDesc"),
      actionLabel: t("maintenance.diagnoseAction"),
      loadingLabel: t("maintenance.diagnosing"),
      onAction: () => runAction("diagnose", () => diagnoseMutation.mutateAsync()),
    },
    {
      key: "clean",
      icon: Trash2,
      iconTile: "bg-amber-500/10 ring-amber-500/15 text-amber-500 dark:text-amber-400",
      label: t("maintenance.clean"),
      description: t("maintenance.cleanDesc"),
      actionLabel: t("maintenance.cleanAction"),
      loadingLabel: t("maintenance.cleaning"),
      onAction: () => setCleanConfirmOpen(true),
    },
    {
      key: "rebuild",
      icon: RotateCw,
      iconTile: "bg-primary/10 text-primary",
      label: t("maintenance.rebuild"),
      description: t("maintenance.rebuildDesc"),
      actionLabel: t("maintenance.rebuildAction"),
      loadingLabel: t("maintenance.rebuilding"),
      onAction: () => runAction("rebuild", () => rebuildMutation.mutateAsync()),
    },
    {
      key: "restart",
      icon: RotateCcw,
      iconTile: "bg-red-500/10 ring-red-500/15 text-red-500 dark:text-red-400",
      label: t("maintenance.restartCodex"),
      description: t("maintenance.restartCodexDesc"),
      actionLabel: t("maintenance.restartCodexAction"),
      loadingLabel: t("maintenance.running"),
      onAction: handleRestartClick,
      variant: "destructive",
    },
  ];

  return (
    <div className="space-y-6">
      <PageHero kicker="Maintenance" icon={<Wrench />} title={t("nav.maintenance")} description={t("maintenance.description")} />

      <div className="grid gap-4 sm:grid-cols-2">
        {actions.map(({ key, icon: Icon, iconTile, label, description, actionLabel, loadingLabel, onAction, variant }) => {
          const result = results[key];
          const busy = !!runningKeys[key];
          return (
            <BentoCard key={key} className="flex flex-col justify-between p-5">
              <div>
                <div className="flex items-center justify-between gap-3">
                  <div className="flex items-center gap-3">
                    <div className={cn("flex h-9 w-9 shrink-0 items-center justify-center rounded-xl ring-1 ring-inset", iconTile)}>
                      <Icon className="h-4 w-4" />
                    </div>
                    <span className="text-[15px] font-semibold tracking-tight">{label}</span>
                  </div>
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={onAction}
                    disabled={busy}
                    className={cn(
                      "shrink-0",
                      variant === "destructive" ? "text-muted-foreground hover:bg-destructive hover:text-white hover:border-destructive" : ""
                    )}
                  >
                    {busy && <Loader2 className="mr-1.5 h-3.5 w-3.5 animate-spin" />}
                    {busy ? loadingLabel : actionLabel}
                  </Button>
                </div>
                <p className="mt-3 text-xs leading-relaxed text-muted-foreground">{description}</p>
              </div>

              {result && (
                <div
                  className={cn(
                    "mt-4 flex items-start gap-2 rounded-xl border px-3 py-2 text-xs",
                    result.type === "success"
                      ? "border-emerald-500/20 bg-emerald-500/5 text-emerald-700 dark:text-emerald-400"
                      : "border-destructive/20 bg-destructive/5 text-destructive"
                  )}
                >
                  {result.type === "success" ? (
                    <CheckCircle2 className="mt-0.5 h-3.5 w-3.5 shrink-0" />
                  ) : (
                    <AlertCircle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
                  )}
                  <span className="leading-relaxed">{result.message}</span>
                </div>
              )}
            </BentoCard>
          );
        })}
      </div>

      <AlertDialog open={cleanConfirmOpen} onOpenChange={setCleanConfirmOpen}>
        <AlertDialogContent>
          <AlertDialogHeader><AlertDialogTitle>{t("baseline.cleanConfirmTitle")}</AlertDialogTitle><AlertDialogDescription>{t("baseline.cleanConfirmDesc")}</AlertDialogDescription></AlertDialogHeader>
          <AlertDialogFooter><AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel><AlertDialogAction onClick={() => { setCleanConfirmOpen(false); void runAction("clean", () => cleanMutation.mutateAsync()); }}>{t("maintenance.cleanAction")}</AlertDialogAction></AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
      <AlertDialog open={restartConfirmOpen} onOpenChange={setRestartConfirmOpen}>
        <AlertDialogContent className="max-w-sm">
          <AlertDialogHeader>
            <AlertDialogTitle>{t("maintenance.restartConfirmTitle")}</AlertDialogTitle>
            <AlertDialogDescription>{t("maintenance.restartConfirmDesc")}</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction
              onClick={handleRestartConfirm}
              className="bg-destructive text-white hover:bg-destructive/90"
            >
              {t("maintenance.restartCodexAction")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}
