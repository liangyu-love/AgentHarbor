import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import type { RelayView } from "@/components/newapi/use-relay-view";

export function GroupAliasDialog({ view }: { view: RelayView }) {
  const { t } = useTranslation();
  const { aliasGroup, setAliasGroup, aliasDraft, setAliasDraft, saveGroupAlias } = view;

  return (
    <Dialog open={aliasGroup !== null} onOpenChange={open => !open && setAliasGroup(null)}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>{t("newapi.groupAliasTitle")}</DialogTitle>
          <DialogDescription>{t("newapi.groupAliasDescription")}</DialogDescription>
        </DialogHeader>
        <Input autoFocus value={aliasDraft} placeholder={t("newapi.groupAliasPlaceholder")} onChange={event => setAliasDraft(event.target.value)} onKeyDown={event => { if (event.key === "Enter") saveGroupAlias(); }} />
        <DialogFooter>
          <Button type="button" variant="ghost" onClick={() => setAliasGroup(null)}>{t("common.cancel")}</Button>
          <Button type="button" onClick={saveGroupAlias} disabled={!aliasDraft.trim()}>{t("newapi.groupAliasSave")}</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
