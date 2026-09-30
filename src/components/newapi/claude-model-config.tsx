import { useState } from "react";
import { useTranslation } from "react-i18next";
import { claudeRoles, emptyClaudeRoles } from "@/lib/client-config";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import type { ClaudeModels, ClaudeOneM } from "@/types/clients";
import type { NewApiProvider } from "@/types/newapi";

export function ClaudeModelConfig({ provider, models, selected, mapping, defaultModel, disabled,
  oneM, onSelectedChange, onMappingChange, onOneMChange }: {
  provider?: NewApiProvider;
  models: string[];
  selected: string[];
  mapping: ClaudeModels;
  defaultModel: string;
  disabled: boolean;
  oneM: ClaudeOneM;
  onSelectedChange: (ids: string[]) => void;
  onMappingChange: (mapping: ClaudeModels) => void;
  onOneMChange: (oneM: ClaudeOneM) => void;
}) {
  const { t } = useTranslation();
  const [search, setSearch] = useState("");
  const label = (id: string) => provider?.models.find(model => model.id === id)?.displayName || id;
  const query = search.trim().toLowerCase();
  const filtered = models.filter(id => (id + " " + label(id)).toLowerCase().includes(query));

  return <div className="space-y-4">
    <section aria-labelledby="client-model-list-label" className="space-y-2">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h3 id="client-model-list-label" className="text-sm font-medium">{t("clients.modelList")}
          <span className="ml-2 text-xs font-normal text-muted-foreground">{t("clients.modelsSelected", { count: selected.length })}</span>
        </h3>
        <Button type="button" size="sm" variant="ghost" disabled={disabled || selected.length === models.length}
          onClick={() => onSelectedChange(models)}>{t("clients.selectAllModels")}</Button>
      </div>
      {models.length > 8 && <Input aria-label={t("clients.searchModels")} placeholder={t("clients.searchModels")}
        value={search} onChange={event => setSearch(event.target.value)} disabled={disabled} />}
      <div className="max-h-44 divide-y overflow-y-auto rounded-lg border">
        {filtered.map(id => {
          const checked = selected.includes(id);
          return <label key={id} className="flex min-w-0 items-center gap-3 px-3 py-2 hover:bg-muted/40">
            <input type="checkbox" data-client-model-id={id} aria-label={t("clients.includeModel", { model: label(id) })}
              checked={checked} disabled={disabled || (checked && selected.length === 1)}
              className="h-4 w-4 shrink-0 accent-primary focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring disabled:opacity-50"
              onChange={() => onSelectedChange(checked ? selected.filter(model => model !== id) : [...selected, id])} />
            <span className="min-w-0 flex-1">
              <span className="block break-words text-sm">{label(id)}</span>
              {label(id) !== id && <span className="block break-all text-xs text-muted-foreground">{id}</span>}
            </span>
          </label>;
        })}
        {filtered.length === 0 && <p className="px-3 py-3 text-sm text-muted-foreground">{t("clients.noMatchingModels")}</p>}
      </div>
      <p className="text-xs text-muted-foreground">{t("clients.modelPickerVersion")}</p>
    </section>
    <section aria-labelledby="client-model-mapping-label" className="space-y-2">
      <div className="flex items-center justify-between gap-2">
        <h3 id="client-model-mapping-label" className="text-sm font-medium">{t("clients.modelMappings")}</h3>
        <Button type="button" size="sm" variant="ghost" disabled={disabled || selected.length === 0}
          onClick={() => onMappingChange(emptyClaudeRoles())}>{t("clients.autoMatchModels")}</Button>
      </div>
      <div className="overflow-hidden rounded-lg border">
        <table className="w-full table-fixed text-sm">
          <thead className="bg-muted/30 text-left text-xs text-muted-foreground">
            <tr><th scope="col" className="w-24 px-3 py-2 font-medium">{t("clients.modelRole")}</th>
              <th scope="col" className="px-3 py-2 font-medium">{t("clients.requestModel")}</th>
              <th scope="col" className="w-20 px-3 py-2 text-center font-medium">{t("clients.oneM")}</th></tr>
          </thead>
          <tbody>{claudeRoles.map(role => {
            const automatic = selected.find(id => role !== "subagent" && id.toLowerCase().includes(role)) ?? defaultModel;
            const modelId = mapping[role] || automatic;
            const oneMEnabled = oneM[role];
            return <tr key={role} className="border-t">
              <th scope="row" className="px-3 py-2 text-left font-medium">
                <Label htmlFor={"client-role-" + role}>{role === "subagent" ? t("clients.subagent") : role[0].toUpperCase() + role.slice(1)}</Label>
              </th>
              <td className="p-2">
                <Select disabled={disabled || selected.length === 0} value={mapping[role] ? "model:" + mapping[role] : "auto"}
                  onValueChange={value => onMappingChange({ ...mapping, [role]: value === "auto" ? "" : value.slice(6) })}>
                  <SelectTrigger id={"client-role-" + role} className="w-full"><SelectValue /></SelectTrigger>
                  <SelectContent>
                    <SelectItem value="auto">{t("clients.autoModel", { model: automatic ? label(automatic) : "—" })}</SelectItem>
                    {selected.map(id => <SelectItem key={id} value={"model:" + id}>{label(id)}</SelectItem>)}
                  </SelectContent>
                </Select>
              </td>
              <td className="p-2 text-center">
                <input type="checkbox" aria-label={t("clients.oneMForRole", { role: t("clients." + role) })}
                    checked={oneMEnabled} disabled={disabled || (!modelId && !oneMEnabled)}
                    className="h-4 w-4 accent-primary focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring disabled:opacity-50"
                    onChange={event => onOneMChange({ ...oneM, [role]: event.target.checked })} />
              </td>
            </tr>;
          })}</tbody>
        </table>
      </div>
      <p className="text-xs text-muted-foreground">{t("clients.oneMHint")}</p>
    </section>
  </div>;
}
