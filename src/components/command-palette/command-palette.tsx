import { useState, useEffect, useMemo, useRef } from "react";
import { useTranslation } from "react-i18next";
import { useQueryClient } from "@tanstack/react-query";
import {
  Search,
  LayoutDashboard,
  Wallet,
  FileCode2,
  Server,
  Sparkles,
  Wrench,
  Settings,
  Sun,
  Moon,
  FolderOpen,
  RotateCw,
  Terminal,
  MessagesSquare,
  ArrowRight,
  ShieldCheck,
  UsersRound,
} from "lucide-react";
import { Dialog, DialogContent } from "@/components/ui/dialog";
import { cn } from "@/lib/utils";
import { api } from "@/lib/api";
import { toast } from "@/hooks/use-toast";
import type { Route } from "@/types/navigation";
import type { CoreEnvelope, CoreSnapshotPayload, McpServerListPayload, SkillListPayload } from "@/types";
import { queryKeys } from "@/lib/query-keys";

interface CommandPaletteProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onNavigate: (route: Route) => void;
  onThemeToggle: () => void;
  isDarkMode: boolean;
}

interface CommandItem {
  id: string;
  title: string;
  subtitle?: string;
  icon: React.ElementType;
  category: "navigation" | "actions" | "mcp" | "skills";
  shortcut?: string;
  onSelect: () => void;
}

export function CommandPalette({
  open,
  onOpenChange,
  onNavigate,
  onThemeToggle,
  isDarkMode,
}: CommandPaletteProps) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [query, setQuery] = useState("");
  const [selectedIndex, setSelectedIndex] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  // Reset query on open
  useEffect(() => {
    if (open) {
      setQuery("");
      setSelectedIndex(0);
      setTimeout(() => inputRef.current?.focus(), 50);
    }
  }, [open]);

  // Load snapshot data for paths
  const snapshotData = queryClient.getQueryData<CoreEnvelope<CoreSnapshotPayload>>(queryKeys.runtimeStateDisplay);
  const codexHome = snapshotData?.data.status.paths.codexHome ?? "";
  const sessionsPath = snapshotData?.data.status.paths.sessionsPath ?? "";

  // Load MCP servers
  const mcpData = queryClient.getQueryData<CoreEnvelope<McpServerListPayload>>(queryKeys.mcpServers);
  const mcpServers = mcpData?.data.items ?? [];

  // Load Skills
  const skillsData = queryClient.getQueryData<CoreEnvelope<SkillListPayload>>(queryKeys.installedSkills);
  const skillsList = skillsData?.data.items ?? [];

  const handleOpenPath = async (path: string, label: string) => {
    if (!path) return;
    try {
      await api.openPath(path);
      toast({ title: `${label} 已打开`, variant: "default" });
    } catch (err) {
      toast({ title: t("common.error"), description: String(err), variant: "destructive" });
    }
  };

  const handleRestartCodex = async () => {
    try {
      await api.targets.restart("codex");
      toast({ title: "Codex 重启指令已发送", variant: "default" });
    } catch (err) {
      toast({ title: t("common.error"), description: String(err), variant: "destructive" });
    }
  };

  // Build command items
  const allItems = useMemo<CommandItem[]>(() => {
    const items: CommandItem[] = [
      // 页面导航
      {
        id: "nav-overview",
        title: t("nav.overview"),
        subtitle: "主工作台与核心能力总览",
        icon: LayoutDashboard,
        category: "navigation",
        shortcut: "⌘1",
        onSelect: () => {
          onNavigate("overview");
          onOpenChange(false);
        },
      },
      {
        id: "nav-accounts",
        title: t("nav.accounts"),
        subtitle: "Codex 官方账号登录、切换与额度监控",
        icon: UsersRound,
        category: "navigation",
        onSelect: () => {
          onNavigate("accounts");
          onOpenChange(false);
        },
      },
      {
        id: "nav-newapi",
        title: t("newapi.title"),
        subtitle: "中转模型配置与本地代理管理",
        icon: Wallet,
        category: "navigation",
        shortcut: "⌘2",
        onSelect: () => {
          onNavigate("newapi");
          onOpenChange(false);
        },
      },
      {
        id: "nav-threads",
        title: t("nav.threads"),
        subtitle: "按文件夹归档的 Codex 对话线程与导出",
        icon: MessagesSquare,
        category: "navigation",
        onSelect: () => {
          onNavigate("threads");
          onOpenChange(false);
        },
      },
      {
        id: "nav-customInstructions",
        title: t("nav.customInstructions"),
        subtitle: "全局 AGENTS.md 自定义指令与模板",
        icon: FileCode2,
        category: "navigation",
        shortcut: "⌘3",
        onSelect: () => {
          onNavigate("customInstructions");
          onOpenChange(false);
        },
      },
      {
        id: "nav-mcp",
        title: t("nav.mcp"),
        subtitle: "MCP 工具协议与本地/远程端点",
        icon: Server,
        category: "navigation",
        shortcut: "⌘4",
        onSelect: () => {
          onNavigate("mcp");
          onOpenChange(false);
        },
      },
      {
        id: "nav-skills",
        title: t("nav.skills"),
        subtitle: "技能插件中心与历史备份归档",
        icon: Sparkles,
        category: "navigation",
        shortcut: "⌘5",
        onSelect: () => {
          onNavigate("skills");
          onOpenChange(false);
        },
      },
      {
        id: "nav-maintenance",
        title: t("nav.maintenance"),
        subtitle: "系统健康诊断、缓存清理与注册表重构",
        icon: Wrench,
        category: "navigation",
        shortcut: "⌘6",
        onSelect: () => {
          onNavigate("maintenance");
          onOpenChange(false);
        },
      },
      {
        id: "nav-settings",
        title: t("nav.settings"),
        subtitle: "外观主题、网络代理与系统偏好",
        icon: Settings,
        category: "navigation",
        shortcut: "⌘7",
        onSelect: () => {
          onNavigate("settings");
          onOpenChange(false);
        },
      },

      // 快捷操作
      {
        id: "act-restart-codex",
        title: "重启 Codex 进程",
        subtitle: "应用最新配置并重启本地 Codex",
        icon: RotateCw,
        category: "actions",
        onSelect: () => {
          void handleRestartCodex();
          onOpenChange(false);
        },
      },
      {
        id: "act-open-codex-home",
        title: "打开 .codex 根目录",
        subtitle: codexHome || "在文件管理器中查看",
        icon: FolderOpen,
        category: "actions",
        onSelect: () => {
          void handleOpenPath(codexHome, ".codex 根目录");
          onOpenChange(false);
        },
      },
      {
        id: "act-open-sessions",
        title: "打开 Sessions 会话目录",
        subtitle: sessionsPath || "查看本地对话日志",
        icon: FolderOpen,
        category: "actions",
        onSelect: () => {
          void handleOpenPath(sessionsPath, "Sessions 目录");
          onOpenChange(false);
        },
      },
      {
        id: "act-toggle-theme",
        title: isDarkMode ? "切换为明亮模式" : "切换为深色模式",
        subtitle: "快速切换系统外观配色",
        icon: isDarkMode ? Sun : Moon,
        category: "actions",
        onSelect: () => {
          onThemeToggle();
          onOpenChange(false);
        },
      },
      {
        id: "act-diagnose",
        title: "运行系统诊断",
        subtitle: "检查本地 Codex 配置文件与环境健康",
        icon: ShieldCheck,
        category: "actions",
        onSelect: () => {
          onNavigate("maintenance");
          onOpenChange(false);
        },
      },
    ];

    // 动态添加 MCP 服务项
    mcpServers.forEach((server) => {
      items.push({
        id: `mcp-${server.name}`,
        title: `MCP: ${server.name}`,
        subtitle: `${server.transport.toUpperCase()} · ${server.enabled ? "运行中" : "已停用"} · ${server.command || server.url}`,
        icon: Terminal,
        category: "mcp",
        onSelect: () => {
          onNavigate("mcp");
          onOpenChange(false);
        },
      });
    });

    // 动态添加 Skills 项
    skillsList.forEach((skill) => {
      items.push({
        id: `skill-${skill.id}`,
        title: `Skill: ${skill.title || skill.name}`,
        subtitle: skill.summary || skill.relativePath,
        icon: Sparkles,
        category: "skills",
        onSelect: () => {
          onNavigate("skills");
          onOpenChange(false);
        },
      });
    });

    return items;
  }, [t, onNavigate, onOpenChange, isDarkMode, onThemeToggle, codexHome, sessionsPath, mcpServers, skillsList]);

  // Filter items by query
  const filteredItems = useMemo(() => {
    if (!query.trim()) return allItems;
    const q = query.toLowerCase().trim();
    return allItems.filter(
      (item) =>
        item.title.toLowerCase().includes(q) ||
        (item.subtitle && item.subtitle.toLowerCase().includes(q))
    );
  }, [allItems, query]);

  // Keep selected index in bound
  useEffect(() => {
    setSelectedIndex(0);
  }, [query]);

  // Keyboard navigation
  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (filteredItems.length === 0) return;

    if (e.key === "ArrowDown") {
      e.preventDefault();
      setSelectedIndex((prev) => (prev + 1) % filteredItems.length);
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setSelectedIndex((prev) => (prev - 1 + filteredItems.length) % filteredItems.length);
    } else if (e.key === "Enter") {
      e.preventDefault();
      filteredItems[selectedIndex]?.onSelect();
    }
  };

  // Group filtered items by category
  const categories = useMemo(() => {
    const map = new Map<string, { label: string; items: CommandItem[] }>();
    const labelMap: Record<string, string> = {
      navigation: "页面导航",
      actions: "快捷指令与操作",
      mcp: "MCP 服务条目",
      skills: "Skills 本地技能",
    };

    filteredItems.forEach((item) => {
      if (!map.has(item.category)) {
        map.set(item.category, {
          label: labelMap[item.category] ?? item.category,
          items: [],
        });
      }
      map.get(item.category)!.items.push(item);
    });

    return Array.from(map.values());
  }, [filteredItems]);

  let globalIndexCounter = 0;

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="p-0 gap-0 max-w-xl overflow-hidden border-primary/20 shadow-2xl rounded-2xl bg-card/95 backdrop-blur-xl">
        {/* Search Input Bar */}
        <div className="flex items-center border-b border-border/70 px-4 py-3 gap-3">
          <Search className="h-5 w-5 text-muted-foreground shrink-0" />
          <input
            ref={inputRef}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={handleKeyDown}
            placeholder="搜索页面、常用指令、MCP 或 Skill... (输入关键词)"
            className="flex-1 bg-transparent text-sm text-foreground placeholder:text-muted-foreground/70 focus:outline-none"
          />
          <kbd className="hidden sm:inline-flex h-5 items-center gap-1 rounded border border-border/60 bg-muted/60 px-1.5 font-mono text-[10px] text-muted-foreground">
            ESC
          </kbd>
        </div>

        {/* Results List */}
        <div ref={listRef} className="max-h-[380px] overflow-y-auto p-2 space-y-3">
          {filteredItems.length === 0 ? (
            <div className="py-12 text-center text-sm text-muted-foreground">
              <Search className="h-8 w-8 mx-auto text-muted-foreground/40 mb-2" />
              未找到与「{query}」相关的页面或指令
            </div>
          ) : (
            categories.map((cat) => (
              <div key={cat.label} className="space-y-1">
                <div className="px-2 py-1 text-[11px] font-semibold text-muted-foreground/70 uppercase tracking-wider">
                  {cat.label}
                </div>
                {cat.items.map((item) => {
                  const itemIndex = globalIndexCounter++;
                  const isSelected = itemIndex === selectedIndex;
                  const Icon = item.icon;

                  return (
                    <div
                      key={item.id}
                      onClick={() => item.onSelect()}
                      onMouseEnter={() => setSelectedIndex(itemIndex)}
                      className={cn(
                        "flex items-center justify-between gap-3 px-3 py-2 rounded-xl text-sm cursor-pointer transition-all",
                        isSelected
                          ? "bg-primary text-primary-foreground shadow-sm"
                          : "text-foreground/90 hover:bg-muted/60"
                      )}
                    >
                      <div className="flex items-center gap-3 min-w-0 flex-1">
                        <div
                          className={cn(
                            "flex h-7 w-7 shrink-0 items-center justify-center rounded-lg ring-1 ring-inset",
                            isSelected
                              ? "bg-primary-foreground/15 text-primary-foreground ring-primary-foreground/20"
                              : "bg-muted text-muted-foreground ring-border/50"
                          )}
                        >
                          <Icon className="h-3.5 w-3.5" />
                        </div>
                        <div className="min-w-0 flex-1">
                          <div className="font-medium truncate leading-none">{item.title}</div>
                          {item.subtitle && (
                            <div
                              className={cn(
                                "text-[11px] truncate mt-1 leading-none",
                                isSelected ? "text-primary-foreground/80" : "text-muted-foreground"
                              )}
                            >
                              {item.subtitle}
                            </div>
                          )}
                        </div>
                      </div>

                      {item.shortcut ? (
                        <kbd
                          className={cn(
                            "h-5 items-center px-1.5 rounded font-mono text-[10px]",
                            isSelected
                              ? "bg-primary-foreground/20 text-primary-foreground"
                              : "bg-muted text-muted-foreground border border-border/50"
                          )}
                        >
                          {item.shortcut}
                        </kbd>
                      ) : isSelected ? (
                        <ArrowRight className="h-3.5 w-3.5 text-primary-foreground/80 shrink-0" />
                      ) : null}
                    </div>
                  );
                })}
              </div>
            ))
          )}
        </div>

        {/* Footer info bar */}
        <div className="flex items-center justify-between border-t border-border/70 px-4 py-2 text-[11px] text-muted-foreground bg-muted/30">
          <div className="flex items-center gap-2">
            <span>导航: <kbd className="font-mono bg-muted px-1 rounded">↑</kbd> <kbd className="font-mono bg-muted px-1 rounded">↓</kbd></span>
            <span>确认: <kbd className="font-mono bg-muted px-1 rounded">↵</kbd></span>
          </div>
          <span className="font-mono text-[10px]">AgentHarbor Spotlight</span>
        </div>
      </DialogContent>
    </Dialog>
  );
}
