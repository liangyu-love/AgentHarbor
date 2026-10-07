import { Terminal } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { toast } from "@/hooks/use-toast";
import { formatInvokeError } from "@/lib/invoke-error";

export function ThreadResumeButton({ id, className = "h-7 w-7 rounded-lg" }: { id: string; className?: string }) {
  const { t } = useTranslation();
  // The log parser accepts UUID session IDs; do not copy arbitrary shell input.
  if (!/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(id)) return null;
  return <Tooltip>
    <TooltipTrigger asChild>
      <Button variant="ghost" size="icon-sm" className={className} aria-label={t("threads.copyResumeCommand")}
        onClick={async event => {
          event.stopPropagation();
          try {
            await navigator.clipboard.writeText(`claude --resume ${id}`);
            toast({ title: t("threads.resumeCommandCopied"), variant: "success" });
          } catch (error) {
            toast({ title: t("threads.copyFailed"), description: formatInvokeError(error), variant: "destructive" });
          }
        }}>
        <Terminal className="h-3.5 w-3.5" />
      </Button>
    </TooltipTrigger>
    <TooltipContent>{t("threads.copyResumeCommand")}</TooltipContent>
  </Tooltip>;
}
