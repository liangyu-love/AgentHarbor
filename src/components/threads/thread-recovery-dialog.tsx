import { useEffect, useState } from "react";
import {
  ArrowRight,
  Database,
  FileJson,
  FolderOpen,
  LoaderCircle,
  RefreshCcw,
  X,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import { api } from "@/lib/api";
import { toast } from "@/hooks/use-toast";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import type { ThreadRecoveryPreview, ThreadSummary } from "@/types/threads";
import { formatInvokeError } from "@/lib/invoke-error";

interface ThreadRecoveryDialogProps {
  thread: ThreadSummary | null;
  onOpenChange: (open: boolean) => void;
  onRecovered: () => void | Promise<void>;
}

export function ThreadRecoveryDialog({
  thread,
  onOpenChange,
  onRecovered,
}: ThreadRecoveryDialogProps) {
  const { t } = useTranslation();
  const [preview, setPreview] = useState<ThreadRecoveryPreview | null>(null);
  const [previewLoading, setPreviewLoading] = useState(false);
  const [applying, setApplying] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [previewVersion, setPreviewVersion] = useState(0);
  const canConfirm = !previewLoading && preview?.canApply === true;
  const adoptingRouter = preview?.targetProvider === "agentharbor_relay"
    && (preview.status === "providerMismatch" || preview.status === "missingCatalog");

  useEffect(() => {
    setError(null);
  }, [thread]);

  useEffect(() => {
    let cancelled = false;
    if (!thread) {
      setPreview(null);
      return;
    }

    setPreview(null);
    setPreviewLoading(true);
    void api.conversations.previewRecovery(thread.id)
      .then((result) => {
        if (!cancelled) setPreview(result.data);
      })
      .catch((cause) => {
        if (!cancelled) setError(formatInvokeError(cause));
      })
      .finally(() => {
        if (!cancelled) setPreviewLoading(false);
      });

    return () => {
      cancelled = true;
    };
  }, [thread, previewVersion]);

  const close = () => {
    if (!applying) onOpenChange(false);
  };

  const apply = async (event: React.MouseEvent) => {
    event.preventDefault();
    if (!thread || !preview || !canConfirm || applying) return;
    const isCatalogInsert = preview.status === "missingCatalog";
    setApplying(true);
    setError(null);
    try {
      const result = await (adoptingRouter
        ? api.conversations.adoptRouter
        : api.conversations.applyRecovery)(thread.id, preview.revision);
      toast({
        title: t(
          adoptingRouter
            ? "threads.routerAdoptionSuccess"
            : isCatalogInsert ? "threads.recoveryCatalogSuccess" : "threads.recoverySuccess",
        ),
        description: result.data.changed
          ? adoptingRouter
            ? t("threads.routerAdoptionSuccessDesc")
            : isCatalogInsert
              ? t("threads.recoveryCatalogSuccessDesc")
              : t("threads.recoverySuccessDesc", {
                  provider: result.data.preview.targetProvider,
                })
          : t("threads.recoveryNoChanges"),
        variant: "success",
      });
      if (result.warnings.some((warning) => warning.code === "CODEX_REOPEN_FAILED")) {
        toast({ title: t("threads.routerAdoptionOpenFailed"),
          description: t("threads.routerAdoptionOpenFailedDesc"), variant: "destructive" });
      }
      onOpenChange(false);
      // A refresh failure must not turn an already committed repair into a
      // reported failure or invite another adoption/restart.
      try {
        await onRecovered();
      } catch {
        toast({ title: t("threads.recoveryRefreshFailed"), variant: "default" });
      }
    } catch (cause) {
      const message = formatInvokeError(cause);
      setError(message);
      toast({
        title: t("threads.recoveryFailed"),
        description: message,
        variant: "destructive",
      });
      setPreviewVersion((version) => version + 1);
    } finally {
      setApplying(false);
    }
  };

  const provider = (value: string | null | undefined) =>
    value || t("threads.unknownProvider");

  return (
    <AlertDialog open={thread !== null} onOpenChange={(open) => !open && close()}>
      <AlertDialogContent className="flex max-h-[min(85dvh,880px)] min-w-0 flex-col gap-3 overflow-hidden p-4 transition-none sm:max-w-2xl sm:gap-4 sm:p-6">
        <AlertDialogHeader className="shrink-0 pr-8">
          <AlertDialogTitle className="flex items-center gap-2">
            <RefreshCcw className="h-4 w-4 shrink-0 text-primary" />
            {t("threads.recoveryTitle")}
          </AlertDialogTitle>
        </AlertDialogHeader>
        <Button
          type="button"
          variant="ghost"
          size="icon-sm"
          className="absolute right-4 top-4"
          aria-label={t("common.close")}
          disabled={applying}
          onClick={close}
        >
          <X className="h-4 w-4" aria-hidden="true" />
        </Button>

        <div className="min-h-0 min-w-0 space-y-4 overflow-y-auto overscroll-contain [overflow-wrap:anywhere]">
          <AlertDialogDescription>
            {thread
              ? t("threads.recoveryDescription", { title: thread.title || thread.id })
              : ""}
          </AlertDialogDescription>

          {previewLoading ? (
            <div className="flex min-h-40 items-center justify-center gap-2 text-sm text-muted-foreground">
              <LoaderCircle className="h-4 w-4 animate-spin" />
              {t("threads.recoveryPreviewLoading")}
            </div>
          ) : error && !preview ? (
            <div role="alert" className="rounded-lg border border-destructive/30 bg-destructive/5 p-3 text-sm">
              <p className="font-medium text-destructive">
                {t("threads.recoveryPreviewFailed")}
              </p>
              <p className="mt-1 break-words text-xs text-muted-foreground">{error}</p>
            </div>
          ) : preview ? (
            <div className="space-y-3">
              <div
                className={cn(
                  "rounded-lg border p-3 text-sm",
                  canConfirm
                    ? "border-amber-500/30 bg-amber-500/5"
                    : preview.status === "compatible"
                      ? "border-emerald-500/30 bg-emerald-500/5"
                      : "border-destructive/30 bg-destructive/5",
                )}
              >
                <p className="font-medium">
                  {t(`threads.recoveryStatus.${preview.status}`)}
                </p>
                {preview.status !== "officialAccountUnverified" && (
                  <p className="mt-1 text-xs text-muted-foreground">
                    {t(`threads.recoveryReason.${preview.status}`, { defaultValue: preview.reason })}
                  </p>
                )}
              </div>

              {adoptingRouter && (
                <p className="rounded-lg border border-amber-500/30 bg-amber-500/5 p-3 text-xs text-muted-foreground">
                  {t("threads.routerAdoptionDescription")}
                </p>
              )}

              {preview.coordination && (
                <div className="space-y-2 rounded-lg border border-destructive/30 bg-destructive/5 p-3 text-xs">
                  <div>
                    <p className="font-medium text-destructive">
                      {t("threads.recoveryCoordinationTitle")}
                    </p>
                    <p className="mt-1 text-muted-foreground">
                      {t("threads.recoveryCoordinationDescription")}
                    </p>
                  </div>
                  <div className="space-y-1 font-mono text-[10px] text-muted-foreground">
                    <p className="break-all">
                      {t("threads.recoveryJournalPath")}: {preview.coordination.journalPath}
                    </p>
                    {preview.coordination.backupPath && (
                      <p className="break-all">
                        {t("threads.recoveryBackupPath")}: {preview.coordination.backupPath}
                      </p>
                    )}
                    <p className="break-words text-destructive">{preview.coordination.detail}</p>
                  </div>
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    onClick={() => {
                      void api.openPath(preview.coordination!.directoryPath).catch((cause) => {
                        toast({
                          title: t("threads.recoveryOpenDirectoryFailed"),
                          description: formatInvokeError(cause),
                          variant: "destructive",
                        });
                      });
                    }}
                  >
                    <FolderOpen className="mr-1.5 h-3.5 w-3.5" />
                    {t("threads.recoveryOpenDirectory")}
                  </Button>
                </div>
              )}

              <div className="grid gap-2 rounded-lg border border-border/60 bg-muted/20 p-3 text-xs sm:grid-cols-2">
                <ProviderRow
                  label={t("threads.configuredProvider")}
                  value={provider(preview.configuredProvider)}
                />
                <ProviderRow
                  label={t("threads.recoveryTargetProvider")}
                  value={provider(preview.targetProvider)}
                  emphasis
                />
                <ProviderRow
                  label={t("threads.databaseProvider")}
                  value={provider(preview.databaseProvider)}
                />
                <ProviderRow
                  label={t("threads.rolloutProvider")}
                  value={provider(preview.rolloutProvider)}
                />
              </div>

              {preview.catalogAnalysis && (
                <div className="space-y-1.5 rounded-lg border border-border/60 bg-muted/20 p-3 text-xs">
                  <p className="font-medium text-foreground">
                    {t("threads.recoveryCatalogAnalysis")}
                  </p>
                  <p className="text-muted-foreground">
                    {preview.catalogAnalysis.ready
                      ? t("threads.recoveryCatalogReady")
                      : t("threads.recoveryCatalogBlocked")}
                  </p>
                  <p className="text-muted-foreground">
                    {t("threads.recoveryThreadRelation")}: {t(
                      "threads.recoveryRelation." + preview.catalogAnalysis.relation,
                    )}
                  </p>
                  {preview.catalogAnalysis.schemaSignature && (
                    <p className="break-all font-mono text-[10px] text-muted-foreground">
                      {t("threads.recoverySchemaSignature")}: {preview.catalogAnalysis.schemaSignature}
                    </p>
                  )}
                  {preview.catalogAnalysis.missingFields.length > 0 && (
                    <p className="text-destructive">
                      {t("threads.recoveryMissingFields", {
                        fields: preview.catalogAnalysis.missingFields.join(", "),
                      })}
                    </p>
                  )}
                  {preview.catalogAnalysis.unsupportedColumns.length > 0 && (
                    <p className="text-destructive">
                      {t("threads.recoveryUnsupportedColumns", {
                        columns: preview.catalogAnalysis.unsupportedColumns.join(", "),
                      })}
                    </p>
                  )}
                  {preview.catalogAnalysis.mappingErrors.length > 0 && (
                    <div className="space-y-1 text-destructive">
                      <p>{t("threads.recoveryMappingErrors")}</p>
                      <ul className="list-disc space-y-0.5 pl-4">
                        {preview.catalogAnalysis.mappingErrors.map((mappingError) => (
                          <li key={mappingError} className="break-words">
                            {mappingError}
                          </li>
                        ))}
                      </ul>
                    </div>
                  )}
                </div>
              )}

              {preview.catalogFields.length > 0 && (
                <div className="space-y-2 rounded-lg border border-border/60 bg-muted/20 p-3 text-xs">
                  <div>
                    <p className="font-medium text-foreground">
                      {t("threads.recoveryCatalogFields")}
                    </p>
                    <p className="mt-1 text-muted-foreground">
                      {t("threads.recoveryCatalogFieldsDescription")}
                    </p>
                  </div>
                  <dl className="grid gap-x-4 gap-y-2 sm:grid-cols-2">
                    {preview.catalogFields.map((field) => (
                      <div key={field.name} className="min-w-0">
                        <dt className="font-mono text-[10px] text-muted-foreground">
                          {field.name}
                        </dt>
                        <dd
                          className="break-words font-mono text-[11px] text-foreground"
                          title={field.value}
                        >
                          {field.value}
                        </dd>
                      </div>
                    ))}
                  </dl>
                </div>
              )}

              <div className="space-y-1.5">
                <p className="text-xs font-medium text-foreground">
                  {t("threads.recoveryChanges", { count: preview.changes.length })}
                </p>
                {preview.changes.length > 0 ? (
                  <div className="space-y-2">
                    {preview.changes.map((change, index) => (
                      <div
                        key={`${change.target}:${change.path}:${index}`}
                        className="rounded-lg border border-border/60 p-2.5"
                      >
                        <div className="flex items-center gap-1.5 text-xs font-medium">
                          {change.target === "database" ? (
                            <Database className="h-3.5 w-3.5 text-primary" />
                          ) : (
                            <FileJson className="h-3.5 w-3.5 text-primary" />
                          )}
                          {t(`threads.recoveryTarget.${change.target}`, {
                            defaultValue: change.target,
                          })}
                        </div>
                        <div className="mt-1 flex items-center gap-2 font-mono text-[11px]">
                          <span className="truncate text-muted-foreground">
                            {provider(change.fromProvider)}
                          </span>
                          <ArrowRight className="h-3 w-3 shrink-0 text-muted-foreground" />
                          <span className="truncate font-medium text-primary">
                            {change.toProvider}
                          </span>
                        </div>
                        <p
                          className="mt-1 break-all font-mono text-[10px] text-muted-foreground/80"
                          title={change.path}
                        >
                          {change.path}
                        </p>
                      </div>
                    ))}
                  </div>
                ) : (
                  <p className="rounded-lg border border-border/60 p-3 text-xs text-muted-foreground">
                    {t("threads.recoveryNoPlannedChanges")}
                  </p>
                )}
              </div>
            </div>
          ) : null}
        </div>

        {error && preview && (
          <div role="alert" className="max-h-[min(6rem,15dvh)] shrink-0 overflow-y-auto rounded-lg border border-destructive/30 bg-destructive/5 p-3 text-xs text-destructive [overflow-wrap:anywhere]">
            {error}
          </div>
        )}
        <AlertDialogFooter className="shrink-0 border-t border-border/60 pt-4">
          <AlertDialogCancel disabled={applying}>
            {canConfirm ? t("common.cancel") : t("common.close")}
          </AlertDialogCancel>
          {canConfirm && (
            <AlertDialogAction disabled={applying} onClick={(event) => void apply(event)}>
              {applying && <LoaderCircle className="mr-1.5 h-4 w-4 animate-spin" />}
              {applying
                ? t("threads.recoveryApplying")
                : adoptingRouter
                  ? t("threads.routerAdoptionConfirm")
                  : preview.status === "recoveryPending"
                    ? t("threads.recoveryPendingConfirm")
                    : preview.status === "missingCatalog"
                      ? t("threads.recoveryCatalogConfirm")
                      : t("threads.recoveryConfirm")}
            </AlertDialogAction>
          )}
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}

function ProviderRow({
  label,
  value,
  emphasis = false,
}: {
  label: string;
  value: string;
  emphasis?: boolean;
}) {
  return (
    <div className="min-w-0">
      <p className="text-muted-foreground">{label}</p>
      <p
        className={cn("truncate font-mono font-medium", emphasis && "text-primary")}
        title={value}
      >
        {value}
      </p>
    </div>
  );
}
