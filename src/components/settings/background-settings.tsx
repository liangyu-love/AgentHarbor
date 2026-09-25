import React, { useRef } from "react";
import { useTranslation } from "react-i18next";
import { Image as ImageIcon, RotateCcw, Trash2, Upload } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { Slider } from "@/components/ui/slider";
import { Badge } from "@/components/ui/badge";
import { useCustomBackground } from "@/hooks/use-custom-background";

export function BackgroundSettings() {
  const { t } = useTranslation();
  const fileInputRef = useRef<HTMLInputElement>(null);

  const {
    enabled,
    opacity,
    blur,
    overlayDim,
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
          <div className="relative h-32 w-full overflow-hidden rounded-xl border border-border/80 shadow-inner">
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
            {overlayDim > 0 && (
              <div
                className="absolute inset-0 bg-background transition-opacity duration-200"
                style={{ opacity: overlayDim }}
              />
            )}

            <div className="relative z-10 flex h-full flex-col justify-between p-3">
              <div className="flex items-center justify-between">
                <span className="rounded-md bg-background/70 px-2 py-0.5 text-[11px] font-medium text-foreground/90 backdrop-blur-md">
                  {t("settings.bgLivePreview")}
                </span>

                <div className="flex items-center gap-1.5">
                  <Button
                    size="sm"
                    variant="secondary"
                    className="h-7 gap-1 rounded-md px-2.5 text-xs bg-background/80 hover:bg-background backdrop-blur-md shadow-xs"
                    onClick={handleChooseFile}
                  >
                    <Upload className="h-3.5 w-3.5" />
                    {t("settings.bgUploadButton")}
                  </Button>

                  {hasCustomUpload && (
                    <Button
                      size="icon-sm"
                      variant="ghost"
                      className="h-7 w-7 rounded-md bg-background/70 hover:bg-destructive/15 hover:text-destructive backdrop-blur-md"
                      title={t("settings.bgClear")}
                      onClick={() => void clearCustomImage()}
                    >
                      <Trash2 className="h-3.5 w-3.5" />
                    </Button>
                  )}
                </div>
              </div>

              <div className="flex items-center justify-between text-[11px] text-foreground/80">
                <span className="rounded bg-background/60 px-1.5 py-0.5 backdrop-blur-md">
                  {hasCustomUpload
                    ? t("settings.bgCustomUploaded")
                    : t("settings.bgUploadEmpty")}
                </span>
                <span className="rounded font-mono text-[10px] bg-background/60 px-1.5 py-0.5 backdrop-blur-md">
                  {Math.round(opacity * 100)}% · {blur}px · {Math.round(overlayDim * 100)}%
                </span>
              </div>
            </div>
          </div>

          <div className="space-y-5 rounded-xl border border-border/70 bg-muted/25 p-4">
            <div className="space-y-2">
              <div className="flex items-center justify-between text-xs">
                <span className="font-medium text-foreground">
                  {t("settings.bgOpacity")}
                </span>
                <span className="rounded bg-primary/10 px-2 py-0.5 font-mono text-[11px] font-semibold text-primary">
                  {Math.round(opacity * 100)}%
                </span>
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

            <div className="space-y-2">
              <div className="flex items-center justify-between text-xs">
                <span className="font-medium text-foreground">
                  {t("settings.bgClarity")}
                </span>
                <span className="rounded bg-primary/10 px-2 py-0.5 font-mono text-[11px] font-semibold text-primary">
                  {blur === 0
                    ? t("settings.bgClaritySharp")
                    : `${blur}px ${t("settings.bgClarityBlur")}`}
                </span>
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

            <div className="space-y-2">
              <div className="flex items-center justify-between text-xs">
                <span className="font-medium text-foreground">
                  {t("settings.bgOverlayDim")}
                </span>
                <span className="rounded bg-primary/10 px-2 py-0.5 font-mono text-[11px] font-semibold text-primary">
                  {Math.round(overlayDim * 100)}%
                </span>
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
            </div>
          </div>

          <div className="flex items-center justify-end pt-1">
            <Button
              size="sm"
              variant="ghost"
              className="h-7 gap-1 text-xs text-muted-foreground hover:text-foreground"
              onClick={() => void resetToDefault()}
            >
              <RotateCcw className="h-3 w-3" />
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
