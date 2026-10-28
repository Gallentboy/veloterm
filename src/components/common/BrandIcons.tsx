/**
 * 专有品牌与功能特性定制矢量图标。
 * 针对深色模式（Dark）与浅色模式（Light）全自动适配高对比度质感。
 * 兼容 LucideIcon 签名：(props) => ReactNode，可在侧边栏、工具栏、列表无缝使用。
 */

import type { SVGProps } from 'react';
import { cn } from '@/lib/utils';

export interface BrandIconProps extends SVGProps<SVGSVGElement> {
  className?: string;
  size?: number | string;
}

/**
 * JS 脚本专有品牌图标（JavaScript Badge）：
 * - 浅色模式：经典明快阳光金底 + 纯黑高对比雕刻字符
 * - 深色模式：高级黑曜琥珀微光卡片 + 发光霓虹金字符
 */
export function JsScriptIcon({ className, size = 18, ...props }: BrandIconProps) {
  return (
    <svg
      viewBox="0 0 24 24"
      width={size}
      height={size}
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
      className={cn('inline-block shrink-0 align-middle transition-colors select-none', className)}
      aria-hidden="true"
      {...props}
    >
      {/* 现代圆角方形外框底座 */}
      <rect
        x="2.2"
        y="2.2"
        width="19.6"
        height="19.6"
        rx="4.2"
        className="fill-amber-400 dark:fill-amber-400/15 stroke-amber-500/80 dark:stroke-amber-400/40"
        strokeWidth="1.2"
      />
      {/* 字母 J */}
      <path
        d="M10.8 7.6v6.2c0 1.5-.9 2.5-2.3 2.5-1.1 0-2-.6-2.3-1.6"
        className="stroke-[#111827] dark:stroke-amber-300"
        strokeWidth="1.9"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      {/* 字母 S */}
      <path
        d="M17.4 9.6c-.4-.9-1.2-1.4-2.2-1.4-1.2 0-2 .7-2 1.8 0 1.1.8 1.6 2 2 1.3.4 2.2 1 2.2 2.2 0 1.3-1 2.2-2.5 2.2-1.3 0-2.3-.7-2.6-1.7"
        className="stroke-[#111827] dark:stroke-amber-300"
        strokeWidth="1.9"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

/**
 * 定时任务现代调度图标（Scheduler）：
 * - 结合时钟圆环、周期脉冲弧与精准指针，比传统单线日历钟更具动感与机械科技美感
 * - 深浅模式通过 currentColor 与语义色无缝融合
 */
export function SchedulerBrandIcon({ className, size = 18, ...props }: BrandIconProps) {
  return (
    <svg
      viewBox="0 0 24 24"
      width={size}
      height={size}
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
      className={cn('inline-block shrink-0 align-middle transition-colors select-none', className)}
      aria-hidden="true"
      {...props}
    >
      {/* 外圈循环轨迹弧（象征周期 cron 调度） */}
      <path
        d="M12 2.5a9.5 9.5 0 1 1-9.5 9.5c0-1.8.5-3.5 1.4-5"
        className="stroke-primary/40 dark:stroke-primary/30"
        strokeWidth="1.6"
        strokeLinecap="round"
      />
      {/* 脉冲箭头 */}
      <path
        d="M3.5 4.5l.8 2.8 2.8-.8"
        className="stroke-primary/80 dark:stroke-primary/70"
        strokeWidth="1.6"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      {/* 时钟中心点与两根指针（精巧时分刻度） */}
      <circle cx="12" cy="12" r="1.5" className="fill-primary" />
      <path
        d="M12 7.5V12l3 2"
        className="stroke-primary"
        strokeWidth="1.8"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}
