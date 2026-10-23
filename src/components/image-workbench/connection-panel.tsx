import { useEffect, useId, useRef, useState } from "react";
import { ChevronDown, PlugZap } from "lucide-react";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { SecretInput } from "@/components/ui/secret-input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { cn } from "@/lib/utils";
import { InkModelCombobox } from "./ink-model-combobox";
import type { ImageConnection } from "./use-image-connection";

/** Where a manual site points, short enough for the folded summary. */
const hostOf = (url: string) => {
  try {
    return new URL(url.trim()).host || url.trim();
  } catch {
    return url.trim();
  }
};

/**
 * Provider, site, key and model: set once and rarely touched again, so the panel folds
 * into a one-line summary. It starts folded when an earlier visit saved a setup.
 */
export function ConnectionPanel({ connection, open, onOpenChange }: {
  connection: ImageConnection;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const bodyId = useId();
  const body = useRef<HTMLDivElement>(null);
  // Folded fields keep their state but leave the tab order and the accessibility tree.
  useEffect(() => { body.current?.toggleAttribute("inert", !open); }, [open]);
  const ready = connection.hasCredentials && connection.model.trim().length > 0;
  const source = connection.providerId
    ? connection.providerName
    : connection.baseUrl.trim() ? hostOf(connection.baseUrl) : "未选择供应商";
  return (
    <section data-image-connection data-open={open || undefined}
      className="rounded-xl bg-black/[0.025] ring-1 ring-border/50 dark:bg-white/[0.02]">
      <button
        type="button"
        aria-expanded={open}
        aria-controls={bodyId}
        aria-label={open ? "收起连接设置" : "展开连接设置"}
        onClick={() => onOpenChange(!open)}
        className="flex w-full min-w-0 items-center gap-2 rounded-xl px-3 py-2.5 text-left outline-none transition-colors hover:bg-black/[0.02] focus-visible:ring-2 focus-visible:ring-[#b83b38]/35 dark:hover:bg-white/[0.03]"
      >
        <PlugZap className="h-3.5 w-3.5 shrink-0 text-[#b83b38] dark:text-[#d94f4b]" />
        <span className="shrink-0 font-serif text-xs font-semibold text-foreground/90">连接</span>
        <span className="min-w-0 flex-1 truncate font-serif text-xs text-muted-foreground">
          {source}{connection.model.trim() && ` · ${connection.model.trim()}`}
        </span>
        <span className={cn(
          "inline-flex shrink-0 items-center gap-1 rounded-full px-1.5 py-0.5 font-serif text-[10px]",
          ready
            ? "bg-emerald-500/10 text-emerald-700 dark:text-emerald-300"
            : "bg-amber-500/10 text-amber-700 dark:text-amber-300",
        )}>
          <span className={cn("h-1.5 w-1.5 rounded-full", ready ? "bg-emerald-500" : "bg-amber-500")} />
          {ready ? "已就绪" : "待配置"}
        </span>
        <ChevronDown className={cn("h-3.5 w-3.5 shrink-0 text-muted-foreground transition-transform duration-300 motion-reduce:transition-none", open && "rotate-180")} />
      </button>

      {/* Folds by animating the row height, so the page below slides instead of jumping. */}
      <div id={bodyId} ref={body}
        className={cn(
          "grid transition-[grid-template-rows,opacity,visibility] duration-300 ease-out motion-reduce:transition-none",
          open ? "visible grid-rows-[1fr] opacity-100" : "invisible grid-rows-[0fr] opacity-0",
        )}>
        <div className="min-h-0 overflow-hidden">
          <div className="grid gap-4 border-t border-border/40 px-3 pb-3 pt-3 sm:grid-cols-2">
            <div className="space-y-1.5 sm:col-span-2">
              <Label className="font-serif text-xs font-semibold text-foreground/90">图像供应商</Label>
              <Select
                value={connection.providerId || "__manual__"}
                onValueChange={value => connection.chooseProvider(value === "__manual__" ? "" : value)}
              >
                <SelectTrigger className="ink-select-trigger" aria-label="图像供应商">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent className="ink-select-content" position="popper">
                  <SelectItem value="__manual__" className="ink-select-item">
                    {connection.imageProviders.length === 0
                      ? "暂无可用图像中转 · 手动填写连接凭据"
                      : "独立配置 / 手动填写连接凭据"}
                  </SelectItem>
                  {connection.imageProviders.map(provider => (
                    <SelectItem key={provider.id} value={provider.id} className="ink-select-item">
                      {provider.name}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              {connection.imageProviders.length === 0 && (
                <p className="font-serif text-[11px] text-muted-foreground">
                  可在「中转站」添加带图像模型的供应商，或在下方手动填写站点与密钥。
                </p>
              )}
            </div>

            {!connection.providerId && (
              <>
                <div className="space-y-1.5">
                  <Label className="font-serif text-xs font-semibold text-foreground/90">站点地址</Label>
                  <Input
                    value={connection.baseUrl}
                    onChange={event => connection.setBaseUrl(event.target.value)}
                    placeholder="https://example.com"
                    className="bg-background/90"
                    list={connection.keySites.length > 0 ? "image-key-sites" : undefined}
                  />
                  {connection.keySites.length > 0 && (
                    <datalist id="image-key-sites">
                      {connection.keySites.map(site => <option key={site} value={site} />)}
                    </datalist>
                  )}
                </div>
                <div className="space-y-1.5">
                  <Label className="font-serif text-xs font-semibold text-foreground/90">API Key</Label>
                  <SecretInput
                    value={connection.apiKey}
                    onChange={connection.setApiKey}
                    onBlur={connection.saveKey}
                    hasStoredSecret={connection.keyStored}
                    placeholder={connection.keyStored ? "已保存密钥；留空则继续使用" : "sk-…"}
                    className="bg-background/90 font-mono"
                  />
                  {connection.keyStored && (
                    <button type="button" onClick={connection.forgetKey}
                      className="text-[11px] text-muted-foreground hover:text-destructive hover:underline">
                      忘记此站点的密钥
                    </button>
                  )}
                </div>
              </>
            )}

            <div className="space-y-1.5 sm:col-span-2">
              <Label className="font-serif text-xs font-semibold text-foreground/90">模型</Label>
              <InkModelCombobox value={connection.model} options={connection.modelOptions} onChange={connection.setModel} />
            </div>
          </div>
        </div>
      </div>
    </section>
  );
}

/** Folded when an earlier visit saved a setup; the user's own toggle wins for the session. */
export function useConnectionPanel(connection: ImageConnection) {
  const [open, setOpen] = useState(() => !connection.configuredBefore);
  return { open, setOpen };
}
