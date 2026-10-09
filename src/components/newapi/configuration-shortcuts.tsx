import { useEffect, useRef, useState } from "react";
import { Monitor, Network, Terminal } from "lucide-react";
import { cn } from "@/lib/utils";
import claudeMark from "@/assets/client-marks/claude.svg";
import codexMark from "@/assets/client-marks/openai.svg";
import openCodeMark from "@/assets/client-marks/opencode.svg";

export type ShortcutTarget = "codex" | "claudeCode" | "claudeDesktop" | "opencode" | "pi" | "aggregation";

// Provider marks are shared with the client cards; Claude's small badge
// distinguishes desktop from CLI without adding a second text row.
export function ClientGlyph({ client, className }: { client: ShortcutTarget; className?: string }) {
  if (client === "aggregation") return <Network className={className} aria-hidden />;
  if (client === "pi") return <svg viewBox="0 0 800 800" className={className} fill="currentColor" aria-hidden>
    <path fillRule="evenodd" d="M165.29 165.29H517.36V400H400V517.36H282.65V634.72H165.29ZM282.65 282.65V400H400V282.65Z" />
    <path d="M517.36 400H634.72V634.72H517.36Z" />
  </svg>;
  const claude = client === "claudeCode" || client === "claudeDesktop";
  const mark = claude ? claudeMark : client === "opencode" ? openCodeMark : codexMark;
  const Badge = client === "claudeDesktop" ? Monitor : Terminal;
  return <span className={cn("relative inline-flex shrink-0", className)} aria-hidden>
    <img src={mark} alt="" className={cn("h-full w-full object-contain", !claude && "dark:invert")} />
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
    // Wait for the folded content to mount. A second click cancels a pending
    // jump so delayed work cannot steal focus from the latest selection.
    let attempts = 0;
    const locate = () => {
      const section = document.getElementById(entry.target);
      if (!section && attempts++ < 8) { frame.current = requestAnimationFrame(locate); return; }
      const target = section ?? document.getElementById("client-configs-title");
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
