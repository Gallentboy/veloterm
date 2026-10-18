import { Eye, EyeOff } from 'lucide-react';
import { useState, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { ControlHintTooltip } from '@/components/ControlHintTooltip';
import { SelectMenu, type SelectMenuOption } from '@/components/ui/SelectMenu';
import { cn } from '@/lib/utils';

/** 表单基元：统一样式，设置/主机等表单复用。 */

/**
 * 表单字段容器：**刻意用 div 而非 label**。
 *
 * label 的隐式关联会把内部点击转发给「第一个可标签化后代」，对复合字段是灾难：
 * - WebKit（Tauri 的 WKWebView）连点击内部按钮也会转发（Chrome 会按规范跳过）；
 * - 就绪钩子字段里第一个可标签控件是下拉触发器 —— 点「取消新建命令」后
 *   React 同步重渲染（下拉刚挂载），浏览器的转发刚好落到它头上，下拉诡异地
 *   自动展开；
 * - 同类隐患：点「所属分组」标题文字误勾第一个分组、点「认证方式」误选第一项。
 * 单控件场景「点标题聚焦输入框」的小便利让位于正确性（用户点输入框本身即可）。
 */
export function FormField({ label, children, className }: { label: ReactNode; children: ReactNode; className?: string }) {
  return (
    <div className={cn('flex flex-col gap-1.5', className)}>
      <span className="text-ui-sm text-foreground-subtle select-none">{label}</span>
      {children}
    </div>
  );
}

const controlClassName =
  'h-8 w-full rounded-md border border-border bg-input px-2 text-ui-base text-foreground outline-none transition-colors placeholder:text-foreground-subtlest focus:border-border-hover';

export function TextInput(props: React.ComponentProps<'input'>) {
  return <input {...props} className={cn(controlClassName, props.className)} />;
}

export function TextArea(props: React.ComponentProps<'textarea'>) {
  return (
    <textarea
      {...props}
      className={cn(controlClassName, 'h-24 resize-none py-1.5 font-mono text-ui-sm leading-relaxed', props.className)}
    />
  );
}

export interface SelectProps {
  options: SelectMenuOption[];
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
  disabled?: boolean;
  className?: string;
  'aria-label'?: string;
}

/** 表单下拉：自绘 SelectMenu 的同签名封装。**严禁用原生 `<select>`** ——
 * 其弹层由系统绘制，macOS 圆角菜单与 Windows 直角列表观感割裂、深色主题
 * 不可控（工程约定，见 SelectMenu 头注释） */
export function Select(props: SelectProps) {
  return <SelectMenu {...props} />;
}

/** 密码输入框：右侧眼睛按钮切换明文/密文 */
export function PasswordInput(props: Omit<React.ComponentProps<'input'>, 'type'>) {
  const { t } = useTranslation();
  const [visible, setVisible] = useState(false);
  const label = visible ? t('common.hidePassword') : t('common.showPassword');

  return (
    <div className="relative">
      <input
        {...props}
        type={visible ? 'text' : 'password'}
        className={cn(controlClassName, 'pr-9', props.className)}
      />
      <div className="absolute right-1 top-1/2 -translate-y-1/2">
        <ControlHintTooltip title={label} side="left">
          <button
            type="button"
            aria-label={label}
            onClick={() => setVisible((value) => !value)}
            className="inline-flex size-6 items-center justify-center rounded-md text-foreground-subtle transition-colors hover:bg-hover hover:text-foreground"
          >
            {visible ? <EyeOff className="size-3.5" /> : <Eye className="size-3.5" />}
          </button>
        </ControlHintTooltip>
      </div>
    </div>
  );
}
