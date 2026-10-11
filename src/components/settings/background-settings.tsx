import React, { useRef } from "react";
import { useTranslation } from "react-i18next";
import {
  Contrast,
  Eye,
  Image as ImageIcon,
  RotateCcw,
  Sparkles,
  Trash2,
  Upload,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { Slider } from "@/components/ui/slider";
import { Badge } from "@/components/ui/badge";
import { useCustomBackground } from "@/hooks/use-custom-background";
import { cn } from "@/lib/utils";

export function BackgroundSettings() {
  const { t } = useTranslation();
  const fileInputRef = useRef<HTMLInputElement>(null);

  const {
    enabled,
    opacity,
    blur,
    overlayDim,
    effectiveOverlayDim,
    autoDimmed,
    hasCustomUpload,
    activeBackgroundStyle,
    setEnabled,
    setOpacity,
    setBlur,
    setOverlayDim,
    uploadCustomImage,
    clearCustomImage,
    resetToDefault,
  } = useCustomBackground();

  const handleChooseFile = () => {
    fileInputRef.current?.click();
  };

  const handleFileChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    e.target.value = "";
    if (file) {
      void uploadCustomImage(file);
    }
  };

  return (
    <div className="space-y-4 px-5 py-4">
      <div className="flex items-center justify-between">
        <div className="space-y-0.5 pr-4">
          <div className="flex items-center gap-2">
            <ImageIcon className="h-4 w-4 text-primary" />
            <span className="text-[13px] font-medium text-foreground">
              {t("settings.backgroundTitle")}
            </span>
            {enabled && hasCustomUpload && (
              <Badge variant="secondary" className="text-[11px] font-normal">
                {t("settings.bgTypeCustom")}
              </Badge>
            )}
          </div>
          <p className="text-xs text-muted-foreground">
            {t("settings.backgroundDesc")}
          </p>
        </div>

        <Switch checked={enabled} onCheckedChange={setEnabled} />
      </div>

      {enabled && (
        <div className="space-y-5 pt-2 animate-in fade-in slide-in-from-top-1 duration-200">
          {/* Live Preview Card */}
          <div className="relative h-36 w-full overflow-hidden rounded-2xl border border-border/80 shadow-md"
            style={{ backgroundColor: "hsl(var(--background))" }}>
            {activeBackgroundStyle && (
              <div
                className="absolute inset-0 transition-all duration-200"
                style={{
                  ...activeBackgroundStyle,
                  backgroundSize: "cover",
                  backgroundPosition: "center",
                  backgroundRepeat: "no-repeat",
                  opacity,
                  filter: blur > 0 ? `blur(${blur}px)` : undefined,
                }}
              />
            )}
            <div
              className="absolute inset-0 transition-opacity duration-200"
              style={{ backgroundColor: "hsl(var(--background))", opacity: effectiveOverlayDim }}
            />

            <div className="relative z-10 flex h-full flex-col justify-between p-3.5">
              <div className="flex items-center justify-between">
                <span className="rounded-md bg-background/80 px-2.5 py-1 text-[11px] font-medium text-foreground/90 backdrop-blur-md shadow-xs ring-1 ring-border/50">
                  {t("settings.bgLivePreview")}
                </span>

                <div className="flex items-center gap-1.5">
                  <Button
                    size="sm"
                    variant="secondary"
                    className="h-7 gap-1.5 rounded-lg px-2.5 text-xs bg-background/85 hover:bg-background backdrop-blur-md shadow-xs ring-1 ring-border/60"
                    onClick={handleChooseFile}
                  >
                    <Upload className="h-3.5 w-3.5" />
                    {t("settings.bgUploadButton")}
                  </Button>

                  {hasCustomUpload && (
                    <Button
                      size="icon-sm"
                      variant="ghost"
                      className="h-7 w-7 rounded-lg bg-background/80 hover:bg-destructive/15 hover:text-destructive backdrop-blur-md ring-1 ring-border/50"
                      title={t("settings.bgClear")}
                      aria-label={t("settings.bgClear")}
                      onClick={() => void clearCustomImage()}
                    >
                      <Trash2 className="h-3.5 w-3.5" />
                    </Button>
                  )}
                </div>
              </div>

              <div className="flex items-center justify-between text-[11px] text-foreground/90">
                <span className="rounded-md bg-background/75 px-2 py-0.5 backdrop-blur-md ring-1 ring-border/40">
                  {hasCustomUpload
                    ? t("settings.bgCustomUploaded")
                    : t("settings.bgUploadEmpty")}
                </span>
                <span className="rounded-md font-mono text-[10px] bg-background/75 px-2 py-0.5 backdrop-blur-md ring-1 ring-border/40 font-medium">
                  {Math.round(opacity * 100)}% · {blur}px · {Math.round(effectiveOverlayDim * 100)}%
                </span>
              </div>
            </div>
          </div>

          {/* Sliders Container Card - High contrast frosted glass */}
          <div className="space-y-6 rounded-2xl border border-border/80 bg-card/80 p-4 sm:p-5 backdrop-blur-md shadow-xs">
            {/* Opacity Slider */}
            <div className="space-y-2.5">
              <div className="flex items-center justify-between text-xs">
                <div className="flex items-center gap-2">
                  <Eye className="h-4 w-4 text-primary" />
                  <span className="font-semibold text-foreground">
                    {t("settings.bgOpacity")}
                  </span>
                </div>
                <div className="flex items-center gap-2">
                  <div className="flex items-center gap-1">
                    {[30, 60, 85, 100].map((preset) => (
                      <button
                        key={preset}
                        type="button"
                        onClick={() => setOpacity(preset / 100)}
                        className={cn(
                          "rounded px-1.5 py-0.5 text-[10px] font-mono transition-colors",
                          Math.round(opacity * 100) === preset
                            ? "bg-primary text-primary-foreground font-semibold"
                            : "bg-muted/70 text-muted-foreground hover:bg-muted hover:text-foreground",
                        )}
                      >
                        {preset}%
                      </button>
                    ))}
                  </div>
                  <span className="min-w-12 text-right rounded-md bg-primary/10 px-2 py-0.5 font-mono text-[11px] font-semibold text-primary ring-1 ring-primary/20">
                    {Math.round(opacity * 100)}%
                  </span>
                </div>
              </div>
              <Slider
                min={0}
                max={100}
                step={1}
                value={Math.round(opacity * 100)}
                onChange={(val) => setOpacity(val / 100)}
              />
              <p className="text-[11px] text-muted-foreground">
                {t("settings.bgOpacityTip")}
              </p>
            </div>

            {/* Clarity / Blur Slider */}
            <div className="space-y-2.5">
              <div className="flex items-center justify-between text-xs">
                <div className="flex items-center gap-2">
                  <Sparkles className="h-4 w-4 text-primary" />
                  <span className="font-semibold text-foreground">
                    {t("settings.bgClarity")}
                  </span>
                </div>
                <div className="flex items-center gap-2">
                  <div className="flex items-center gap-1">
                    {[
                      { val: 0, label: "0px" },
                      { val: 4, label: "4px" },
                      { val: 8, label: "8px" },
                      { val: 16, label: "16px" },
                    ].map(({ val, label }) => (
                      <button
                        key={val}
                        type="button"
                        onClick={() => setBlur(val)}
                        className={cn(
                          "rounded px-1.5 py-0.5 text-[10px] font-mono transition-colors",
                          blur === val
                            ? "bg-primary text-primary-foreground font-semibold"
                            : "bg-muted/70 text-muted-foreground hover:bg-muted hover:text-foreground",
                        )}
                      >
                        {label}
                      </button>
                    ))}
                  </div>
                  <span className="min-w-16 text-right rounded-md bg-primary/10 px-2 py-0.5 font-mono text-[11px] font-semibold text-primary ring-1 ring-primary/20">
                    {blur === 0
                      ? t("settings.bgClaritySharp")
                      : `${blur}px ${t("settings.bgClarityBlur")}`}
                  </span>
                </div>
              </div>
              <Slider
                min={0}
                max={32}
                step={1}
                value={blur}
                onChange={setBlur}
              />
              <p className="text-[11px] text-muted-foreground">
                {t("settings.bgClarityTip")}
              </p>
            </div>

            {/* Overlay Dim Slider */}
            <div className="space-y-2.5">
              <div className="flex items-center justify-between text-xs">
                <div className="flex items-center gap-2">
                  <Contrast className="h-4 w-4 text-primary" />
                  <span className="font-semibold text-foreground">
                    {t("settings.bgOverlayDim")}
                  </span>
                </div>
                <div className="flex items-center gap-2">
                  <div className="flex items-center gap-1">
                    {[0, 10, 25, 45].map((preset) => (
                      <button
                        key={preset}
                        type="button"
                        onClick={() => setOverlayDim(preset / 100)}
                        className={cn(
                          "rounded px-1.5 py-0.5 text-[10px] font-mono transition-colors",
                          Math.round(overlayDim * 100) === preset
                            ? "bg-primary text-primary-foreground font-semibold"
                            : "bg-muted/70 text-muted-foreground hover:bg-muted hover:text-foreground",
                        )}
                      >
                        {preset}%
                      </button>
                    ))}
                  </div>
                  <span className="min-w-12 text-right rounded-md bg-primary/10 px-2 py-0.5 font-mono text-[11px] font-semibold text-primary ring-1 ring-primary/20">
                    {Math.round(overlayDim * 100)}%
                  </span>
                </div>
              </div>
              <Slider
                min={0}
                max={80}
                step={1}
                value={Math.round(overlayDim * 100)}
                onChange={(val) => setOverlayDim(val / 100)}
              />
              <p className="text-[11px] text-muted-foreground">
                {t("settings.bgOverlayDimTip")}
              </p>
              {autoDimmed && (
                <p data-wallpaper-auto-dim className="flex items-center gap-1.5 text-[11px] font-medium text-primary">
                  <Contrast aria-hidden className="h-3.5 w-3.5 shrink-0" />
                  {t("settings.bgAutoDimmed", { value: Math.round(effectiveOverlayDim * 100) })}
                </p>
              )}
            </div>
          </div>

          <div className="flex items-center justify-end pt-1">
            <Button
              size="sm"
              variant="ghost"
              className="h-7 gap-1.5 text-xs text-muted-foreground hover:text-foreground rounded-lg"
              onClick={() => void resetToDefault()}
            >
              <RotateCcw className="h-3.5 w-3.5" />
              {t("settings.bgReset")}
            </Button>
          </div>
        </div>
      )}

      <input
        ref={fileInputRef}
        type="file"
        accept="image/png,image/jpeg,image/webp,image/avif"
        hidden
        onChange={handleFileChange}
      />
    </div>
  );
}
