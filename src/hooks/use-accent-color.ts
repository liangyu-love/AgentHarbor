import { useEffect, useState } from "react";

export type AccentPreset = "ocean" | "forest" | "sunset" | "lavender" | "slate";

export const ACCENT_PRESETS: Record<AccentPreset, { label: string; hsl: string; hex: string }> = {
  ocean: { label: "Ocean", hsl: "220 70% 50%", hex: "#2563EB" },
  forest: { label: "Forest", hsl: "152 60% 42%", hex: "#16A34A" },
  sunset: { label: "Sunset", hsl: "25 95% 53%", hex: "#EA580C" },
  lavender: { label: "Lavender", hsl: "262 60% 58%", hex: "#7C3AED" },
  slate: { label: "Slate", hsl: "215 16% 47%", hex: "#64748B" },
};

export function useAccentColor() {
  const [accent, setAccentState] = useState<AccentPreset>(
    () => (localStorage.getItem("accent_color") as AccentPreset) || "lavender",
  );

  useEffect(() => {
    const root = document.documentElement;
    const preset = ACCENT_PRESETS[accent] ?? ACCENT_PRESETS.lavender;
    const hsl = preset.hsl;
    root.style.setProperty("--accent-color", hsl);
    root.style.setProperty("--primary", hsl);
    root.style.setProperty("--ring", hsl);
    root.style.setProperty("--sidebar-primary", hsl);
    root.style.setProperty("--sidebar-ring", hsl);
    localStorage.setItem("accent_color", accent);
  }, [accent]);

  return {
    accent,
    setAccent: setAccentState,
  };
}
