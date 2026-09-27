import * as React from "react";
import { Slot } from "@radix-ui/react-slot";
import { cva, type VariantProps } from "class-variance-authority";

import { cn } from "@/lib/utils";

const rainbowButtonVariants = cva(
  cn(
    "relative cursor-pointer group transition-[transform,background-color,box-shadow] duration-200 ease-out active:scale-[0.985] motion-reduce:transition-none motion-reduce:active:scale-100",
    "inline-flex items-center justify-center gap-2 shrink-0",
    "rounded-[8px] outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 aria-invalid:border-destructive",
    "text-sm font-medium whitespace-nowrap",
    "disabled:pointer-events-none disabled:opacity-50",
    "[&_svg]:pointer-events-none [&_svg:not([class*='size-'])]:size-4 [&_svg]:shrink-0",
  ),
  {
    variants: {
      variant: {
        default:
          "border border-primary bg-primary text-primary-foreground shadow-sm hover:bg-primary/90 hover:shadow-md",
        outline:
          "border border-input bg-card text-foreground hover:border-primary/40 hover:bg-muted/40",
      },
      size: {
        default: "h-[33px] px-4 py-2",
        sm: "h-[29px] rounded-[6px] px-3 text-xs",
        lg: "h-[41px] rounded-[6px] px-8",
        icon: "size-[33px]",
      },
    },
    defaultVariants: {
      variant: "default",
      size: "default",
    },
  },
);

export interface RainbowButtonProps
  extends React.ButtonHTMLAttributes<HTMLButtonElement>,
    VariantProps<typeof rainbowButtonVariants> {
  asChild?: boolean;
}

const RainbowButton = React.forwardRef<HTMLButtonElement, RainbowButtonProps>(
  ({ className, variant, size, asChild = false, ...props }, ref) => {
    const Comp = asChild ? Slot : "button";
    return (
      <Comp
        data-slot="button"
        className={cn(rainbowButtonVariants({ variant, size, className }))}
        ref={ref}
        {...props}
      />
    );
  },
);
RainbowButton.displayName = "RainbowButton";

export { RainbowButton, rainbowButtonVariants };
