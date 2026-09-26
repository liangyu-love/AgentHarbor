import * as React from "react";
import { cn } from "@/lib/utils";

export interface SliderProps
  extends Omit<React.InputHTMLAttributes<HTMLInputElement>, "onChange" | "value"> {
  value: number;
  min?: number;
  max?: number;
  step?: number;
  onChange: (val: number) => void;
  className?: string;
  trackClassName?: string;
}

export function Slider({
  value,
  min = 0,
  max = 100,
  step = 1,
  onChange,
  className,
  trackClassName,
  disabled,
  ...props
}: SliderProps) {
  const percentage = Math.max(0, Math.min(100, ((value - min) / Math.max(1, max - min)) * 100));

  return (
    <div className={cn("relative flex w-full items-center select-none touch-none py-1.5", className)}>
      {/* Decorative underlay track with border & subtle shadow for crystal-clear visibility on any wallpaper */}
      <div className="pointer-events-none absolute inset-x-0 h-2.5 rounded-full bg-zinc-200/90 dark:bg-zinc-800/90 ring-1 ring-border/80 dark:ring-white/15 shadow-inner overflow-hidden">
        <div
          className="h-full rounded-full bg-gradient-to-r from-primary to-primary/90 transition-all duration-75 shadow-[0_0_8px_hsl(var(--primary)/0.45)]"
          style={{ width: `${percentage}%` }}
        />
      </div>

      <input
        type="range"
        min={min}
        max={max}
        step={step}
        value={value}
        disabled={disabled}
        onChange={(e) => onChange(Number(e.target.value))}
        className={cn(
          "relative z-10 h-2.5 w-full cursor-pointer appearance-none bg-transparent transition-all focus:outline-none",
          "disabled:cursor-not-allowed disabled:opacity-50",
          // Webkit thumb styling
          "[&::-webkit-slider-thumb]:h-5 [&::-webkit-slider-thumb]:w-5 [&::-webkit-slider-thumb]:appearance-none [&::-webkit-slider-thumb]:rounded-full",
          "[&::-webkit-slider-thumb]:bg-background [&::-webkit-slider-thumb]:border-[2.5px] [&::-webkit-slider-thumb]:border-primary",
          "[&::-webkit-slider-thumb]:shadow-[0_2px_8px_rgba(0,0,0,0.25)] [&::-webkit-slider-thumb]:ring-2 [&::-webkit-slider-thumb]:ring-primary/20",
          "[&::-webkit-slider-thumb]:transition-all hover:[&::-webkit-slider-thumb]:scale-115 hover:[&::-webkit-slider-thumb]:ring-primary/40 active:[&::-webkit-slider-thumb]:scale-95",
          // Firefox thumb styling
          "[&::-moz-range-thumb]:h-5 [&::-moz-range-thumb]:w-5 [&::-moz-range-thumb]:appearance-none [&::-moz-range-thumb]:rounded-full",
          "[&::-moz-range-thumb]:bg-background [&::-moz-range-thumb]:border-[2.5px] [&::-moz-range-thumb]:border-primary",
          "[&::-moz-range-thumb]:shadow-[0_2px_8px_rgba(0,0,0,0.25)] [&::-moz-range-thumb]:ring-2 [&::-moz-range-thumb]:ring-primary/20",
          "[&::-moz-range-thumb]:transition-all hover:[&::-moz-range-thumb]:scale-115 hover:[&::-moz-range-thumb]:ring-primary/40 active:[&::-moz-range-thumb]:scale-95",
          trackClassName,
        )}
        {...props}
      />
    </div>
  );
}
