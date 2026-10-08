import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import type { CustomInstructionTemplate } from "@/lib/custom-instruction-templates";
import { useTranslation } from "react-i18next";
import { cn } from "@/lib/utils";
import { Eye, ArrowRight, Check, Trash2 } from "lucide-react";

interface TemplateCardProps {
  template: CustomInstructionTemplate;
  isSelected: boolean;
  onSelect: () => void;
  onPreview: () => void;
  disabled?: boolean;
  onDelete?: () => void;
}

export function TemplateCard({ template, isSelected, onSelect, onPreview, disabled, onDelete }: TemplateCardProps) {
  const { t } = useTranslation();

  return (
    <div
      data-instruction-template={template.code}
      className={cn(
        "rounded-xl border p-4 flex flex-col justify-between transition-all duration-200",
        isSelected
          ? "border-primary/50 bg-primary/[0.05] ring-1 ring-primary/20 shadow-sm"
          : "border-border/80 bg-card hover:border-primary/30 hover:shadow-sm"
      )}
    >
      <div>
        <div className="flex items-start justify-between gap-2.5">
          <div className="flex items-center gap-2 min-w-0">
            <h3 className="text-sm font-semibold text-foreground" title={template.title}>
              {template.title}
            </h3>
          </div>
          {typeof template.applyCount === "number" && template.applyCount > 0 && (
            <span className="shrink-0 text-[10px] font-mono text-muted-foreground bg-muted px-2 py-0.5 rounded-full">
              {t("customInstructions.appliedCount", { count: template.applyCount })}
            </span>
          )}
        </div>

        <p className="mt-2.5 text-xs text-muted-foreground line-clamp-2 leading-relaxed min-h-[32px]">
          {template.summary}
        </p>

        <div className="mt-3 flex flex-wrap gap-1.5">
          {template.tags.map((tag) => (
            <Badge key={tag} variant="outline" className="text-[10px] px-1.5 py-0 bg-muted/30">
              {tag}
            </Badge>
          ))}
        </div>
      </div>

      <div className="mt-4 pt-2.5 border-t border-border/50 flex items-center justify-between gap-2">
        <Button variant="outline" size="sm" disabled={disabled} className="h-8 text-xs px-2.5 gap-1" onClick={onPreview}>
          <Eye className="h-3.5 w-3.5 text-muted-foreground" />
          {t("customInstructions.previewAction")}
        </Button>
        <Button
          variant={isSelected ? "default" : "outline"}
          size="sm"
          className="h-7 text-xs px-2.5 gap-1"
          onClick={onSelect}
          disabled={disabled}
        >
          {isSelected ? (
            <>
              <Check className="h-3 w-3" />
              {t("customInstructions.loadedToEditor")}
            </>
          ) : (
            <>
              <ArrowRight className="h-3 w-3" />
              {t("customInstructions.loadToEditor")}
            </>
          )}
        </Button>
        {onDelete && <Button variant="ghost" size="icon" disabled={disabled} className="h-8 w-8 shrink-0 text-muted-foreground hover:text-destructive" aria-label={t("customInstructions.deletePreset", { name: template.title })} onClick={onDelete}><Trash2 className="h-4 w-4" /></Button>}
      </div>
    </div>
  );
}
