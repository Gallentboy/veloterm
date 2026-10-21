import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import rehypeHighlight from 'rehype-highlight';
import { Check, Copy } from 'lucide-react';
import React, { memo, useCallback, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { cn } from '@/lib/utils';

interface MarkdownPreviewProps {
  content: string;
  className?: string;
}

/** 单个代码块容器：带顶部语言栏 + 一键复制代码 */
function CodeBlock({
  language,
  codeText,
  children,
}: {
  language?: string;
  codeText: string;
  children: React.ReactNode;
}) {
  const { t } = useTranslation();
  const [copied, setCopied] = useState(false);

  const handleCopy = useCallback(async () => {
    try {
      await navigator.clipboard.writeText(codeText);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch (err) {
      console.warn('Failed to copy code', err);
    }
  }, [codeText]);

  return (
    <div className="group relative my-3 overflow-hidden rounded-xl border border-border/70 bg-surface/60">
      <div className="flex items-center justify-between border-b border-border/50 bg-surface/80 px-3 py-1.5 text-ui-xs text-foreground-subtle">
        <span className="font-mono text-ui-xs uppercase tracking-wider">{language || 'text'}</span>
        <button
          type="button"
          onClick={handleCopy}
          className="flex items-center gap-1 rounded px-1.5 py-0.5 text-foreground-subtle transition-colors hover:bg-hover hover:text-foreground"
          title={copied ? t('files.copied', '已复制') : t('files.copyCode', '复制代码')}
          aria-label={copied ? t('files.copied', '已复制') : t('files.copyCode', '复制代码')}
        >
          {copied ? (
            <>
              <Check className="size-3 text-terminal-green" />
              <span className="text-ui-xs text-terminal-green">{t('files.copied', '已复制')}</span>
            </>
          ) : (
            <>
              <Copy className="size-3" />
              <span className="text-ui-xs">{t('files.copyCode', '复制')}</span>
            </>
          )}
        </button>
      </div>
      <div className="overflow-x-auto p-3 font-mono text-ui-sm leading-relaxed">
        {children}
      </div>
    </div>
  );
}

/**
 * SFTP Markdown 文件富文本预览：
 * - react-markdown + GFM（表格/任务列表/删除线）+ highlight.js 语法高亮
 * - 全量主题令牌（深色/浅色自适应）
 * - 代码块右上角带复制按钮与语言名提示
 * - GFM 表格带自适应滚动容器与斑马线
 */
export const MarkdownPreview = memo(function MarkdownPreview({
  content,
  className,
}: MarkdownPreviewProps) {
  const components = useMemo(
    () => ({
      h1: (props: React.HTMLAttributes<HTMLHeadingElement>) => (
        <h1
          {...props}
          className="mt-6 mb-3 border-b border-border/60 pb-2 text-ui-xl font-bold tracking-tight text-foreground first:mt-0"
        />
      ),
      h2: (props: React.HTMLAttributes<HTMLHeadingElement>) => (
        <h2
          {...props}
          className="mt-5 mb-2.5 border-b border-border/40 pb-1.5 text-ui-lg font-semibold tracking-tight text-foreground first:mt-0"
        />
      ),
      h3: (props: React.HTMLAttributes<HTMLHeadingElement>) => (
        <h3
          {...props}
          className="mt-4 mb-2 text-ui-base font-semibold text-foreground first:mt-0"
        />
      ),
      h4: (props: React.HTMLAttributes<HTMLHeadingElement>) => (
        <h4
          {...props}
          className="mt-3 mb-1.5 text-ui-base font-medium text-foreground first:mt-0"
        />
      ),
      h5: (props: React.HTMLAttributes<HTMLHeadingElement>) => (
        <h5
          {...props}
          className="mt-2.5 mb-1 text-ui-sm font-medium text-foreground-subtle first:mt-0"
        />
      ),
      h6: (props: React.HTMLAttributes<HTMLHeadingElement>) => (
        <h6
          {...props}
          className="mt-2 mb-1 text-ui-xs font-medium text-foreground-subtlest first:mt-0"
        />
      ),
      p: (props: React.HTMLAttributes<HTMLParagraphElement>) => (
        <p {...props} className="my-2.5 text-ui-sm leading-[1.7] text-foreground" />
      ),
      ul: (props: React.HTMLAttributes<HTMLUListElement>) => (
        <ul {...props} className="my-2 list-disc space-y-1 pl-5 text-ui-sm text-foreground" />
      ),
      ol: (props: React.HTMLAttributes<HTMLOListElement>) => (
        <ol {...props} className="my-2 list-decimal space-y-1 pl-5 text-ui-sm text-foreground" />
      ),
      li: (props: React.HTMLAttributes<HTMLLIElement>) => (
        <li {...props} className="leading-[1.7] text-foreground" />
      ),
      strong: (props: React.HTMLAttributes<HTMLElement>) => (
        <strong {...props} className="font-semibold text-foreground" />
      ),
      blockquote: (props: React.HTMLAttributes<HTMLQuoteElement>) => (
        <blockquote
          {...props}
          className="my-3 border-l-3 border-terminal-blue/70 bg-surface/40 py-1.5 pr-3 pl-3.5 text-ui-sm italic text-foreground-subtle"
        />
      ),
      a: (props: React.HTMLAttributes<HTMLAnchorElement>) => (
        <a
          {...props}
          className="text-terminal-blue underline-offset-3 hover:underline"
          target="_blank"
          rel="noopener noreferrer"
        />
      ),
      hr: (props: React.HTMLAttributes<HTMLHRElement>) => (
        <hr {...props} className="my-5 border-border/70" />
      ),
      table: (props: React.HTMLAttributes<HTMLTableElement>) => (
        <div className="my-3 overflow-x-auto rounded-xl border border-border/70">
          <table {...props} className="w-full border-collapse text-left text-ui-sm" />
        </div>
      ),
      thead: (props: React.HTMLAttributes<HTMLTableSectionElement>) => (
        <thead {...props} className="bg-surface border-b border-border/60" />
      ),
      th: (props: React.HTMLAttributes<HTMLTableCellElement>) => (
        <th
          {...props}
          className="border-r border-border/40 px-3 py-2 font-semibold text-foreground last:border-r-0"
        />
      ),
      td: (props: React.HTMLAttributes<HTMLTableCellElement>) => (
        <td
          {...props}
          className="border-t border-r border-border/30 px-3 py-2 text-foreground last:border-r-0"
        />
      ),
      pre: ({ children }: React.HTMLAttributes<HTMLPreElement>) => {
        // pre 内部直接放 code，外层包装由 code 自行渲染 CodeBlock 容器
        return <>{children}</>;
      },
      code: ({
        className: cls,
        children,
        ...rest
      }: React.HTMLAttributes<HTMLElement> & { children?: React.ReactNode }) => {
        const match = /language-(\w+)/.exec(cls || '');
        const isBlock = Boolean(match) || (cls && cls.includes('hljs'));
        const rawCode = String(children || '').replace(/\n$/, '');

        if (isBlock) {
          const lang = match ? match[1] : '';
          return (
            <CodeBlock language={lang} codeText={rawCode}>
              <code {...rest} className={cn(cls, 'block')}>
                {children}
              </code>
            </CodeBlock>
          );
        }
        return (
          <code
            {...rest}
            className="rounded-md bg-surface px-1.5 py-0.5 font-mono text-[0.875em] text-terminal-cyan"
          >
            {children}
          </code>
        );
      },
      input: (props: React.InputHTMLAttributes<HTMLInputElement>) => {
        if (props.type === 'checkbox') {
          return (
            <input
              {...props}
              disabled
              className="mr-1.5 size-3.5 align-middle accent-primary"
            />
          );
        }
        return <input {...props} />;
      },
    }),
    [],
  );

  return (
    <div className={cn('h-full w-full overflow-y-auto px-6 py-5 select-text', className)}>
      <div className="mx-auto max-w-4xl">
        <ReactMarkdown
          remarkPlugins={[remarkGfm]}
          rehypePlugins={[rehypeHighlight]}
          components={components as never}
        >
          {content}
        </ReactMarkdown>
      </div>
    </div>
  );
});
