import { Eye, EyeOff } from 'lucide-react';
import { useState, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { cn } from '@/lib/utils';

/** 表单基元：统一样式，设置/主机等表单复用。 */

export function FormField({ label, children }: { label: ReactNode; children: ReactNode }) {
  return (
    <label className="flex flex-col gap-1.5">
      <span className="text-ui-sm text-foreground-subtle select-none">{label}</span>
      {children}
    </label>
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

export function Select(props: React.ComponentProps<'select'>) {
  return (
    <select
      {...props}
      className={cn(controlClassName, 'cursor-pointer appearance-none pr-6', props.className)}
    />
  );
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
      <button
        type="button"
        title={label}
        aria-label={label}
        onClick={() => setVisible((value) => !value)}
        className="absolute right-1 top-1/2 inline-flex size-6 -translate-y-1/2 items-center justify-center rounded-md text-foreground-subtle transition-colors hover:bg-hover hover:text-foreground"
      >
        {visible ? <EyeOff className="size-3.5" /> : <Eye className="size-3.5" />}
      </button>
    </div>
  );
}
