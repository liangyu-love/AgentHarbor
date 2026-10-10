import { useState, type CSSProperties } from "react";
import { useTranslation } from "react-i18next";
import { PenLine, Search } from "lucide-react";
import { cn } from "@/lib/utils";
import { PROVIDER_PRESETS, type ProviderPreset } from "@/lib/provider-presets";
import { relayHue } from "@/components/newapi/relay-groups";

type Category = "all" | "coding" | "aggregator" | "official";
const CATEGORIES: Category[] = ["all", "coding", "aggregator", "official"];
const inCategory = (preset: ProviderPreset, category: Category) =>
  category === "all" || (category === "coding" ? preset.category === "coding" || preset.category === "direct" : preset.category === category);
const host = (url: string) => { try { return new URL(url).host; } catch { return url; } };
const protocolShort = { responses: "Responses", chatCompletions: "Chat", anthropicMessages: "Anthropic" } as const;

// Picking a preset fills the connection; "custom" leaves every field to the user.
export function ProviderPresetGallery({ selected, disabled, onPick, onCustom }: {
  selected: string | null;
  disabled?: boolean;
  onPick: (preset: ProviderPreset) => void;
  onCustom: () => void;
}) {
  const { t } = useTranslation();
  const [category, setCategory] = useState<Category>("all");
  const [query, setQuery] = useState("");
  const needle = query.trim().toLowerCase();
  const presets = PROVIDER_PRESETS.filter(preset => inCategory(preset, category)
    && (!needle || `${preset.name} ${preset.baseUrl} ${preset.description ?? ""}`.toLowerCase().includes(needle)));

  return (
    <section className="space-y-2.5" aria-labelledby="provider-preset-title" data-provider-presets>
      <div className="flex flex-wrap items-center gap-2">
        <h3 id="provider-preset-title" className="font-display text-base font-semibold tracking-tight">{t("newapi.presetGallery.title")}</h3>
        <span className="text-xs text-muted-foreground">{t("newapi.presetGallery.subtitle")}</span>
        <label className="relative ml-auto w-full sm:w-52">
          <Search aria-hidden className="pointer-events-none absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
          <input value={query} onChange={event => setQuery(event.target.value)} placeholder={t("newapi.presetGallery.search")}
            aria-label={t("newapi.presetGallery.search")}
            className="h-8 w-full rounded-xl border border-input bg-background/80 pl-8 pr-2 text-xs outline-none transition-[border-color,box-shadow] focus:border-primary/50 focus:ring-4 focus:ring-primary/10" />
        </label>
      </div>
      <div className="flex flex-wrap gap-1" role="toolbar" aria-label={t("newapi.presetGallery.categories")}>
        {CATEGORIES.map(item => (
          <button key={item} type="button" aria-pressed={category === item} onClick={() => setCategory(item)}
            className={cn("h-7 rounded-full px-3 text-xs font-medium transition-colors",
              category === item ? "bg-foreground text-background" : "bg-muted/70 text-muted-foreground hover:text-foreground")}>
            {t(`newapi.presetGallery.${item}`)}
          </button>
        ))}
      </div>
      <div className="preset-grid grid max-h-[232px] gap-2 overflow-y-auto pr-0.5 sm:grid-cols-2 lg:grid-cols-3">
        <button type="button" disabled={disabled} onClick={onCustom} aria-pressed={selected === null} data-preset-custom
          className={cn("preset-card", selected === null && "preset-card-on")}>
          <span className="preset-mark bg-muted text-muted-foreground"><PenLine className="h-4 w-4" /></span>
          <span className="min-w-0 flex-1">
            <span className="block truncate text-[13px] font-semibold">{t("newapi.customProvider")}</span>
            <span className="block truncate text-[11px] text-muted-foreground">{t("newapi.presetGallery.customDesc")}</span>
          </span>
        </button>
        {presets.map(preset => (
          <button key={preset.name} type="button" disabled={disabled} data-preset={preset.name} aria-pressed={selected === preset.name}
            onClick={() => onPick(preset)} title={preset.description}
            className={cn("preset-card", selected === preset.name && "preset-card-on")}
            style={{ "--relay-hue": relayHue(host(preset.baseUrl)) } as CSSProperties}>
            <span aria-hidden className="preset-mark relay-monogram">{preset.name.charAt(0)}</span>
            <span className="min-w-0 flex-1">
              <span className="flex items-center gap-1.5">
                <span className="truncate text-[13px] font-semibold">{preset.name}</span>
                <span className="shrink-0 rounded bg-muted px-1 font-mono text-[9.5px] text-muted-foreground">{protocolShort[preset.protocol]}</span>
              </span>
              <span className="block truncate font-mono text-[10.5px] text-muted-foreground">{host(preset.baseUrl)}</span>
            </span>
          </button>
        ))}
        {presets.length === 0 && <p className="col-span-full py-4 text-center text-xs text-muted-foreground">{t("newapi.presetGallery.empty")}</p>}
      </div>
    </section>
  );
}
