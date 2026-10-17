import { AnimatedSegmentedControl } from "@/components/ui/animated-segmented-control";

/** The compact switch the usage page uses for its ranges, measures and groupings. */
export function UsageSegments<T extends string>({ label, value, items, onChange }: {
  label: string;
  value: T;
  items: readonly { value: T; label: string }[];
  onChange: (value: T) => void;
}) {
  return <AnimatedSegmentedControl ariaLabel={label} items={items} value={value}
    onValueChange={next => onChange(next as T)}
    className="usage-segments gap-0.5"
    indicatorClassName="rounded-[9px] bg-background shadow-sm dark:bg-white/[0.1]"
    itemClassName="h-7 whitespace-nowrap rounded-[9px] px-2.5 text-[12px] font-medium"
    activeItemClassName="text-foreground"
    inactiveItemClassName="text-muted-foreground hover:text-foreground" />;
}
