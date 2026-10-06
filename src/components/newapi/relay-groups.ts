import { cn } from "@/lib/utils";
import type { GlowColor } from "@/components/ui/glow-card";
import { errorKey } from "@/lib/invoke-error";
import type { ModelProtocol, NewApiProvider } from "@/types/newapi";

export const protocolBadgeKey: Record<ModelProtocol, string> = {
  responses: "newapi.protocolShortResponses",
  chatCompletions: "newapi.protocolShortChat",
  anthropicMessages: "newapi.protocolShortAnthropic",
};

const cardColors: GlowColor[] = ["gold", "blue", "purple", "green", "red", "amber"];
export const providerColor = (id: string): GlowColor => {
  let hash = 0;
  for (const ch of id) hash = (hash * 31 + ch.charCodeAt(0)) >>> 0;
  return cardColors[hash % cardColors.length];
};

export const providerErrorKey = (e: unknown) => errorKey(e, ["newapi", "aggregation", "modelTargets"], "newapi.errors.unexpected");

// DeepSeek peak/off-peak pricing windows in Beijing time: peak 9-12 & 14-18 on
// weekdays, off-peak (half price) everywhere else including whole weekends.
export const deepSeekPeakNow = () => {
  const parts = new Intl.DateTimeFormat("en-US", { timeZone: "Asia/Shanghai", weekday: "short", hour: "numeric", hour12: false }).formatToParts(new Date());
  const weekday = parts.find(p => p.type === "weekday")?.value ?? "";
  const hour = Number(parts.find(p => p.type === "hour")?.value ?? "0") % 24;
  const weekend = weekday === "Sat" || weekday === "Sun";
  return !weekend && ((hour >= 9 && hour < 12) || (hour >= 14 && hour < 18));
};

export type RelayGroup = {
  key: string;
  url: string;
  providers: NewApiProvider[];
};

export function relayGroupKey(baseUrl: string) {
  try {
    const url = new URL(baseUrl);
    url.hash = "";
    url.search = "";
    url.hostname = url.hostname.toLowerCase();
    if ((url.protocol === "https:" && url.port === "443") || (url.protocol === "http:" && url.port === "80")) url.port = "";
    url.pathname = url.pathname.replace(/\/+$/, "") || "/";
    return url.toString();
  } catch {
    return baseUrl.trim().replace(/\/+$/, "").toLowerCase();
  }
}

// Providers sharing one relay base URL render as a single stacked card.
export function groupProviders(providers: NewApiProvider[]): RelayGroup[] {
  const grouped = new Map<string, RelayGroup>();
  for (const provider of providers) {
    const key = relayGroupKey(provider.baseUrl);
    const existing = grouped.get(key);
    if (existing) existing.providers.push(provider);
    else grouped.set(key, { key, url: provider.baseUrl, providers: [provider] });
  }
  return [...grouped.values()].sort((a, b) => a.key.localeCompare(b.key));
}

export const hostOf = (provider: NewApiProvider) => {
  try { return new URL(provider.baseUrl).host; } catch { return provider.baseUrl; }
};

export const iconButton =
  "inline-flex h-7 w-7 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-muted hover:text-foreground disabled:opacity-40 disabled:pointer-events-none";
export const cardButton =
  "flex h-7 w-7 items-center justify-center rounded-md bg-black/[0.05] text-foreground/70 transition-colors hover:bg-black/[0.09] hover:text-foreground disabled:opacity-40 disabled:pointer-events-none dark:bg-white/[0.07] dark:hover:bg-white/[0.12]";
export const chipClass = (selected: boolean) =>
  cn(
    "flex shrink-0 items-center gap-1.5 rounded-lg border bg-card px-3 py-1.5 text-[13px] font-medium shadow-sm transition-all",
    selected
      ? "border-primary/50 bg-primary/[0.06] text-primary ring-1 ring-primary/15"
      : "text-foreground/80 hover:-translate-y-px hover:bg-muted/60",
  );
export const softBadge = (tone: "green" | "amber" | "red" | "gray") => {
  const tones = {
    green: "border-transparent bg-emerald-500/15 text-emerald-700 hover:bg-emerald-500/15 dark:text-emerald-400",
    amber: "border-transparent bg-amber-500/15 text-amber-700 hover:bg-amber-500/15 dark:text-amber-400",
    red: "border-transparent bg-red-500/10 text-red-600 hover:bg-red-500/10 dark:text-red-400",
    gray: "border-input bg-muted/60 text-muted-foreground hover:bg-muted/60",
  };
  return cn("rounded-full px-2 py-0.5 text-[11px] font-medium shadow-none", tones[tone]);
};
