import { useState } from "react";
import { useTranslation } from "react-i18next";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Check, ChevronDown, Copy, FolderOpen, Loader2, Play, RefreshCw, SquareTerminal } from "lucide-react";
import { api } from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";
import { storageKeys } from "@/lib/storage-keys";
import { errorKey } from "@/lib/invoke-error";
import { toast } from "@/hooks/use-toast";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import type { TargetId } from "@/types/clients";

const RECENT = 6;

interface Folders {
  recent: string[];
  last: Partial<Record<TargetId, string>>;
}

const readFolders = (): Folders => {
  try {
    const value = JSON.parse(localStorage.getItem(storageKeys.launchFolders) ?? "null") as Partial<Folders> | null;
    return {
      recent: Array.isArray(value?.recent) ? value.recent.filter(item => typeof item === "string") : [],
      last: value?.last && typeof value.last === "object" ? value.last : {},
    };
  } catch {
    return { recent: [], last: {} };
  }
};

const rememberFolder = (target: TargetId, folder: string): Folders => {
  const current = readFolders();
  const next = { recent: [folder, ...current.recent.filter(item => item !== folder)].slice(0, RECENT), last: { ...current.last, [target]: folder } };
  try {
    localStorage.setItem(storageKeys.launchFolders, JSON.stringify(next));
  } catch {
    // Storage unavailable: the folder is asked for again next time.
  }
  return next;
};

const folderName = (path: string) => path.replace(/[\\/]+$/, "").split(/[\\/]/).pop() || path;

/** How a client panel starts its client: a terminal in a remembered folder, an app, or a command to copy. */
export function useClientLaunch(target: TargetId, harnessProfile?: string | null) {
  return useQuery({
    queryKey: queryKeys.clientLaunch(target, harnessProfile),
    queryFn: () => api.targets.launchPlan(target, harnessProfile),
    // Apps are watched so the button says "start" or "restart" as they come and go.
    refetchInterval: query => query.state.data?.kind === "app" ? 10_000 : false,
  });
}

// The start button of a CLI client's panel. The first start asks for the
// folder to work in; the folder is remembered per client and recent folders
// stay one click away in the menu.
export function ClientLaunch({ target, name, harnessProfile, disabled }: {
  target: TargetId;
  name: string;
  harnessProfile?: string | null;
  disabled?: boolean;
}) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const plan = useClientLaunch(target, harnessProfile);
  const [folders, setFolders] = useState(readFolders);
  const [busy, setBusy] = useState(false);
  const [copied, setCopied] = useState(false);
  const data = plan.data;
  if (!data) return null;

  const run = async (folder?: string) => {
    setBusy(true);
    try {
      await api.targets.launch(target, harnessProfile, folder ?? null);
      if (folder) setFolders(rememberFolder(target, folder));
      toast({
        title: t(data.kind === "app" ? (data.running ? "clients.launch.restarted" : "clients.launch.started") : "clients.launch.opened", { name }),
        description: folder,
        variant: "success",
      });
    } catch (error) {
      toast({ title: t(errorKey(error, ["clients"], "clients.errors.launch"), { name }), variant: "destructive" });
    } finally {
      setBusy(false);
      void queryClient.invalidateQueries({ queryKey: queryKeys.clientLaunch(target, harnessProfile) });
    }
  };

  const pick = async () => {
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const chosen = await open({ directory: true, multiple: false, defaultPath: folders.last[target] ?? folders.recent[0] });
      if (typeof chosen === "string") await run(chosen);
    } catch (error) {
      toast({ title: t("clients.errors.launch", { name }), description: String(error), variant: "destructive" });
    }
  };

  const buttonClass = "h-8 gap-1.5 text-xs";
  if (data.kind === "copy") {
    return (
      <Button type="button" size="sm" variant="outline" className={`${buttonClass} rounded-lg`} data-client-launch="copy"
        title={data.command} disabled={disabled}
        onClick={() => {
          void navigator.clipboard?.writeText(data.command).then(() => {
            setCopied(true);
            window.setTimeout(() => setCopied(false), 1500);
          });
        }}>
        {copied ? <Check className="h-3.5 w-3.5 text-emerald-500" /> : <Copy className="h-3.5 w-3.5" />}
        {t("clients.launch.copy")}
      </Button>
    );
  }

  if (data.kind === "app") {
    return (
      <Button type="button" size="sm" variant="outline" className={`${buttonClass} rounded-lg`} data-client-launch="app"
        disabled={disabled || busy || !data.installed} title={data.installed ? undefined : t("clients.launch.appMissing", { name })}
        onClick={() => void run()}>
        {busy ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : data.running ? <RefreshCw className="h-3.5 w-3.5" /> : <Play className="h-3.5 w-3.5" />}
        {t(data.running ? "clients.launch.restart" : "clients.launch.start", { name })}
      </Button>
    );
  }

  const folder = folders.last[target];
  const locked = disabled || busy || !data.installed;
  return (
    <div className="inline-flex shrink-0 items-stretch rounded-lg shadow-sm" data-client-launch="terminal">
      <Button type="button" size="sm" variant="outline" className={`${buttonClass} rounded-r-none border-r-0`} disabled={locked}
        title={!data.installed ? t("clients.launch.cliMissing", { command: data.command })
          : folder ? t("clients.launch.runIn", { command: data.command, folder }) : t("clients.launch.pickFirst")}
        onClick={() => void (folder ? run(folder) : pick())}>
        {busy ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <SquareTerminal className="h-3.5 w-3.5" />}
        {t("clients.launch.terminal")}
        {folder && <span className="max-w-24 truncate font-mono text-[11px] text-muted-foreground">{folderName(folder)}</span>}
      </Button>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button type="button" size="sm" variant="outline" className="h-8 w-7 rounded-l-none px-0" disabled={locked}
            aria-label={t("clients.launch.folders")}>
            <ChevronDown className="h-3.5 w-3.5" />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" className="w-72">
          <DropdownMenuLabel className="text-[11px] font-medium text-muted-foreground">
            {t("clients.launch.runCommand", { command: data.command })}
          </DropdownMenuLabel>
          {folders.recent.map(path => (
            <DropdownMenuItem key={path} onSelect={() => void run(path)} className="flex-col items-start gap-0">
              <span className="flex w-full items-center gap-1.5 text-xs font-medium">
                {folderName(path)}
                {path === folder && <Check className="ml-auto h-3.5 w-3.5 text-primary" />}
              </span>
              <span className="w-full truncate font-mono text-[10.5px] text-muted-foreground" title={path}>{path}</span>
            </DropdownMenuItem>
          ))}
          {folders.recent.length > 0 && <DropdownMenuSeparator />}
          <DropdownMenuItem onSelect={() => void pick()} className="gap-2 text-xs">
            <FolderOpen className="h-3.5 w-3.5" />{t("clients.launch.pick")}
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
    </div>
  );
}
