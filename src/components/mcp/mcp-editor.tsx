import { useState } from "react";
import { useTranslation } from "react-i18next";
import { ClipboardPaste, Server, Wand2 } from "lucide-react";
import { api } from "@/lib/api";
import { cn } from "@/lib/utils";
import { storageKeys } from "@/lib/storage-keys";
import { formatInvokeError } from "@/lib/invoke-error";
import { toast } from "@/hooks/use-toast";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Dialog, DialogContent, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { SegmentedOptions } from "@/components/ui/segmented-options";
import { Textarea } from "@/components/ui/textarea";
import type { McpApp, McpKind, McpLibraryView, McpServerView, McpSpec, PastedMcpServer } from "@/types";
import { MCP_APPS, MCP_TEMPLATES, linesOf, mapOf } from "./mcp-templates";
import type { McpLibrary } from "./use-mcp-library";

const NAME = /^[A-Za-z0-9_.@-]{1,64}$/;
const HARNESS_NAME = /^[A-Za-z0-9_-]{1,32}$/;

/** Why `app` cannot hold this server, as an i18n key, mirroring apps.rs. */
export function unsupportedReason(app: McpApp, name: string, spec: McpSpec) {
  if (spec.transport === "sse" && (app === "codex" || app === "pi" || app === "harness")) return "mcp.unsupported.sse";
  if (spec.transport !== "stdio" && app === "claudeDesktop") return "mcp.unsupported.remote";
  if (app === "harness" && name && !HARNESS_NAME.test(name)) return "mcp.unsupported.harnessName";
  return null;
}

/** The agents new servers were last written to. */
function rememberedApps(installed: McpApp[]): McpApp[] {
  try {
    const saved = JSON.parse(localStorage.getItem(storageKeys.mcpTargets) ?? "null");
    if (Array.isArray(saved)) return saved.filter((app): app is McpApp => installed.includes(app));
  } catch {
    // Unreadable: fall through to the default.
  }
  return installed.filter(app => app === "codex" || app === "claudeCode");
}

type Draft = {
  name: string; transport: McpKind; command: string; args: string; env: string; cwd: string;
  url: string; headers: string; note: string;
};

const draftOf = (name: string, spec: McpSpec, note = ""): Draft => ({
  name, transport: spec.transport, command: spec.command ?? "", args: (spec.args ?? []).join("\n"),
  env: linesOf(spec.env, "="), cwd: spec.cwd ?? "", url: spec.url ?? "", headers: linesOf(spec.headers, ": "), note,
});

const specOf = (draft: Draft): McpSpec => draft.transport === "stdio"
  ? {
    transport: "stdio", command: draft.command.trim(),
    args: draft.args.split("\n").map(arg => arg.replace(/\r$/, "")).filter(Boolean),
    env: mapOf(draft.env, "="), ...(draft.cwd.trim() ? { cwd: draft.cwd.trim() } : {}),
  }
  : { transport: draft.transport, url: draft.url.trim(), headers: mapOf(draft.headers, ":") };

export function McpEditor({ server, view, library, onClose }: {
  /** The server to edit; null adds one. */
  server: McpServerView | null;
  view: McpLibraryView;
  library: McpLibrary;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const installed = view.apps.filter(app => app.installed).map(app => app.app);
  const [draft, setDraft] = useState<Draft>(() => server
    ? draftOf(server.name, server.spec, server.note)
    : draftOf("", { transport: "stdio" }));
  const [apps, setApps] = useState<McpApp[]>(() => server
    ? server.cells.filter(cell => cell.on).map(cell => cell.app)
    : rememberedApps(installed));
  const [paste, setPaste] = useState("");
  const [pasted, setPasted] = useState<PastedMcpServer[]>([]);
  const [recognizing, setRecognizing] = useState(false);
  const update = (patch: Partial<Draft>) => setDraft(current => ({ ...current, ...patch }));

  const name = draft.name.trim();
  const spec = specOf(draft);
  const taken = view.servers.map(item => item.name);
  const problem = !NAME.test(name) ? (name ? "mcp.errors.name" : "")
    : !server && taken.includes(name) ? "mcp.errors.duplicate"
      : draft.transport === "stdio" ? (spec.command ? "" : "mcp.errors.command")
        : /^https?:\/\/\S+$/i.test(spec.url ?? "") ? "" : "mcp.errors.url";

  const fill = (item: PastedMcpServer) => {
    setDraft(current => ({ ...draftOf(item.name || current.name, item.spec), note: current.note }));
  };

  const recognize = async () => {
    setRecognizing(true);
    try {
      const servers = await api.parseMcpPaste(paste);
      if (servers.length === 1) {
        fill(servers[0]);
        setPasted([]);
        toast({ title: t("mcp.editor.recognized"), variant: "success" });
      } else {
        setPasted(servers);
      }
    } catch (error) {
      toast({ title: formatInvokeError(error), variant: "destructive" });
    } finally {
      setRecognizing(false);
    }
  };

  // A whole command line typed into the command field is split into arguments.
  const splitCommand = async () => {
    if (!/\s/.test(draft.command.trim()) || draft.args.trim()) return;
    try {
      const [item] = await api.parseMcpPaste(draft.command);
      if (item?.spec.transport === "stdio") update({ command: item.spec.command ?? "", args: (item.spec.args ?? []).join("\n") });
    } catch {
      // Not a command line: leave it for the user.
    }
  };

  const remember = () => {
    if (server) return;
    try {
      localStorage.setItem(storageKeys.mcpTargets, JSON.stringify(apps));
    } catch {
      // Storage unavailable; next time starts from the default.
    }
  };

  const save = async () => {
    if (problem) return;
    remember();
    const saved = await library.run(() => api.saveMcpServer(server ? server.name : null, {
      name, spec, note: draft.note.trim(), apps: apps.filter(app => !unsupportedReason(app, name, spec)),
    }));
    if (saved) onClose();
  };

  const addAll = async () => {
    remember();
    let added = 0;
    let skipped = 0;
    for (const item of pasted) {
      if (!item.name || taken.includes(item.name)) {
        skipped += 1;
        continue;
      }
      const writeTo = apps.filter(app => !unsupportedReason(app, item.name, item.spec));
      const saved = await library.run(() => api.saveMcpServer(null, { name: item.name, spec: item.spec, note: "", apps: writeTo }));
      if (saved) added += 1; else skipped += 1;
    }
    toast({ title: t("mcp.editor.addedAll", { count: added, skipped }), variant: added > 0 ? "success" : "default" });
    if (added > 0) onClose();
  };

  const label = "text-xs font-medium text-muted-foreground";
  const field = (key: keyof Draft, text: string, options: { mono?: boolean; area?: boolean; placeholder?: string; hint?: string; onBlur?: () => void } = {}) => (
    <label className="block space-y-1.5">
      <span className={label}>{text}</span>
      {options.area ? (
        <Textarea value={draft[key]} onChange={event => update({ [key]: event.target.value })} rows={3}
          placeholder={options.placeholder} className={cn(options.mono && "font-mono text-xs")} />
      ) : (
        <Input value={draft[key]} onChange={event => update({ [key]: event.target.value })} onBlur={options.onBlur}
          placeholder={options.placeholder} disabled={key === "name" && !!server} className={cn(options.mono && "font-mono")} />
      )}
      {options.hint && <span className="block text-[11px] leading-relaxed text-muted-foreground/80">{options.hint}</span>}
    </label>
  );

  return (
    <Dialog open onOpenChange={open => !open && !library.busy && onClose()}>
      <DialogContent className="max-h-[90vh] overflow-y-auto sm:max-w-3xl" data-mcp-editor>
        <DialogHeader className="border-b border-border/40 pb-3">
          <div className="flex items-center gap-3">
            <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl bg-primary/10 text-primary ring-1 ring-inset ring-primary/15">
              <Server className="h-4 w-4" />
            </div>
            <DialogTitle className="text-base font-semibold">
              {server ? t("mcp.editor.editTitle", { name: server.name }) : t("mcp.editor.addTitle")}
            </DialogTitle>
          </div>
        </DialogHeader>

        <fieldset disabled={library.busy} className="space-y-5">
          {!server && (
            <div className="space-y-3">
              <div className="space-y-1.5">
                <p className={label}>{t("mcp.editor.templates")}</p>
                <div className="flex flex-wrap gap-1.5">
                  {MCP_TEMPLATES.map(template => (
                    <Button key={template.name} type="button" size="sm" variant="outline" className="h-7 rounded-full px-3 text-xs"
                      onClick={() => fill({ name: template.name, spec: template.spec })}>
                      {template.label}
                    </Button>
                  ))}
                </div>
              </div>
              <div className="space-y-1.5">
                <p className={label}>{t("mcp.editor.paste")}</p>
                <div className="flex items-start gap-2">
                  <Textarea value={paste} onChange={event => setPaste(event.target.value)} rows={2} aria-label={t("mcp.editor.paste")}
                    placeholder={t("mcp.editor.pastePlaceholder")} className="min-h-[3.25rem] font-mono text-xs" />
                  <Button type="button" variant="outline" size="sm" className="shrink-0 gap-1.5" disabled={!paste.trim() || recognizing}
                    onClick={() => void recognize()}>
                    <ClipboardPaste className="h-3.5 w-3.5" /> {t("mcp.editor.recognize")}
                  </Button>
                </div>
                {pasted.length > 1 && (
                  <div className="rounded-lg border border-border/60 bg-muted/30 p-2.5" data-mcp-pasted>
                    <div className="mb-2 flex items-center justify-between gap-2">
                      <span className="text-xs font-medium">{t("mcp.editor.pasteMany", { count: pasted.length })}</span>
                      <Button type="button" size="sm" className="h-7 gap-1.5 text-xs" onClick={() => void addAll()}>
                        <Wand2 className="h-3.5 w-3.5" /> {t("mcp.editor.addAll")}
                      </Button>
                    </div>
                    <ul className="space-y-1">
                      {pasted.map((item, index) => (
                        <li key={`${item.name}-${index}`} className="flex items-center justify-between gap-2 text-xs">
                          <span className="min-w-0 truncate font-mono">
                            {item.name || "—"} <span className="text-muted-foreground">· {item.spec.transport === "stdio" ? item.spec.command : item.spec.url}</span>
                          </span>
                          {taken.includes(item.name)
                            ? <span className="shrink-0 text-muted-foreground">{t("mcp.editor.exists")}</span>
                            : <Button type="button" variant="ghost" size="sm" className="h-6 shrink-0 px-2 text-xs" onClick={() => fill(item)}>{t("mcp.editor.useOne")}</Button>}
                        </li>
                      ))}
                    </ul>
                  </div>
                )}
              </div>
            </div>
          )}

          <div className="grid gap-4 sm:grid-cols-2">
            <div className="space-y-3.5">
              {field("name", t("mcp.editor.name"), { mono: true, placeholder: "context7", hint: server ? undefined : t("mcp.editor.nameHint") })}
              <div className="space-y-1.5">
                <span className={label}>{t("mcp.editor.transport")}</span>
                <div>
                  <SegmentedOptions value={draft.transport} onChange={value => update({ transport: value as McpKind })}
                    items={[{ value: "stdio", label: t("mcp.editor.stdio") }, { value: "http", label: "HTTP" }, { value: "sse", label: "SSE" }]} />
                </div>
              </div>
              {draft.transport === "stdio"
                ? field("command", t("mcp.editor.command"), { mono: true, placeholder: "npx", hint: t("mcp.editor.commandHint"), onBlur: () => void splitCommand() })
                : field("url", t("mcp.editor.url"), { mono: true, placeholder: "https://…/mcp" })}
              {draft.transport === "stdio" && field("cwd", t("mcp.editor.cwd"), { mono: true })}
            </div>
            <div className="space-y-3.5">
              {draft.transport === "stdio" ? (
                <>
                  {field("args", t("mcp.editor.args"), { mono: true, area: true, placeholder: "-y\n@upstash/context7-mcp" })}
                  {field("env", t("mcp.editor.env"), { mono: true, area: true, placeholder: "API_KEY=…" })}
                </>
              ) : field("headers", t("mcp.editor.headers"), { mono: true, area: true, placeholder: "Authorization: Bearer …" })}
              {field("note", t("mcp.editor.note"))}
            </div>
          </div>

          <div className="space-y-2" data-mcp-write-to>
            <p className={label}>{t("mcp.editor.writeTo")}</p>
            <div className="flex flex-wrap gap-x-4 gap-y-2">
              {MCP_APPS.map(app => {
                const target = view.apps.find(item => item.app === app);
                const reason = !target?.installed ? "mcp.column.notInstalled" : unsupportedReason(app, name, spec);
                const checked = !reason && apps.includes(app);
                return (
                  <label key={app} title={reason ? t(reason) : target?.path}
                    className={cn("inline-flex items-center gap-2 text-sm", reason ? "cursor-not-allowed opacity-50" : "cursor-pointer")}>
                    <Checkbox checked={checked} disabled={!!reason} aria-label={t(`mcp.apps.${app}`)}
                      onCheckedChange={on => setApps(current => on === true ? [...new Set([...current, app])] : current.filter(item => item !== app))} />
                    {t(`mcp.apps.${app}`)}
                  </label>
                );
              })}
            </div>
          </div>
        </fieldset>

        <DialogFooter className="border-t border-border/40 pt-3">
          {problem && name && <p role="alert" className="mr-auto self-center text-xs text-destructive">{t(problem)}</p>}
          <Button variant="outline" size="sm" disabled={library.busy} onClick={onClose}>{t("mcp.editor.cancel")}</Button>
          <Button size="sm" disabled={!!problem || library.busy} onClick={() => void save()}>{t("mcp.editor.save")}</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
