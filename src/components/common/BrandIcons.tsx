/**
 * 专有功能特性矢量图标库。
 * 遵循 Lucide 图标标准规范：纯线条描边（stroke="currentColor", strokeWidth="2"）、无背景色填充（fill="none"）。
 * 与侧边栏、按钮与菜单项的色彩状态（默认中灰、悬停/激活跟随字体色）无缝融合，拒绝任何突兀色块。
 */

import type { SVGProps } from 'react';
import { cn } from '@/lib/utils';

export interface BrandIconProps extends SVGProps<SVGSVGElement> {
  className?: string;
  size?: number | string;
}

/**
 * JS 脚本图标（JavaScript Badge）：
 * 遵循 Lucide 极简单色线框风格，外框为圆角方框，内置 JS 单线字符，纯 currentColor 描边。
 */
export function JsScriptIcon({ className, size = 18, ...props }: BrandIconProps) {
  return (
    <svg
      viewBox="0 0 24 24"
      width={size}
      height={size}
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      xmlns="http://www.w3.org/2000/svg"
      className={cn('inline-block shrink-0 align-middle transition-colors select-none', className)}
      aria-hidden="true"
      {...props}
    >
      {/* 极简圆角外框 */}
      <rect x="3" y="3" width="18" height="18" rx="3.5" />
      {/* 字母 J */}
      <path d="M9.5 8.5v5a2 2 0 0 1-4 0" />
      {/* 字母 S */}
      <path d="M14 9.5a1.5 1.5 0 0 1 3 0c0 1.5-3 1.5-3 3a1.5 1.5 0 0 0 3 0" />
    </svg>
  );
}

/**
 * 定时任务调度图标（Scheduler）：
 * 遵循 Lucide 极简单色线框风格，结合时钟指针与周期循环弧线，纯 currentColor 描边。
 */
export function SchedulerBrandIcon({ className, size = 18, ...props }: BrandIconProps) {
  return (
    <svg
      viewBox="0 0 24 24"
      width={size}
      height={size}
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      xmlns="http://www.w3.org/2000/svg"
      className={cn('inline-block shrink-0 align-middle transition-colors select-none', className)}
      aria-hidden="true"
      {...props}
    >
      {/* 循环调度外弧 */}
      <path d="M12 2.5a9.5 9.5 0 1 1-9.5 9.5c0-1.8.5-3.5 1.4-5" />
      {/* 循环箭头 */}
      <path d="M3.5 4.5l.8 2.8 2.8-.8" />
      {/* 时分指针 */}
      <path d="M12 7.5V12l3 2" />
    </svg>
  );
}
