import { useEffect, useMemo, useRef, useState } from "react";
import { Check, ChevronDown } from "lucide-react";
import { Input } from "@/components/ui/input";
import { cn } from "@/lib/utils";

type InkModelComboboxProps = {
  value: string;
  options: string[];
  onChange: (value: string) => void;
};

export function InkModelCombobox({ value, options, onChange }: InkModelComboboxProps) {
  const rootRef = useRef<HTMLDivElement>(null);
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");

  const filteredOptions = useMemo(() => {
    const normalized = query.trim().toLocaleLowerCase();
    if (!normalized) return options;
    return options.filter(option => option.toLocaleLowerCase().includes(normalized));
  }, [options, query]);

  useEffect(() => {
    if (!open) return;
    const closeOnOutsidePress = (event: PointerEvent) => {
      if (!rootRef.current?.contains(event.target as Node)) setOpen(false);
    };
    document.addEventListener("pointerdown", closeOnOutsidePress);
    return () => document.removeEventListener("pointerdown", closeOnOutsidePress);
  }, [open]);

  const choose = (option: string) => {
    onChange(option);
    setQuery("");
    setOpen(false);
  };

  return (
    <div ref={rootRef} className="relative">
      <Input
        value={value}
        onChange={event => {
          onChange(event.target.value);
          setQuery(event.target.value);
          setOpen(true);
        }}
        onFocus={() => {
          setQuery("");
          setOpen(true);
        }}
        onKeyDown={event => {
          if (event.key === "Escape") setOpen(false);
          if (event.key === "Enter") setOpen(false);
          if (event.key === "ArrowDown") {
            event.preventDefault();
            setOpen(true);
            requestAnimationFrame(() => {
              rootRef.current?.querySelector<HTMLButtonElement>('[role="option"]')?.focus();
            });
          }
        }}
        role="combobox"
        aria-autocomplete="list"
        aria-expanded={open}
        aria-controls="image-model-options"
        placeholder="gpt-image-2"
        className="ink-model-input pr-10 font-mono text-sm"
      />
      <button
        type="button"
        aria-label={open ? "收起模型选项" : "展开模型选项"}
        aria-expanded={open}
        onMouseDown={event => event.preventDefault()}
        onClick={() => {
          setQuery("");
          setOpen(current => !current);
        }}
        className="absolute inset-y-0 right-0 flex w-10 items-center justify-center text-[#685d50] outline-none transition-colors hover:text-[#b83b38] focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-[#b83b38]/35 dark:text-[#cbbda8] dark:hover:text-[#efaaa7]"
      >
        <ChevronDown className={cn("h-4 w-4 transition-transform duration-200", open && "rotate-180")} />
      </button>
      {open && (
        <div
          id="image-model-options"
          role="listbox"
          aria-label="可用生图模型"
          className="ink-combobox-content"
        >
          <div className="ink-combobox-heading">可用模型</div>
          <div className="max-h-52 overflow-y-auto p-1.5 ink-combobox-scroll">
            {filteredOptions.length > 0 ? filteredOptions.map(option => (
              <button
                key={option}
                type="button"
                role="option"
                aria-selected={option === value}
                onClick={() => choose(option)}
                className="ink-combobox-item"
              >
                <span className="min-w-0 flex-1 truncate font-mono text-sm">{option}</span>
                {option === value && <Check className="h-4 w-4 shrink-0 text-[#b83b38] dark:text-[#efaaa7]" />}
              </button>
            )) : (
              <p className="px-3 py-5 text-center font-serif text-xs text-muted-foreground">
                无匹配模型，可直接使用当前输入
              </p>
            )}
          </div>
          {query.trim() && !options.some(option => option.toLocaleLowerCase() === query.trim().toLocaleLowerCase()) && (
            <div className="ink-combobox-note">按 Enter 保留自定义模型名称</div>
          )}
        </div>
      )}
    </div>
  );
}
