import { useTranslation } from "react-i18next";
import { FileDown, FileUp, Plus, RadioTower } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { AuroraCard } from "@/components/ui/aurora-card";
import { RainbowButton } from "@/registry/magicui/rainbow-button";
import {
  DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { chipClass } from "@/components/newapi/relay-groups";
import { RelayCard } from "@/components/newapi/relay-card";
import type { ProviderState } from "@/components/newapi/use-provider-state";
import type { RelayView } from "@/components/newapi/use-relay-view";
import type { NewApiProvider } from "@/types/newapi";

// Relay section: header with add / import / export, filter chips and the card grid.
export function RelayList({ store, view, providers, onEdit, onTest, onRemove, onBalance }: {
  store: ProviderState;
  view: RelayView;
  providers: NewApiProvider[];
  onEdit: (provider: NewApiProvider | null) => void;
  onTest: (provider: NewApiProvider) => void;
  onRemove: (provider: NewApiProvider) => void;
  onBalance: (provider: NewApiProvider) => void;
}) {
  const { t } = useTranslation();
  const { state, mutationLocked, exportBusy, importProviders, exportProviders } = store;
  const { filter, setFilter, groups, shown, groupLabel } = view;

  return (
    <section className="space-y-4" aria-labelledby="newapi-providers-title">
      <div className="flex flex-wrap items-center gap-3">
        <h2 id="newapi-providers-title" className="text-lg font-semibold tracking-tight">
          {t("newapi.providersSection")}
        </h2>
        <span className="rounded-full bg-muted px-2 py-0.5 text-xs font-medium tabular-nums text-muted-foreground">
          {groups.length}
        </span>
        <div className="ml-auto flex items-center gap-2">
          <RainbowButton className="h-9 rounded-lg px-3.5" onClick={() => onEdit(null)} disabled={mutationLocked}>
            <Plus className="h-4 w-4" />
            {t("newapi.addProvider")}
          </RainbowButton>
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button variant="outline" size="icon" className="h-9 w-9 rounded-lg" aria-label={t("newapi.importExport")}>
                <FileUp className="h-4 w-4" />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end">
              <DropdownMenuItem onClick={() => void importProviders()} disabled={mutationLocked}>
                <FileUp className="h-4 w-4" />
                {t("newapi.importProviders")}
              </DropdownMenuItem>
              <DropdownMenuItem onClick={() => void exportProviders()} disabled={providers.length === 0 || exportBusy}>
                <FileDown className="h-4 w-4" />
                {t("newapi.exportProviders")}
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        </div>
      </div>

      <div className="flex min-w-0 items-center gap-2 overflow-x-auto pb-0.5">
        <button type="button" onClick={() => setFilter("all")} className={chipClass(filter === "all")}>
          {t("newapi.allProviders")}
        </button>
        {groups.map(group => (
          <button key={group.key} type="button" onClick={() => setFilter(group.key)} className={chipClass(filter === group.key)}>
            <span className="h-2 w-2 shrink-0 rounded-full bg-primary/70" />
            {groupLabel(group)}
            <span className="text-[10px] text-muted-foreground">{group.providers.length}</span>
          </button>
        ))}
      </div>

      {state.isLoading ? (
        <div className="grid items-start gap-4 sm:grid-cols-1 md:grid-cols-2 xl:grid-cols-3">
          {[0, 1, 2].map(i => (
            <AuroraCard key={i} color="blue" className="pointer-events-none opacity-70">
              <div className="space-y-3 p-4">
                <div className="flex items-center justify-between">
                  <Skeleton className="h-5 w-28" />
                  <Skeleton className="h-5 w-9 rounded-full" />
                </div>
                <div className="grid grid-cols-2 gap-2">
                  <Skeleton className="h-16 w-full rounded-xl" />
                  <Skeleton className="h-16 w-full rounded-xl" />
                </div>
                <div className="flex items-center justify-between pt-1">
                  <Skeleton className="h-4 w-24" />
                  <Skeleton className="h-6 w-28 rounded-md" />
                </div>
              </div>
            </AuroraCard>
          ))}
        </div>
      ) : shown.length === 0 ? (
        <div className="rounded-2xl border border-dashed border-border/80 bg-card/50 px-6 py-14 text-center">
          <div className="mx-auto flex h-12 w-12 items-center justify-center rounded-2xl bg-gradient-to-br from-primary/15 to-primary/5 ring-1 ring-inset ring-primary/10">
            <RadioTower className="h-5 w-5 text-primary" />
          </div>
          <p className="mt-4 text-sm text-muted-foreground">{t("newapi.empty")}</p>
          <Button variant="outline" size="sm" className="mt-5" onClick={() => onEdit(null)}>
            <Plus className="h-4 w-4" />
            {t("newapi.addProvider")}
          </Button>
        </div>
      ) : (
        <div className="grid items-start gap-4 sm:grid-cols-1 md:grid-cols-2 xl:grid-cols-3">
          {shown.map(group => (
            <RelayCard key={group.key} group={group} store={store} view={view} onEdit={onEdit} onTest={onTest} onRemove={onRemove} onBalance={onBalance} />
          ))}          </div>
      )}
    </section>
  );
}
