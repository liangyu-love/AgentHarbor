import { useTranslation } from "react-i18next";
import { Badge } from "@/components/ui/badge";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import type {
  ClaudeDesktopProviderCandidate,
  ClaudeDesktopRouteChoice,
  ClaudeDesktopRouteRoleView,
} from "@/types/claude-desktop";

const EMPTY = "__unconfigured__";

export function desktopRouteSupports1m(model: { protocol: string; supports1m: boolean } | undefined) {
  return !!model?.supports1m && model.protocol === "anthropicMessages";
}

export function desktopProtocolLabel(protocol: string) {
  return protocol === "anthropicMessages" ? "Messages"
    : protocol === "responses" ? "Responses"
      : protocol === "chatCompletions" ? "Chat" : protocol;
}

export function ClaudeDesktopRouteMapping({ roles, providers, choices, onChange }: {
  roles: ClaudeDesktopRouteRoleView[];
  providers: ClaudeDesktopProviderCandidate[];
  choices: ClaudeDesktopRouteChoice[];
  onChange: (choices: ClaudeDesktopRouteChoice[]) => void;
}) {
  const { t } = useTranslation();
  const update = (role: ClaudeDesktopRouteRoleView, next: ClaudeDesktopRouteChoice | null) => {
    const updated = [...choices.filter(choice => choice.role !== role.role), ...(next ? [next] : [])];
    onChange(updated.sort((a, b) => roles.findIndex(entry => entry.role === a.role) - roles.findIndex(entry => entry.role === b.role)));
  };

  return (
    <div data-desktop-route-mapping className="space-y-2">
      {roles.map(role => {
        const choice = choices.find(entry => entry.role === role.role);
        const provider = providers.find(entry => entry.providerId === choice?.providerId);
        const model = provider?.models.find(entry => entry.modelId === choice?.modelId);
        const available = !!provider?.eligible && !!model?.selectable;
        const id = `desktop-route-${role.role}`;
        return (
          <div key={role.role} data-desktop-route-role={role.role} className="min-w-0 space-y-2 rounded-lg border px-3 py-3">
            <div className="flex min-w-0 flex-wrap items-center gap-2">
              <p className="text-sm font-medium capitalize">{role.role}</p>
              <span className="ml-auto text-[11px] text-muted-foreground">
                {!choice?.modelId ? t("claudeDesktop.routing.unconfigured") : model ? desktopProtocolLabel(model.protocol) : "—"}
              </span>
            </div>
            <div className="grid min-w-0 gap-2 sm:grid-cols-2">
              <div className="min-w-0 space-y-1">
                <Label htmlFor={`${id}-provider`} className="sr-only">{t("claudeDesktop.routing.providerForRole", { role: role.role })}</Label>
                <Select value={choice?.providerId || EMPTY} onValueChange={providerId => {
                  update(role, providerId === EMPTY ? null : { ...role, providerId, modelId: "", labelOverride: null, oneMMode: "off" });
                }}>
                  <SelectTrigger id={`${id}-provider`} className="h-9 min-w-0 text-xs">
                    <SelectValue placeholder={t("claudeDesktop.routing.chooseProvider")} />
                  </SelectTrigger>
                  <SelectContent className="max-w-[calc(100vw-2rem)]">
                    <SelectItem value={EMPTY}>{t("claudeDesktop.routing.unconfigured")}</SelectItem>
                    {choice?.providerId && !provider && <SelectItem value={choice.providerId} disabled>{choice.providerId}</SelectItem>}
                    {providers.map(entry => (
                      <SelectItem key={entry.providerId} value={entry.providerId} disabled={!entry.eligible}>
                        {entry.name}{!entry.enabled ? ` · ${t("claudeDesktop.routing.providerDisabled")}` : !entry.hasInferenceKey ? ` · ${t("claudeDesktop.routing.noKey")}` : ""}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
              <div className="min-w-0 space-y-1">
                <Label htmlFor={`${id}-model`} className="sr-only">{t("claudeDesktop.routing.modelForRole", { role: role.role })}</Label>
                <Select value={choice?.modelId || EMPTY} disabled={!provider?.eligible} onValueChange={modelId => {
                  if (choice) update(role, { ...choice, modelId: modelId === EMPTY ? "" : modelId, labelOverride: null, oneMMode: "off" });
                }}>
                  <SelectTrigger id={`${id}-model`} className="h-9 min-w-0 text-xs">
                    <SelectValue placeholder={t("claudeDesktop.routing.chooseModel")} />
                  </SelectTrigger>
                  <SelectContent className="max-w-[calc(100vw-2rem)]">
                    <SelectItem value={EMPTY}>{t("claudeDesktop.routing.chooseModel")}</SelectItem>
                    {choice?.modelId && !model && <SelectItem value={choice.modelId} disabled>{choice.modelId}</SelectItem>}
                    {(provider?.models ?? []).map(entry => (
                      <SelectItem key={entry.modelId} value={entry.modelId} disabled={!entry.selectable} className="break-all text-xs">
                        {entry.modelId}{!entry.selectable ? ` · ${t("claudeDesktop.routing.unavailable")}` : ""}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
            </div>
            {!!choice?.modelId && (
              <div className="flex min-w-0 flex-wrap items-center gap-2">
                <Input
                  value={choice.labelOverride ?? ""}
                  maxLength={64}
                  aria-label={t("claudeDesktop.routing.labelForRole", { role: role.role })}
                  placeholder={`${provider?.name ?? choice.providerId} · ${model?.label ?? choice.modelId}`}
                  className="h-8 min-w-0 flex-1 text-xs"
                  onChange={event => update(role, { ...choice, labelOverride: event.target.value || null })}
                />
                <div className="flex items-center gap-2">
                  <Checkbox id={`${id}-onem`} checked={choice.oneMMode === "auto"}
                    disabled={!desktopRouteSupports1m(model) && choice.oneMMode !== "auto"}
                    onCheckedChange={checked => update(role, { ...choice, oneMMode: checked === true ? "auto" : "off" })} />
                  <label htmlFor={`${id}-onem`} className="cursor-pointer text-xs">{t("claudeDesktop.dialog.oneMOptions")}</label>
                </div>
                {!available && <Badge variant="destructive" className="text-[10px]">{t("claudeDesktop.routing.unavailable")}</Badge>}
                {available && choice.oneMMode === "auto" && !desktopRouteSupports1m(model) && (
                  <p role="status" className="w-full text-[11px] text-destructive">{t("claudeDesktop.routing.oneMUnavailable")}</p>
                )}
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
}
