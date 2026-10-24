import { Loader } from 'lucide-react';

import { cn } from '@/lib/utils';

/**
 * 加载指示器（移植自 ZCode packages/ui/src/components/ui/spinner.tsx）：
 * **Loader 图标**（8 根辐条呈扇形展开，非闭合圆环——ZCode 的 LoaderIcon
 * 即此形态）+ Tailwind 内置 animate-spin，旋转观感与 ZCode 一致；附带
 * role="status" + aria-label 无障碍语义。ZCode 用 i18n 取 label，本项目
 * 组件层不接 i18n（调用方如需本地化可传 aria-label 覆盖）。
 */
function Spinner({ className, ...props }: React.ComponentProps<'svg'>) {
  return (
    <Loader
      role="status"
      aria-label="Loading"
      className={cn('size-4 animate-spin', className)}
      {...props}
    />
  );
}

export { Spinner };
