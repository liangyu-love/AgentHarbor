import { useTranslation } from "react-i18next";
import { Upload, RotateCcw, Loader2 } from "lucide-react";
import { AppAvatarImage, useAppAvatar } from "@/hooks/use-app-avatar";
import { Button } from "@/components/ui/button";

export function AvatarSettings() {
  const { t } = useTranslation();
  const avatar = useAppAvatar();
  return (
    <div className="px-5 py-4">
      <div className="flex flex-wrap items-center gap-4">
        <AppAvatarImage className="h-16 w-16 shrink-0 rounded-full bg-muted object-cover" />
        <div className="min-w-0 flex-1">
          <h3 className="text-[13px] font-medium">{t("avatar.title")}</h3>
          <p className="mt-1 text-xs text-muted-foreground">{t("avatar.description")}</p>
          <p className="mt-1 text-xs text-muted-foreground">{t("avatar.localOnly")}</p>
        </div>
        <div className="flex flex-wrap gap-2">
          <Button size="sm" onClick={avatar.chooseImage} disabled={avatar.busy} aria-busy={avatar.busy}>
            {avatar.busy ? <Loader2 className="h-4 w-4 animate-spin" /> : <Upload className="h-4 w-4" />}
            {t(avatar.busy ? "avatar.processing" : "avatar.choose")}
          </Button>
          <Button size="sm" variant="outline" onClick={avatar.reset} disabled={!avatar.isCustom || avatar.busy}>
            <RotateCcw className="h-4 w-4" />{t("avatar.reset")}
          </Button>
        </div>
      </div>
      {avatar.errorKey && <p role="alert" className="mt-3 text-sm text-destructive">{t(avatar.errorKey)}</p>}
    </div>
  );
}
