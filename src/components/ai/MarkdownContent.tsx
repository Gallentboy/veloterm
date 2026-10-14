import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import rehypeHighlight from 'rehype-highlight';
import { memo, useMemo } from 'react';
import { cn } from '@/lib/utils';

/**
 * AI 气泡 Markdown 渲染（参考 ZCode MessageResponse 的样式规格）：
 * - react-markdown + GFM（表格/删除线/任务列表）+ highlight.js 代码高亮
 * - 无气泡背景的 assistant 全宽文本（ZCode 同款：user 有气泡、AI 是平面排版）
 * - 标题/表格/代码块/引用/行内代码全部用主题令牌
 * - memo 避免流式 delta 触发整列表重渲（仅 content 引用变化时重渲）
 */

const MarkdownContent = memo(function MarkdownContent({ content }: { content: string }) {
  const components = useMemo(
    () => ({
      h1: (props: React.HTMLAttributes<HTMLHeadingElement>) => (
        <h1
          {...props}
          className="text-ui-lg font-semibold text-foreground mt-3 mb-1.5 [&:first-child]:mt-0"
        />
      ),
      h2: (props: React.HTMLAttributes<HTMLHeadingElement>) => (
        <h2
          {...props}
          className="text-ui-base font-semibold text-foreground mt-3 mb-1 [&:first-child]:mt-0"
        />
      ),
      h3: (props: React.HTMLAttributes<HTMLHeadingElement>) => (
        <h3
          {...props}
          className="text-ui-base font-medium text-foreground mt-2.5 mb-1 [&:first-child]:mt-0"
        />
      ),
      p: (props: React.HTMLAttributes<HTMLParagraphElement>) => (
        <p {...props} className="leading-[1.7] text-foreground mb-2 last:mb-0" />
      ),
      ul: (props: React.HTMLAttributes<HTMLUListElement>) => (
        <ul {...props} className="list-disc pl-4 mb-2 last:mb-0 space-y-0.5 text-foreground" />
      ),
      ol: (props: React.HTMLAttributes<HTMLOListElement>) => (
        <ol {...props} className="list-decimal pl-4 mb-2 last:mb-0 space-y-0.5 text-foreground" />
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
          className="border-l-2 border-border pl-3 my-2 text-foreground-subtle"
        />
      ),
      a: (props: React.HTMLAttributes<HTMLAnchorElement>) => (
        <a {...props} className="text-terminal-blue hover:underline" target="_blank" rel="noopener" />
      ),
      code: ({
        className: cls,
        children,
        ...rest
      }: React.HTMLAttributes<HTMLElement> & { children?: React.ReactNode }) => {
        const isBlock = typeof cls === 'string' && cls.includes('language-');
        if (isBlock) {
          return (
            <code
              {...rest}
              className={cn(cls, 'block font-mono text-ui-sm leading-[1.6] text-foreground')}
            >
              {children}
            </code>
          );
        }
        return (
          <code
            {...rest}
            className="rounded bg-surface px-1.5 py-0.5 font-mono text-ui-sm text-foreground"
          >
            {children}
          </code>
        );
      },
      pre: (props: React.HTMLAttributes<HTMLPreElement>) => (
        <pre
          {...props}
          className="overflow-x-auto rounded-lg border border-border bg-background p-3 my-2 [&>code]:bg-transparent"
        />
      ),
      table: (props: React.HTMLAttributes<HTMLTableElement>) => (
        <div className="my-2 overflow-x-auto rounded-lg border border-border">
          <table {...props} className="w-full border-collapse text-ui-sm" />
        </div>
      ),
      thead: (props: React.HTMLAttributes<HTMLTableSectionElement>) => (
        <thead {...props} className="bg-surface" />
      ),
      th: (props: React.HTMLAttributes<HTMLTableCellElement>) => (
        <th
          {...props}
          className="border-b border-border px-2.5 py-1.5 text-left font-medium text-foreground-subtle"
        />
      ),
      td: (props: React.HTMLAttributes<HTMLTableCellElement>) => (
        <td
          {...props}
          className="border-b border-border/50 px-2.5 py-1.5 align-top text-foreground last:[&]:border-b-0"
        />
      ),
      hr: (props: React.HTMLAttributes<HTMLHRElement>) => (
        <hr {...props} className="border-border my-3" />
      ),
    }),
    [],
  );

  return (
    <div className="w-full">
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        rehypePlugins={[rehypeHighlight]}
        components={components as never}
      >
        {content}
      </ReactMarkdown>
    </div>
  );
});

export { MarkdownContent };
