import type { ComponentProps } from 'react';
import { cva, type VariantProps } from 'class-variance-authority';
import { cn } from '@/lib/utils';

/**
 * 通用按钮（移植自 ZCode components/ui/button.tsx，去掉 radix Slot/asChild）。
 * 变体/尺寸与 ZCode 完全一致；所有窗口 chrome 按钮（头部、浮层、侧边栏）统一走这里，
 * 新增按钮不需要再单独调 hover/尺寸——选 variant + size 即可。
 */
const buttonVariants = cva(
  "group/button inline-flex shrink-0 items-center justify-center rounded-lg border border-transparent bg-clip-padding text-ui-base/relaxed whitespace-nowrap transition-all outline-none select-none disabled:pointer-events-none disabled:opacity-50 [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-4",
  {
    variants: {
      variant: {
        default:
          'bg-primary text-primary-foreground shadow-xs hover:bg-primary/90 active:bg-primary/80 font-medium',
        outline:
          'border-border/80 bg-surface/40 text-foreground hover:border-border hover:bg-hover hover:text-foreground active:bg-hover/80 shadow-2xs font-medium aria-expanded:bg-hover aria-expanded:text-foreground',
        secondary:
          'bg-secondary text-foreground hover:bg-secondary/80 active:bg-secondary/70 font-medium aria-expanded:bg-secondary aria-expanded:text-foreground',
        ghost:
          'text-foreground hover:bg-hover hover:text-foreground active:bg-hover/80 font-medium aria-expanded:bg-hover aria-expanded:text-foreground',
        destructive:
          'bg-destructive text-destructive-foreground hover:bg-destructive/90 active:bg-destructive/80 font-medium aria-expanded:bg-destructive aria-expanded:text-destructive-foreground',
        warning:
          'bg-warning text-warning-foreground hover:bg-warning/90 active:bg-warning/80 font-medium aria-expanded:bg-warning aria-expanded:text-warning-foreground',
        link: 'text-primary underline-offset-4 hover:underline',
      },
      size: {
        default:
          "h-7.5 gap-1 px-2 text-ui-xs [&_svg:not([class*='size-'])]:size-3.5",
        xs: "h-5 gap-0.5 rounded-md px-1.5 text-ui-xs [&_svg:not([class*='size-'])]:size-2.5",
        sm: "h-7 gap-1 px-2 text-ui-xs [&_svg:not([class*='size-'])]:size-3",
        md: "h-8 gap-1.5 px-2.5 text-ui-sm [&_svg:not([class*='size-'])]:size-3.5",
        lg: "h-8.5 gap-1.5 px-2.5 text-ui-sm [&_svg:not([class*='size-'])]:size-4",
        icon: "size-7.5 [&_svg:not([class*='size-'])]:size-4",
        'icon-xs': "size-5.5 rounded-md [&_svg:not([class*='size-'])]:size-2.5",
        'icon-sm': "size-6.5 [&_svg:not([class*='size-'])]:size-3",
        'icon-md': "size-7.5 rounded-lg [&_svg:not([class*='size-'])]:size-4",
        'icon-lg': "size-8.5 rounded-lg [&_svg:not([class*='size-'])]:size-4",
      },
    },
    defaultVariants: {
      variant: 'default',
      size: 'default',
    },
  },
);

export type ButtonProps = ComponentProps<'button'> & VariantProps<typeof buttonVariants>;

export function Button({ className, variant, size, type = 'button', ...props }: ButtonProps) {
  return <button type={type} className={cn(buttonVariants({ variant, size }), className)} {...props} />;
}

export { buttonVariants };
