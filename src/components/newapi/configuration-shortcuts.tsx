import { useEffect, useRef, useState } from "react";
import { Monitor, Network, Terminal } from "lucide-react";
import { cn } from "@/lib/utils";
import claudeMark from "@/assets/client-marks/claude.svg";
import codexMark from "@/assets/client-marks/openai.svg";
import openCodeMark from "@/assets/client-marks/opencode.svg";
import { HARNESS_MARK_PATH, HARNESS_MARK_VIEWBOX } from "./harness-mark";

export type ShortcutTarget = "codex" | "claudeCode" | "claudeDesktop" | "opencode" | "pi" | "cursor" | "harness" | "aggregation";

// Provider marks are shared with the client cards; Claude's small badge
// distinguishes desktop from CLI without adding a second text row.
export function ClientGlyph({ client, className }: { client: ShortcutTarget; className?: string }) {
  if (client === "aggregation") return <Network className={className} aria-hidden />;
  // pi.dev's mark follows the foreground colour like the official favicon.
  if (client === "pi") return <svg viewBox="0 0 560 560" className={className} fill="currentColor" aria-hidden>
    <path d="M420 280H280V140H0V0H420V280Z" />
    <path d="M560 560H420V280H560V560Z" />
    <path d="M140 560H0V140H140V280H280V420H140V560Z" />
  </svg>;
  // Cursor's cube from its official favicon, following the foreground colour.
  if (client === "cursor") return <svg viewBox="88 64 336 384" className={className} fill="currentColor" aria-hidden>
    <path d="m415.035 156.35-151.503-87.4695c-4.865-2.8094-10.868-2.8094-15.733 0l-151.4969 87.4695c-4.0897 2.362-6.6146 6.729-6.6146 11.459v176.383c0 4.73 2.5249 9.097 6.6146 11.458l151.5039 87.47c4.865 2.809 10.868 2.809 15.733 0l151.504-87.47c4.089-2.361 6.614-6.728 6.614-11.458v-176.383c0-4.73-2.525-9.097-6.614-11.459zm-9.516 18.528-146.255 253.32c-.988 1.707-3.599 1.01-3.599-.967v-165.872c0-3.314-1.771-6.379-4.644-8.044l-143.645-82.932c-1.707-.988-1.01-3.599.968-3.599h292.509c4.154 0 6.75 4.503 4.673 8.101h-.007z" />
  </svg>;
  // Harness's own whale, following the foreground colour as its sidebar does.
  if (client === "harness") return <svg viewBox={HARNESS_MARK_VIEWBOX} className={className} fill="currentColor" aria-hidden>
    <path d={HARNESS_MARK_PATH} />
  </svg>;
  const claude = client === "claudeCode" || client === "claudeDesktop";
  const mark = claude ? claudeMark : client === "opencode" ? openCodeMark : codexMark;
  const Badge = client === "claudeDesktop" ? Monitor : Terminal;
  return <span className={cn("relative inline-flex shrink-0", className)} aria-hidden>
    <img src={mark} alt="" className={cn("h-full w-full object-contain",
      client === "opencode" ? "rounded-[22%]" : !claude && "dark:invert")} />
    {claude && <span className="absolute -bottom-0.5 -right-1 flex h-3 w-3 items-center justify-center rounded-sm bg-background ring-1 ring-border">
      <Badge className="h-2.5 w-2.5" strokeWidth={2} />
    </span>}
  </span>;
}

export const shortcutEntries: Record<ShortcutTarget, { label: string; fold: string; target: string }> = {
  codex: { label: "newapi.routeSourceCodex", fold: "codex-access", target: "newapi-routing-title" },
  claudeCode: { label: "newapi.routeSourceClaudeCode", fold: "client-configs", target: "client-config-claudeCode" },
  claudeDesktop: { label: "newapi.routeSourceClaudeDesktop", fold: "claude-desktop", target: "claude-desktop-title" },
  opencode: { label: "newapi.routeSourceOpenCode", fold: "client-configs", target: "client-config-opencode" },
  pi: { label: "newapi.routeSourcePi", fold: "client-configs", target: "client-config-pi" },
  cursor: { label: "newapi.routeSourceCursor", fold: "client-configs", target: "cursor-access-title" },
  harness: { label: "newapi.routeSourceHarness", fold: "client-configs", target: "client-config-harness" },
  aggregation: { label: "aggregation.title", fold: "aggregation", target: "aggregation-title" },
};

/** Expand a folded section, then focus and scroll to its first control. */
export function useSectionJump() {
  const [active, setActive] = useState<ShortcutTarget | null>(null);
  const frame = useRef(0);
  useEffect(() => () => cancelAnimationFrame(frame.current), []);
  const jump = (client: ShortcutTarget) => {
    const entry = shortcutEntries[client];
    setActive(client);
    cancelAnimationFrame(frame.current);
    window.dispatchEvent(new CustomEvent("agentharbor:expand-section", { detail: entry.fold }));
    if (client !== "aggregation") window.dispatchEvent(new CustomEvent("agentharbor:select-client", { detail: client }));
    // Wait for the folded content to mount. A second click cancels a pending
    // jump so delayed work cannot steal focus from the latest selection.
    let attempts = 0;
    const locate = () => {
      // The console shows the client on its next render; wait for its panel.
      const section = document.getElementById(entry.target);
      const shown = !!section && !section.closest("[hidden]");
      if (!shown && attempts++ < 8) { frame.current = requestAnimationFrame(locate); return; }
      // A client the console has no configuration for still gets its tab.
      const target = shown ? section : client === "aggregation" ? null : document.getElementById(`client-tab-${client}`);
      if (!target) return;
      const focus = target.querySelector<HTMLButtonElement>(entry.target.startsWith("client-config-")
        ? "button[data-config-action]:not(:disabled)" : "button:not(:disabled)") ?? target;
      focus.focus({ preventScroll: true });
      target.scrollIntoView({ block: "start", behavior: window.matchMedia("(prefers-reduced-motion: reduce)").matches ? "instant" : "smooth" });
    };
    frame.current = requestAnimationFrame(locate);
  };
  return { active, jump };
}
