import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import type { Update, DownloadEvent } from "@tauri-apps/plugin-updater";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import { toast } from "@/hooks/use-toast";
import type { UpdateInstallabilityPayload } from "@/types";
import { isTauriRuntime } from "@/lib/tauri-runtime";
import { api } from "@/lib/api";
import { errorMessage, formatInvokeError } from "@/lib/invoke-error";

interface UpdateInfo {
  version: string;
  currentVersion: string;
  body: string | null;
}

interface DownloadProgress {
  total: number;
  downloaded: number;
}

type UpdateStatus = "idle" | "checking" | "available" | "downloading" | "installing" | "ready" | "error";

export type UpdateCheckResult = "available" | "up-to-date" | "error" | "unconfigured";

/** Tells the result of a check someone asked for; an available update shows its own overlay. */
export function announceUpdateCheck(t: TFunction, result: UpdateCheckResult) {
  if (result === "up-to-date") {
    toast({ title: t("settings.upToDate"), description: t("settings.upToDateDesc"), variant: "default" });
  } else if (result === "unconfigured") {
    toast({ title: t("baseline.updaterUnavailable"), description: t("baseline.updaterUnavailableDesc") });
  } else if (result === "error") {
    toast({ title: t("settings.updateCheckFailed"), description: t("settings.updateCheckFailedDesc"), variant: "destructive" });
  }
}

export function useUpdateCheck() {
  const { t } = useTranslation();
  const [status, setStatus] = useState<UpdateStatus>("idle");
  const [updateInfo, setUpdateInfo] = useState<UpdateInfo | null>(null);
  const [progress, setProgress] = useState<DownloadProgress | null>(null);
  const [error, setError] = useState<string | null>(null);
  const updateRef = useRef<Update | null>(null);
  const installedRef = useRef(false);

  const checkForUpdate = useCallback(async (): Promise<UpdateCheckResult> => {
    if (!isTauriRuntime()) {
      return "unconfigured";
    }
    if (installedRef.current) {
      setStatus("ready");
      setError(null);
      return "available";
    }
    setStatus("checking");
    setError(null);
    try {
      if (!(await api.isUpdaterConfigured())) {
        setStatus("idle");
        return "unconfigured";
      }
      const { check } = await import("@tauri-apps/plugin-updater");
      // GitHub is often reachable only through a proxy; the download reuses it.
      const proxy = await api.updaterProxy().catch(() => null);
      const update = await check(proxy ? { proxy } : undefined);
      if (update) {
        updateRef.current = update;
        setUpdateInfo({
          version: update.version,
          currentVersion: update.currentVersion,
          body: update.body ?? null,
        });
        setStatus("available");
        return "available";
      }
      setStatus("idle");
      return "up-to-date";
    } catch (e) {
      setError(formatInvokeError(e));
      setStatus("error");
      return "error";
    }
  }, []);

  const installUpdate = useCallback(async () => {
    const update = updateRef.current;
    if (!update && !installedRef.current) return;

    try {
      const { invoke } = await import("@tauri-apps/api/core");
      if (installedRef.current) {
        setStatus("ready");
        setError(null);
        await invoke("graceful_restart_for_update");
        return;
      }
      const installability = await invoke<UpdateInstallabilityPayload>(
        "check_update_installability",
      );
      if (!installability.canInstall) {
        setError(localizeUpdateInstallabilityError(t, installability));
        setStatus("error");
        return;
      }

      setStatus("downloading");
      setProgress({ total: 0, downloaded: 0 });

      let totalBytes = 0;
      let downloadedBytes = 0;

      await update!.downloadAndInstall((event: DownloadEvent) => {
        if (event.event === "Started" && event.data.contentLength) {
          totalBytes = event.data.contentLength;
          setProgress({ total: totalBytes, downloaded: 0 });
        } else if (event.event === "Progress") {
          downloadedBytes += event.data.chunkLength;
          setProgress({ total: totalBytes, downloaded: downloadedBytes });
        } else if (event.event === "Finished") {
          setStatus("installing");
        }
      });

      installedRef.current = true;
      setStatus("ready");
      await invoke("graceful_restart_for_update");
    } catch (e) {
      setError(localizeUpdateRuntimeError(t, e));
      setStatus("error");
    }
  }, [t]);

  useEffect(() => {
    if (!isTauriRuntime()) return;
    const unlisten = listen<{ intent: "quit" | "restart" }>("claude-desktop-exit-cancelled", event => {
      if (event.payload.intent === "restart" && installedRef.current) {
        setStatus("idle");
        setError(null);
      }
    });
    return () => { void unlisten.then(remove => remove()).catch(() => {}); };
  }, []);

  useEffect(() => {
    if (!isTauriRuntime()) return;
    const timer = setTimeout(() => {
      checkForUpdate();
    }, 1500);
    return () => clearTimeout(timer);
  }, [checkForUpdate]);

  const dismiss = useCallback(() => {
    setStatus("idle");
    setError(null);
    if (updateRef.current) {
      updateRef.current.close().catch(() => {});
      updateRef.current = null;
    }
  }, []);

  return {
    status,
    updateInfo,
    progress,
    error,
    checkForUpdate,
    installUpdate,
    dismiss,
  };
}

function localizeUpdateInstallabilityError(
  t: (key: string) => string,
  installability: UpdateInstallabilityPayload,
) {
  if (installability.code === "app_translocation") {
    return t("update.installBlockedAppTranslocation");
  }
  if (installability.code === "read_only_location") {
    return t("update.installBlockedReadOnlyLocation");
  }
  return t("update.installBlocked");
}

function localizeUpdateRuntimeError(t: (key: string) => string, error: unknown) {
  const message = errorMessage(error);
  if (
    message.includes("Read-only file system") ||
    message.includes("os error 30")
  ) {
    return t("update.installBlockedAppTranslocation");
  }
  return message;
}
