import { useTranslation } from "react-i18next";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { BentoCard } from "@/components/ui/bento-card";
import { RainbowButton } from "@/registry/magicui/rainbow-button";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import {
  Pagination,
  PaginationContent,
  PaginationItem,
  PaginationLink,
  PaginationNext,
  PaginationPrevious,
  PaginationEllipsis,
} from "@/components/ui/pagination";
import { Plus, Search, Server } from "lucide-react";
import { TooltipProvider } from "@/components/ui/tooltip";
import { formatInvokeError } from "@/lib/invoke-error";
import { McpEditorDialog } from "@/components/mcp/mcp-editor-dialog";
import { McpHeader } from "@/components/mcp/mcp-header";
import { McpServerCard } from "@/components/mcp/mcp-server-card";
import { McpStats } from "@/components/mcp/mcp-stats";
import { McpToolbar } from "@/components/mcp/mcp-toolbar";
import { useMcpServers } from "@/components/mcp/use-mcp-servers";

export function McpPage() {
  const { t } = useTranslation();
  const mcp = useMcpServers();
  const {
    refetch, error, isPending, servers, filteredServers, pagedServers, totalPages, paginationRange,
    currentPage, setCurrentPage, setSearchQuery, setTransportFilter, editing, setEditing, removing, setRemoving, removeMutation,
  } = mcp;

  if (error) return <div role="alert" className="space-y-3 p-4"><p>{t("baseline.readFailed")}</p><p className="text-sm text-muted-foreground">{formatInvokeError(error)}</p><Button onClick={() => void refetch()}>{t("common.refresh")}</Button></div>;
  if (isPending) return <p role="status" className="p-4 text-muted-foreground">{t("baseline.loading")}</p>;

  return (
    <TooltipProvider delayDuration={150}>
      <div className="space-y-6">
        {/* Page header */}
        <McpHeader mcp={mcp} />

        {/* Stats & Config path banner */}
        <McpStats mcp={mcp} />

        {/* Search & Filter Toolbar */}
        <McpToolbar mcp={mcp} />

        {servers.length === 0 ? (
          <BentoCard>
            <div className="flex h-56 flex-col items-center justify-center text-center p-6">
              <div className="flex h-12 w-12 items-center justify-center rounded-2xl bg-gradient-to-br from-primary/20 to-primary/5 ring-1 ring-inset ring-primary/15">
                <Server className="h-6 w-6 text-primary" />
              </div>
              <p className="mt-3.5 text-base font-semibold text-foreground">{t("mcp.empty")}</p>
              <p className="mt-1 text-xs text-muted-foreground max-w-sm">
                直接添加本地进程或远程 MCP 端点，Codex 即可在对话中随时调用工具能力。
              </p>
              <RainbowButton size="sm" className="mt-4 rounded-lg" onClick={() => setEditing("new")}>
                <Plus className="h-3.5 w-3.5" />
                {t("mcp.addServer")}
              </RainbowButton>
            </div>
          </BentoCard>
        ) : filteredServers.length === 0 ? (
          <BentoCard>
            <div className="flex h-44 flex-col items-center justify-center text-center p-6">
              <Search className="h-8 w-8 text-muted-foreground/60" />
              <p className="mt-2.5 text-sm font-medium text-muted-foreground">{t("mcp.noFilterResults")}</p>
              <Button
                variant="outline"
                size="sm"
                className="mt-3"
                onClick={() => {
                  setSearchQuery("");
                  setTransportFilter("all");
                }}
              >
                {t("mcp.clearSearch")}
              </Button>
            </div>
          </BentoCard>
        ) : (
          <>
            <div className="grid gap-4 sm:grid-cols-1 md:grid-cols-2 xl:grid-cols-3">
              {pagedServers.map((server) => <McpServerCard key={server.name} server={server} mcp={mcp} />)}
            </div>

            {/* Pagination if more than 12 servers */}
            {totalPages > 1 && (
              <Pagination className="mt-2">
                <PaginationContent>
                  <PaginationItem>
                    <PaginationPrevious
                      onClick={() => setCurrentPage((p) => Math.max(1, p - 1))}
                      className={cn(currentPage <= 1 && "pointer-events-none opacity-50")}
                    />
                  </PaginationItem>
                  {paginationRange.map((page, i) =>
                    page === "ellipsis" ? (
                      <PaginationItem key={`e${i}`}>
                        <PaginationEllipsis />
                      </PaginationItem>
                    ) : (
                      <PaginationItem key={page}>
                        <PaginationLink
                          isActive={page === currentPage}
                          onClick={() => setCurrentPage(page as number)}
                        >
                          {page}
                        </PaginationLink>
                      </PaginationItem>
                    )
                  )}
                  <PaginationItem>
                    <PaginationNext
                      onClick={() => setCurrentPage((p) => Math.min(totalPages, p + 1))}
                      className={cn(currentPage >= totalPages && "pointer-events-none opacity-50")}
                    />
                  </PaginationItem>
                </PaginationContent>
              </Pagination>
            )}
          </>
        )}

        {/* Remove confirm dialog */}
        <AlertDialog open={removing !== null} onOpenChange={(v) => !v && setRemoving(null)}>
          <AlertDialogContent>
            <AlertDialogHeader>
              <AlertDialogTitle>{t("mcp.delete")}</AlertDialogTitle>
              <AlertDialogDescription>{t("mcp.confirmDelete")}</AlertDialogDescription>
            </AlertDialogHeader>
            <AlertDialogFooter>
              <AlertDialogCancel>{t("mcp.cancel")}</AlertDialogCancel>
              <AlertDialogAction
                className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
                onClick={() => removing && removeMutation.mutate(removing)}
              >
                {t("mcp.delete")}
              </AlertDialogAction>
            </AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>

        <McpEditorDialog
          key={editing === "new" ? "__new__" : editing?.name ?? "__closed__"}
          open={editing !== null}
          server={editing === "new" ? undefined : editing ?? undefined}
          existingNames={servers.map((s) => s.name)}
          onClose={() => setEditing(null)}
        />
      </div>
    </TooltipProvider>
  );
}
