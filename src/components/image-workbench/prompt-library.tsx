import { useEffect, useMemo, useState } from "react";
import { Check, ChevronDown, ChevronUp, Copy, Feather, Pin, PinOff, Search, Sparkles, Star, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  PROMPT_LIBRARY,
  loadPromptFavorites,
  promptRecipeKey,
  savePromptFavorites,
  togglePromptFavorite,
  type PromptRecipe,
} from "@/lib/image-prompts";
import { cn } from "@/lib/utils";

type PromptCategory = (typeof PROMPT_LIBRARY)[number];

/** Searchable prompt recipes with favorites pinned to the top; picking one fills the prompt. */
export function PromptLibrary({ prompt, onPromptChange }: {
  prompt: string;
  onPromptChange: (prompt: string) => void;
}) {
  const [activeCategory, setActiveCategory] = useState<string>(PROMPT_LIBRARY[0]?.category ?? "");
  const [promptQuery, setPromptQuery] = useState("");
  const [promptFavorites, setPromptFavorites] = useState<string[]>(() => loadPromptFavorites());
  const [copiedPrompt, setCopiedPrompt] = useState<string | null>(null);
  const [expandedPrompt, setExpandedPrompt] = useState<string | null>(null);
  const [libraryCollapsed, setLibraryCollapsed] = useState(false);

  const filteredPromptCategories = useMemo(() => {
    const q = promptQuery.trim().toLowerCase();
    if (!q) return PROMPT_LIBRARY;
    return PROMPT_LIBRARY.map(cat => ({
      ...cat,
      recipes: cat.recipes.filter(
        recipe =>
          recipe.title.toLowerCase().includes(q) ||
          recipe.tip.toLowerCase().includes(q) ||
          recipe.prompt.toLowerCase().includes(q) ||
          cat.category.toLowerCase().includes(q),
      ),
    })).filter(cat => cat.recipes.length > 0);
  }, [promptQuery]);

  const favoriteRecipes = useMemo(() => {
    const index = new Map<string, { category: string; recipe: PromptRecipe }>();
    for (const cat of PROMPT_LIBRARY) {
      for (const recipe of cat.recipes) {
        index.set(promptRecipeKey(cat.category, recipe), { category: cat.category, recipe });
      }
    }
    return promptFavorites
      .map(key => index.get(key))
      .filter((item): item is { category: string; recipe: PromptRecipe } => Boolean(item));
  }, [promptFavorites]);

  const visibleRecipes = useMemo(() => {
    const attach = (category: string, recipe: PromptRecipe) => ({
      category,
      recipe,
      key: promptRecipeKey(category, recipe),
    });
    if (promptQuery.trim()) {
      return filteredPromptCategories.flatMap(cat => cat.recipes.map(recipe => attach(cat.category, recipe)));
    }
    const pinned = favoriteRecipes.map(item => attach(item.category, item.recipe));
    const active = filteredPromptCategories.find(c => c.category === activeCategory)?.recipes ?? [];
    const pinnedKeys = new Set(pinned.map(item => item.key));
    const rest = active
      .map(recipe => attach(activeCategory, recipe))
      .filter(item => !pinnedKeys.has(item.key));
    return [...pinned, ...rest];
  }, [filteredPromptCategories, activeCategory, promptQuery, favoriteRecipes]);

  const toggleFavorite = (category: string, recipe: PromptRecipe) => {
    const key = promptRecipeKey(category, recipe);
    const next = togglePromptFavorite(promptFavorites, key);
    setPromptFavorites(next);
    savePromptFavorites(next);
  };

  useEffect(() => {
    if (promptQuery.trim()) return;
    if (PROMPT_LIBRARY.some(c => c.category === activeCategory)) return;
    setActiveCategory(PROMPT_LIBRARY[0]?.category ?? "");
  }, [promptQuery, activeCategory]);

  const applyPromptRecipe = (recipe: PromptRecipe) => {
    onPromptChange(recipe.prompt);
    setExpandedPrompt(recipe.prompt);
  };

  const copyPromptRecipe = async (recipe: PromptRecipe) => {
    try {
      await navigator.clipboard.writeText(recipe.prompt);
      setCopiedPrompt(recipe.prompt);
      setTimeout(() => setCopiedPrompt(null), 1600);
    } catch {
      applyPromptRecipe(recipe);
    }
  };

  return (
    <div className="space-y-2.5 rounded-xl bg-black/[0.025] p-2.5 ring-1 ring-border/50 dark:bg-white/[0.02] sm:space-y-3 sm:p-3">
      <div className="flex flex-wrap items-center justify-between gap-x-2 gap-y-1 text-xs">
        <div className="flex items-center gap-2">
          <span className="inline-flex min-w-0 items-center gap-1.5 font-serif font-semibold text-muted-foreground">
            <Sparkles className="h-3.5 w-3.5 shrink-0 text-[#b83b38] dark:text-[#d94f4b]" />
            <span className="truncate">画谱 · 提示词灵感</span>
            {promptFavorites.length > 0 && (
              <span className="shrink-0 rounded-full bg-amber-500/15 px-1.5 py-px text-[10px] font-normal text-amber-700 dark:text-amber-300">
                已藏 {promptFavorites.length}
              </span>
            )}
          </span>
        </div>
        <div className="flex items-center gap-2">
          <span className="hidden font-serif text-[11px] text-muted-foreground/80 sm:inline">
            点击填入，可继续编辑
          </span>
          <Button
            type="button"
            variant="ghost"
            size="xs"
            className="h-6 gap-1 px-1.5 text-[11px] text-muted-foreground hover:text-foreground"
            onClick={() => setLibraryCollapsed(!libraryCollapsed)}
          >
            <span>{libraryCollapsed ? "展开画谱" : "收起画谱"}</span>
            {libraryCollapsed ? <ChevronDown className="h-3.5 w-3.5" /> : <ChevronUp className="h-3.5 w-3.5" />}
          </Button>
        </div>
      </div>

      {!libraryCollapsed && (
        <>
          <div className="relative">
            <Search className="pointer-events-none absolute left-2 top-1/2 h-3 w-3 -translate-y-1/2 text-muted-foreground/70" />
            <input
              value={promptQuery}
              onChange={event => setPromptQuery(event.target.value)}
              placeholder="检索画谱：人物 / Cosplay / 海报…"
              className="h-8 w-full min-w-0 rounded-md border border-border/60 bg-background/80 pl-7 pr-7 text-xs outline-none transition-colors focus:border-[#b83b38]/40"
              aria-label="检索提示词画谱"
            />
            {promptQuery && (
              <button
                type="button"
                onClick={() => setPromptQuery("")}
                className="absolute right-2 top-1/2 -translate-y-1/2 text-muted-foreground/70 hover:text-foreground"
                aria-label="清除检索"
              >
                <X className="h-3 w-3" />
              </button>
            )}
          </div>
          {!promptQuery && (
            <div
              role="tablist"
              aria-label="提示词分类"
              className="flex flex-wrap gap-1 rounded-lg bg-background/45 p-1 ring-1 ring-border/45"
            >
              {filteredPromptCategories.map((cat: PromptCategory) => (
                <button
                  key={cat.category}
                  type="button"
                  role="tab"
                  aria-selected={activeCategory === cat.category}
                  onClick={() => setActiveCategory(cat.category)}
                  className={cn(
                    "inline-flex h-7 shrink-0 items-center justify-center gap-1 whitespace-nowrap rounded-md px-2.5 font-serif text-[11px] outline-none transition-[background-color,color,box-shadow] focus-visible:ring-2 focus-visible:ring-[#b83b38]/35 focus-visible:ring-offset-1",
                    activeCategory === cat.category
                      ? "bg-background font-bold text-[#b83b38] shadow-xs ring-1 ring-[#b83b38]/20 dark:bg-white/[0.06] dark:text-[#fca5a5] dark:ring-[#d94f4b]/25"
                      : "text-muted-foreground hover:bg-muted/70 hover:text-foreground",
                  )}
                >
                  <span>{cat.category}</span>
                  <span
                    className={cn(
                      "min-w-3.5 text-center font-sans text-[9px] tabular-nums opacity-60",
                      activeCategory === cat.category && "opacity-75",
                    )}
                  >
                    {cat.recipes.length}
                  </span>
                </button>
              ))}
            </div>
          )}
          <div className="flex max-h-96 min-h-[9rem] flex-col gap-2 overflow-y-auto overscroll-contain pr-1">
            {visibleRecipes.length === 0 && (
              <p className="py-6 text-center font-serif text-xs text-muted-foreground">
                画谱中没有匹配「{promptQuery}」的条目
              </p>
            )}
            {visibleRecipes.map(({ category, recipe, key }) => {
              const expanded = expandedPrompt === recipe.prompt;
              const filled = prompt === recipe.prompt;
              const starred = promptFavorites.includes(key);
              return (
                <div
                  key={key}
                  className={cn(
                    "shrink-0 rounded-lg border border-border/60 bg-background/70 p-2 transition-all sm:p-2.5",
                    filled
                      ? "border-[#b83b38]/45 bg-[#b83b38]/[0.04] dark:border-[#d94f4b]/40 dark:bg-[#d94f4b]/[0.06]"
                      : "hover:border-[#b83b38]/35",
                  )}
                >
                  <div className="flex items-start justify-between gap-2">
                    <div className="min-w-0">
                      <p className="flex items-center gap-1 font-serif text-xs font-semibold text-foreground/90">
                        {starred && <Star className="h-3 w-3 fill-amber-400 text-amber-500" />}
                        {recipe.title}
                        {promptQuery.trim() && (
                          <span className="ml-1 rounded bg-muted px-1 py-px text-[9px] font-normal text-muted-foreground">
                            {category}
                          </span>
                        )}
                      </p>
                      <p className="mt-0.5 font-serif text-[11px] leading-snug text-muted-foreground/85">
                        {recipe.tip}
                      </p>
                    </div>
                    <div className="flex shrink-0 items-center gap-0.5">
                      <Button
                        type="button"
                        size="icon"
                        variant="ghost"
                        className="h-7 w-7 text-muted-foreground hover:text-amber-500"
                        title={starred ? "取消收藏" : "收藏并置顶"}
                        aria-label={starred ? "取消收藏" : "收藏并置顶"}
                        onClick={() => toggleFavorite(category, recipe)}
                      >
                        {starred ? (
                          <PinOff className="h-3.5 w-3.5" />
                        ) : (
                          <Pin className="h-3.5 w-3.5" />
                        )}
                      </Button>
                      <Button
                        type="button"
                        size="icon"
                        variant="ghost"
                        className="h-7 w-7 text-muted-foreground hover:text-[#b83b38] dark:hover:text-[#fca5a5]"
                        title={filled ? "已填入" : "填入画题"}
                        aria-label={filled ? "已填入" : "填入画题"}
                        onClick={() => applyPromptRecipe(recipe)}
                      >
                        {filled ? <Check className="h-3.5 w-3.5 text-emerald-500" /> : <Feather className="h-3.5 w-3.5" />}
                      </Button>
                      <Button
                        type="button"
                        size="icon"
                        variant="ghost"
                        className="h-7 w-7 text-muted-foreground hover:text-foreground"
                        title="复制提示词"
                        aria-label="复制提示词"
                        onClick={() => void copyPromptRecipe(recipe)}
                      >
                        {copiedPrompt === recipe.prompt ? (
                          <Check className="h-3.5 w-3.5 text-emerald-500" />
                        ) : (
                          <Copy className="h-3.5 w-3.5" />
                        )}
                      </Button>
                    </div>
                  </div>
                  <button
                    type="button"
                    className="mt-1 w-full text-left"
                    onClick={() => setExpandedPrompt(expanded ? null : recipe.prompt)}
                  >
                    <p
                      className={cn(
                        "whitespace-pre-wrap break-words font-serif text-[11px] leading-relaxed text-muted-foreground transition-all",
                        expanded ? "" : "line-clamp-2",
                      )}
                    >
                      {recipe.prompt}
                    </p>
                    <span className="mt-0.5 inline-block font-serif text-[10px] text-muted-foreground/60 hover:text-foreground">
                      {expanded ? "收起" : "展开全文"}
                    </span>
                  </button>
                </div>
              );
            })}
          </div>
        </>
      )}
    </div>
  );
}
