import { useTranslation } from "react-i18next";
import { Globe } from "lucide-react";
import { api } from "@/lib/api";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import type { NewApiProvider } from "@/types/newapi";
import type { ProviderState } from "./use-provider-state";

const WEB_SEARCH_SECTION_ID = "client-web-search";
const WEB_SEARCH_PROVIDER_ID = "client-web-search-provider";
/** The model choice that stands for a card's REST Search API. */
const SEARCH_API = "__search_api";

/** Client panels link here: bring the shared setting into view and focus its search service. */
export function revealWebSearchSettings() {
  const section = document.getElementById(WEB_SEARCH_SECTION_ID);
  if (!section) return;
  // A disabled select cannot take focus; the section still can.
  const select = document.getElementById(WEB_SEARCH_PROVIDER_ID) as HTMLButtonElement | null;
  (select && !select.disabled ? select : section).focus({ preventScroll: true });
  section.scrollIntoView({ block: "center", behavior: window.matchMedia("(prefers-reduced-motion: reduce)").matches ? "instant" : "smooth" });
}

// The account and model that run WebSearch for every client routed through
// AgentHarbor. It sits under the client console rather than in one client's
// panel, and stays editable in Codex's native mode because the others still route.
export function WebSearchSettings({ store, providers }: { store: ProviderState; providers: NewApiProvider[] }) {
  const { t } = useTranslation();
  const { state, mutationLocked, run, applyChange } = store;
  const router = state.data?.router;
  const providerId = router?.webSearchProviderId ?? null;
  const modelId = router?.webSearchModelId ?? null;
  const searchProviders = providers.filter(provider => provider.webSearchCapable);
  const selected = searchProviders.find(provider => provider.id === providerId);
  const unavailable = providerId !== null && !selected;
  const models = selected?.webSearchModels ?? [];
  const searchApi = selected?.webSearchApi === "deepSeek";
  const disabled = !state.isSuccess || mutationLocked || (searchProviders.length === 0 && !unavailable);

  const save = (nextProvider: string | null, nextModel: string | null) =>
    void run(
      "web-search-provider",
      async () => applyChange(await api.routing.setWebSearchProvider(nextProvider, nextModel)),
      t("newapi.webSearchSaveFailed"),
    );

  return (
    <section id={WEB_SEARCH_SECTION_ID} tabIndex={-1} aria-labelledby="client-web-search-title"
      className="flex min-w-0 scroll-mt-4 flex-wrap items-end gap-x-6 gap-y-3 focus:outline-none" data-web-search-settings>
      <div className="min-w-0 flex-1 basis-72 self-center">
        <div className="flex flex-wrap items-baseline gap-x-2.5 gap-y-1">
          <h3 id="client-web-search-title" className="flex items-center gap-1.5 text-[13px] font-semibold text-foreground">
            <Globe aria-hidden className="h-3.5 w-3.5 text-primary" />
            {t("newapi.webSearchPanel")}
          </h3>
          <span className="rounded-full bg-primary/10 px-2 py-0.5 text-[10px] font-medium text-primary">{t("newapi.webSearchShared")}</span>
        </div>
        <p className="mt-1 text-[11px] leading-relaxed text-muted-foreground">
          {t(searchProviders.length === 0 && !unavailable ? "newapi.webSearchNone" : "newapi.webSearchHint")}
        </p>
      </div>
      <div className="flex w-full min-w-0 flex-wrap gap-3 sm:w-auto">
        <div className="min-w-0 flex-1 space-y-1.5 sm:w-56 sm:flex-none">
          <Label htmlFor={WEB_SEARCH_PROVIDER_ID} className="text-xs text-muted-foreground">{t("newapi.webSearchService")}</Label>
          <Select value={providerId ?? "__automatic"} disabled={disabled}
            onValueChange={value => {
              const provider = searchProviders.find(candidate => candidate.id === value);
              const offered = provider?.webSearchModels ?? [];
              // A DeepSeek card searches through its Search API unless a model is picked.
              save(value === "__automatic" ? null : value,
                provider?.webSearchApi === "deepSeek" ? null
                  : offered.includes(provider?.model ?? "") ? provider!.model : offered[0] ?? null);
            }}>
            <SelectTrigger id={WEB_SEARCH_PROVIDER_ID} className="min-w-0 bg-card [&>span]:truncate"><SelectValue /></SelectTrigger>
            <SelectContent>
              <SelectItem value="__automatic">{t("newapi.webSearchAutomatic")}</SelectItem>
              {unavailable && <SelectItem value={providerId} disabled>{t("newapi.webSearchUnavailable")}</SelectItem>}
              {searchProviders.map(provider => <SelectItem key={provider.id} value={provider.id}>{provider.name}</SelectItem>)}
            </SelectContent>
          </Select>
        </div>
        {selected && (models.length > 0 || searchApi) && (
          <div className="min-w-0 flex-1 space-y-1.5 sm:w-56 sm:flex-none">
            <Label htmlFor="client-web-search-model" className="text-xs text-muted-foreground">{t("newapi.webSearchModel")}</Label>
            <Select value={modelId ?? (searchApi ? SEARCH_API : "__missing")} disabled={disabled}
              onValueChange={value => save(selected.id, value === SEARCH_API ? null : value)}>
              <SelectTrigger id="client-web-search-model" className="min-w-0 bg-card [&>span]:truncate"><SelectValue /></SelectTrigger>
              <SelectContent>
                {searchApi && <SelectItem value={SEARCH_API}>{t("newapi.webSearchDeepSeekApi")}</SelectItem>}
                {(modelId ? !models.includes(modelId) : !searchApi) && (
                  <SelectItem value={modelId ?? "__missing"} disabled>{t("newapi.webSearchUnavailable")}</SelectItem>
                )}
                {models.map(model => <SelectItem key={model} value={model}>{model}</SelectItem>)}
              </SelectContent>
            </Select>
          </div>
        )}
      </div>
    </section>
  );
}
