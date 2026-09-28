import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Server } from "lucide-react";
import { api } from "@/lib/api";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
} from "@/components/ui/dialog";
import type { McpServerSummary, McpTransport } from "@/types";
import { queryKeys } from "@/lib/query-keys";
import { InputField, TextareaField } from "@/components/mcp/mcp-form-fields";

export function McpEditorDialog({
  open,
  server,
  existingNames,
  onClose,
}: {
  open: boolean;
  server?: McpServerSummary;
  existingNames: string[];
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [name, setName] = useState("");
  const [transport, setTransport] = useState<McpTransport>("stdio");
  const [command, setCommand] = useState("");
  const [args, setArgs] = useState("");
  const [url, setUrl] = useState("");
  const [envText, setEnvText] = useState("");
  const [headersText, setHeadersText] = useState("");

  useEffect(() => {
    if (open) {
      setName(server?.name ?? "");
      setTransport(server?.transport ?? "stdio");
      setCommand(server?.command ?? "");
      setArgs(server?.args.join("\n") ?? "");
      setUrl(server?.url ?? "");
      setEnvText(
        server?.environment ? Object.entries(server.environment).map(([k, v]) => `${k}=${v}`).join("\n") : ""
      );
      setHeadersText(
        server?.headers ? Object.entries(server.headers).map(([k, v]) => `${k}: ${v}`).join("\n") : ""
      );
    }
  }, [open, server]);

  const trimmedName = name.trim();
  // Saving a new server under an existing name would silently replace that server.
  const duplicateName = !server && existingNames.includes(trimmedName);

  const upsertMutation = useMutation({
    mutationFn: () => {
      const environment: Record<string, string> = {};
      envText.split("\n").filter(Boolean).forEach((line) => {
        const idx = line.indexOf("=");
        if (idx > 0) environment[line.slice(0, idx).trim()] = line.slice(idx + 1).trim();
      });
      const headers: Record<string, string> = {};
      headersText.split("\n").filter(Boolean).forEach((line) => {
        const idx = line.indexOf(":");
        if (idx > 0) headers[line.slice(0, idx).trim()] = line.slice(idx + 1).trim();
      });
      return api.upsertMcpServer({
        name: trimmedName,
        transport,
        enabled: server?.enabled ?? true,
        command: transport === "stdio" ? command : null,
        args: transport === "stdio" ? args.split("\n").map((s) => s.replace(/\r$/, "")).filter((s) => s.length > 0) : [],
        url: transport !== "stdio" ? url : null,
        headers,
        environment,
      });
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.mcpServers });
      onClose();
    },
  });

  return (
    <Dialog open={open} onOpenChange={(v) => !v && onClose()}>
      <DialogContent className="max-h-[90vh] overflow-y-auto sm:max-w-2xl">
        <DialogHeader className="border-b border-border/40 pb-3">
          <div className="flex items-center gap-3">
            <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl bg-primary/10 text-primary ring-1 ring-inset ring-primary/15">
              <Server className="h-4 w-4" />
            </div>
            <div>
              <DialogTitle className="text-base font-semibold">{server ? t("mcp.edit") : t("mcp.add")}</DialogTitle>
              <p className="mt-0.5 text-xs text-muted-foreground">{t("mcp.description")}</p>
            </div>
          </div>
        </DialogHeader>

        <div className="grid gap-4 sm:grid-cols-2 pt-1">
          {/* Left Column: Basic Service Configuration */}
          <div className="space-y-3.5">
            <InputField label={t("mcp.name")} value={name} onChange={setName} disabled={!!server} placeholder="e.g. filesystem" />
            <div className="space-y-1.5">
              <label className="text-xs font-medium text-muted-foreground">{t("mcp.transport")}</label>
              <div className="flex gap-2">
                {(["stdio", "http", "sse"] as McpTransport[]).map((tr) => (
                  <Button
                    key={tr}
                    variant={transport === tr ? "soft" : "outline"}
                    size="sm"
                    className="flex-1"
                    onClick={() => setTransport(tr)}
                  >
                    {tr.toUpperCase()}
                  </Button>
                ))}
              </div>
            </div>
            {transport === "stdio" ? (
              <InputField label={t("mcp.command")} value={command} onChange={setCommand} mono placeholder="npx / uvx / python" />
            ) : (
              <InputField label={t("mcp.url")} value={url} onChange={setUrl} mono placeholder="https://..." />
            )}
          </div>

          {/* Right Column: Parameters, Headers & Environment */}
          <div className="space-y-3.5">
            {transport === "stdio" ? (
              <TextareaField label={t("baseline.argsPerLine")} value={args} onChange={setArgs} mono placeholder={"-y\n@modelcontextprotocol/server-filesystem"} />
            ) : (
              <TextareaField label={t("mcp.headers")} value={headersText} onChange={setHeadersText} placeholder="Authorization: Bearer sk-..." mono />
            )}
            <TextareaField label={t("mcp.env")} value={envText} onChange={setEnvText} placeholder="API_KEY=your_key" mono />
          </div>
        </div>

        <DialogFooter className="border-t border-border/40 pt-3">
          <Button variant="outline" size="sm" onClick={onClose}>
            {t("mcp.cancel")}
          </Button>
          {duplicateName && (
            <p role="alert" className="mr-auto self-center text-xs text-destructive">{t("mcp.duplicateName")}</p>
          )}
          <Button size="sm" onClick={() => upsertMutation.mutate()} disabled={!trimmedName || duplicateName || upsertMutation.isPending}>
            {t("mcp.save")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
