import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { useTranslation } from "react-i18next";
import { Loader2 } from "lucide-react";
import { claudeDesktopApi, desktopErrorKey } from "@/lib/claude-desktop";
import type { ClaudeDesktopExitRequest } from "@/types/claude-desktop";
import {
  AlertDialog, AlertDialogAction, AlertDialogCancel, AlertDialogContent,
  AlertDialogDescription, AlertDialogFooter, AlertDialogHeader, AlertDialogTitle,
} from "@/components/ui/alert-dialog";

/** 所有完整退出入口都由后端协调；此处只提交用户选择。 */
export function ClaudeDesktopRouterExitDialog() {
  const { t } = useTranslation();
  const [request, setRequest] = useState<ClaudeDesktopExitRequest | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const exitError = (cause: unknown, fallback: string) => desktopErrorKey(cause, fallback);

  useEffect(() => {
    let alive = true;
    let receivedEvent = false;
    const unlisten = listen<ClaudeDesktopExitRequest>("claude-desktop-exit-required", event => {
      if (!alive) return;
      receivedEvent = true;
      setRequest(event.payload);
      setError(event.payload.error ? exitError(event.payload.error, "claudeDesktop.errors.storage") : null);
    });
    // 先监听再读取：后端可能在前端初始化前已经收到退出请求。
    void unlisten.then(async () => {
      const pending = await claudeDesktopApi.exitRequest();
      if (alive && !receivedEvent && pending) {
        setRequest(pending);
        setError(pending.error ? exitError(pending.error, "claudeDesktop.errors.storage") : null);
      }
    }).catch(() => { /* 浏览器预览没有 Tauri 退出请求。 */ });
    return () => { alive = false; void unlisten.then(remove => remove()).catch(() => {}); };
  }, []);

  const resolve = async (decision: "background" | "stopAndExit" | "leaveAndExit") => {
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      await claudeDesktopApi.resolveExit(decision);
      setRequest(null);
    } catch (cause) {
      setError(exitError(cause, "claudeDesktop.errors.process"));
    } finally {
      setBusy(false);
    }
  };

  return (
    <AlertDialog open={request !== null}>
      <AlertDialogContent data-desktop-exit-dialog>
        <AlertDialogHeader>
          <AlertDialogTitle>{t(request?.intent === "restart" ? "claudeDesktop.exit.restartTitle" : "claudeDesktop.exit.quitTitle")}</AlertDialogTitle>
          <AlertDialogDescription>{t("claudeDesktop.exit.description")}</AlertDialogDescription>
        </AlertDialogHeader>
        {error && <p role="alert" className="break-words text-sm text-destructive">{t(error)}</p>}
        {/* A restore that keeps failing must never trap the app: Desktop can be
            left on the local gateway, which works again when AgentHarbor runs. */}
        {error && (
          <div className="space-y-2 rounded-lg border border-border/70 bg-muted/40 px-3 py-2.5" data-desktop-exit-leave>
            <p className="text-xs text-muted-foreground">{t("claudeDesktop.exit.leaveHint")}</p>
            <button type="button" disabled={busy} onClick={() => void resolve("leaveAndExit")}
              className="text-xs font-medium text-primary underline-offset-2 hover:underline disabled:opacity-50">
              {t(request?.intent === "restart" ? "claudeDesktop.exit.leaveAndRestart" : "claudeDesktop.exit.leaveAndExit")}
            </button>
          </div>
        )}
        <AlertDialogFooter>
          <AlertDialogCancel disabled={busy} onClick={event => { event.preventDefault(); void resolve("background"); }}>
            {t("claudeDesktop.exit.background")}
          </AlertDialogCancel>
          <AlertDialogAction disabled={busy} onClick={event => { event.preventDefault(); void resolve("stopAndExit"); }}>
            {busy && <Loader2 className="h-4 w-4 animate-spin" />}
            {t(busy ? "claudeDesktop.exit.restoring"
              : request?.intent === "restart" ? "claudeDesktop.exit.stopAndRestart" : "claudeDesktop.exit.stopAndExit")}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
