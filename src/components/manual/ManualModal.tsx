import {
  BookOpen,
  ChevronDown,
  ChevronRight,
  Eye,
  FileCode2,
  FolderSync,
  Maximize2,
  Minimize2,
  Network,
  PanelLeftClose,
  PanelLeftOpen,
  Search,
  Server,
  ShieldCheck,
  Terminal,
  TerminalSquare,
  Wrench,
  X,
  ArrowLeft,
  ArrowRight,
  Check,
  Copy,
} from 'lucide-react';
import React, {
  memo,
  useCallback,
  useDeferredValue,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from 'react';
import { useTranslation } from 'react-i18next';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import rehypeHighlight from 'rehype-highlight';

import {
  flattenManualSections,
  getManualChapters,
} from '@/manual';
import { cn } from '@/lib/utils';
import { Button } from '@/components/ui/button';
import { ControlHintTooltip } from '@/components/ControlHintTooltip';

interface ManualModalProps {
  open: boolean;
  onClose: () => void;
}

const DEFAULT_SIDEBAR_WIDTH = 280;
const MIN_SIDEBAR_WIDTH = 220;
const MAX_SIDEBAR_WIDTH = 460;

/** 章节图标动态映射 */
function renderChapterIcon(iconName: string, className?: string): ReactNode {
  const iconProps = { className: cn('size-4 shrink-0', className) };
  switch (iconName) {
    case 'Terminal':
      return <Terminal {...iconProps} />;
    case 'FolderSync':
      return <FolderSync {...iconProps} />;
    case 'Eye':
      return <Eye {...iconProps} />;
    case 'FileCode2':
      return <FileCode2 {...iconProps} />;
    case 'Network':
      return <Network {...iconProps} />;
    case 'Server':
      return <Server {...iconProps} />;
    case 'TerminalSquare':
      return <TerminalSquare {...iconProps} />;
    case 'Wrench':
      return <Wrench {...iconProps} />;
    case 'ShieldCheck':
      return <ShieldCheck {...iconProps} />;
    default:
      return <BookOpen {...iconProps} />;
  }
}

/** 手册专用代码块组件 */
function ManualCodeBlock({
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
      <div className="overflow-x-auto p-3 font-mono text-ui-sm leading-relaxed">{children}</div>
    </div>
  );
}

/** 电子书式使用说明全景大弹窗 */
export const ManualModal = memo(function ManualModal({ open, onClose }: ManualModalProps) {
  const { t, i18n } = useTranslation();
  const chapters = useMemo(() => getManualChapters(i18n.language), [i18n.language]);
  const flatSections = useMemo(() => flattenManualSections(chapters), [chapters]);

  // 当前激活的小节与章节 ID
  const [activeSectionId, setActiveSectionId] = useState<string>(() => {
    return chapters[0]?.sections[0]?.id ?? '';
  });
  // 展开折叠的章节 Map（默认全部展开）
  const [expandedChapters, setExpandedChapters] = useState<Record<string, boolean>>(() => {
    const initial: Record<string, boolean> = {};
    for (const ch of chapters) {
      initial[ch.id] = true;
    }
    return initial;
  });

  // 搜索关键字
  const [searchQuery, setSearchQuery] = useState('');
  const deferredSearch = useDeferredValue(searchQuery.trim().toLowerCase());

  // 布局控制：是否全屏、是否收起目录侧栏、侧栏宽度
  const [isFullscreen, setIsFullscreen] = useState(false);
  const [isSidebarVisible, setIsSidebarVisible] = useState(true);
  const [sidebarWidth, setSidebarWidth] = useState(DEFAULT_SIDEBAR_WIDTH);
  const [isResizing, setIsResizing] = useState(false);
  const resizeSessionRef = useRef<{ startX: number; startWidth: number } | null>(null);

  // 正文视口容器 ref，切换小节自动滚顶
  const contentScrollRef = useRef<HTMLDivElement>(null);

  // 确保当前激活小节在有效列表内
  useEffect(() => {
    if (!flatSections.some((item) => item.section.id === activeSectionId)) {
      if (flatSections[0]) {
        setActiveSectionId(flatSections[0].section.id);
      }
    }
  }, [flatSections, activeSectionId]);

  // 切换小节自动回到正文顶部
  const handleSelectSection = useCallback((sectionId: string) => {
    setActiveSectionId(sectionId);
    if (contentScrollRef.current) {
      contentScrollRef.current.scrollTo({ top: 0, behavior: 'instant' });
    }
  }, []);

  // 搜索过滤匹配计算
  const filteredChapters = useMemo(() => {
    if (!deferredSearch) {
      return chapters;
    }
    return chapters
      .map((ch) => {
        const matchedSections = ch.sections.filter((sec) => {
          const matchTitle = sec.title.toLowerCase().includes(deferredSearch);
          const matchKeyword = sec.keywords?.some((kw) => kw.toLowerCase().includes(deferredSearch));
          const matchContent = sec.content.toLowerCase().includes(deferredSearch);
          return matchTitle || matchKeyword || matchContent;
        });
        return {
          ...ch,
          sections: matchedSections,
        };
      })
      .filter((ch) => ch.sections.length > 0);
  }, [chapters, deferredSearch]);

  // 搜索时自动展开所有包含结果的章节
  useEffect(() => {
    if (deferredSearch) {
      const nextExpanded: Record<string, boolean> = {};
      for (const ch of filteredChapters) {
        nextExpanded[ch.id] = true;
      }
      setExpandedChapters(nextExpanded);
    }
  }, [deferredSearch, filteredChapters]);

  // 当前小节与所属章节信息
  const currentItem = useMemo(() => {
    return (
      flatSections.find((item) => item.section.id === activeSectionId) ??
      flatSections[0] ??
      null
    );
  }, [flatSections, activeSectionId]);

  // 上一节 / 下一节
  const prevItem = useMemo(() => {
    if (!currentItem || currentItem.globalIndex <= 0) return null;
    return flatSections[currentItem.globalIndex - 1] ?? null;
  }, [currentItem, flatSections]);

  const nextItem = useMemo(() => {
    if (!currentItem || currentItem.globalIndex >= flatSections.length - 1) return null;
    return flatSections[currentItem.globalIndex + 1] ?? null;
  }, [currentItem, flatSections]);

  // 键盘快捷键监听：Escape 退出、Alt+Left/Right 切页
  useEffect(() => {
    if (!open) return;
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        onClose();
        return;
      }
      if (e.altKey && e.key === 'ArrowLeft' && prevItem) {
        e.preventDefault();
        handleSelectSection(prevItem.section.id);
        return;
      }
      if (e.altKey && e.key === 'ArrowRight' && nextItem) {
        e.preventDefault();
        handleSelectSection(nextItem.section.id);
        return;
      }
    };
    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [open, onClose, prevItem, nextItem, handleSelectSection]);

  // 侧栏拖拽调整宽度
  const handlePointerDown = (e: React.PointerEvent) => {
    if (e.button !== 0) return;
    resizeSessionRef.current = { startX: e.clientX, startWidth: sidebarWidth };
    setIsResizing(true);
    e.currentTarget.setPointerCapture(e.pointerId);
    e.preventDefault();
  };

  const handlePointerMove = (e: React.PointerEvent) => {
    if (!resizeSessionRef.current) return;
    const delta = e.clientX - resizeSessionRef.current.startX;
    const nextWidth = Math.max(
      MIN_SIDEBAR_WIDTH,
      Math.min(MAX_SIDEBAR_WIDTH, resizeSessionRef.current.startWidth + delta),
    );
    setSidebarWidth(nextWidth);
  };

  const handlePointerUp = (e: React.PointerEvent) => {
    if (!resizeSessionRef.current) return;
    resizeSessionRef.current = null;
    setIsResizing(false);
    if (e.currentTarget.hasPointerCapture(e.pointerId)) {
      e.currentTarget.releasePointerCapture(e.pointerId);
    }
  };

  // 折叠单个章节
  const toggleChapter = (chapterId: string) => {
    setExpandedChapters((prev) => ({
      ...prev,
      [chapterId]: !prev[chapterId],
    }));
  };

  // Markdown 渲染组件配置
  const markdownComponents = useMemo(
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
          className="mt-6 mb-3 border-b border-border/40 pb-1.5 text-ui-lg font-semibold tracking-tight text-foreground first:mt-0"
        />
      ),
      h3: (props: React.HTMLAttributes<HTMLHeadingElement>) => (
        <h3
          {...props}
          className="mt-5 mb-2 text-ui-base font-semibold text-primary first:mt-0"
        />
      ),
      h4: (props: React.HTMLAttributes<HTMLHeadingElement>) => (
        <h4 {...props} className="mt-4 mb-1.5 text-ui-base font-medium text-foreground" />
      ),
      p: (props: React.HTMLAttributes<HTMLParagraphElement>) => (
        <p {...props} className="my-2.5 text-ui-sm leading-[1.75] text-foreground" />
      ),
      ul: (props: React.HTMLAttributes<HTMLUListElement>) => (
        <ul {...props} className="my-2 ml-5 list-disc space-y-1 text-ui-sm text-foreground" />
      ),
      ol: (props: React.HTMLAttributes<HTMLOListElement>) => (
        <ol {...props} className="my-2 ml-5 list-decimal space-y-1 text-ui-sm text-foreground" />
      ),
      li: (props: React.HTMLAttributes<HTMLLIElement>) => (
        <li {...props} className="leading-relaxed" />
      ),
      blockquote: (props: React.HTMLAttributes<HTMLQuoteElement>) => (
        <blockquote
          {...props}
          className="my-3 rounded-r-lg border-l-4 border-terminal-blue/80 bg-surface/50 py-2 pr-3 pl-3.5 text-ui-sm italic text-foreground-subtle"
        />
      ),
      code: ({
        inline,
        className,
        children,
        ...props
      }: React.HTMLAttributes<HTMLElement> & { inline?: boolean }) => {
        const match = /language-(\w+)/.exec(className || '');
        const codeText = String(children).replace(/\n$/, '');
        if (!inline && (match || codeText.includes('\n'))) {
          return (
            <ManualCodeBlock language={match ? match[1] : undefined} codeText={codeText}>
              <pre className="m-0 p-0">
                <code className={className} {...props}>
                  {children}
                </code>
              </pre>
            </ManualCodeBlock>
          );
        }
        return (
          <code
            className="rounded-md bg-surface px-1.5 py-0.5 font-mono text-[0.875em] text-terminal-cyan"
            {...props}
          >
            {children}
          </code>
        );
      },
      table: (props: React.HTMLAttributes<HTMLTableElement>) => (
        <div className="my-4 overflow-x-auto rounded-xl border border-border/70">
          <table {...props} className="w-full border-collapse text-left text-ui-sm" />
        </div>
      ),
      thead: (props: React.HTMLAttributes<HTMLTableSectionElement>) => (
        <thead {...props} className="border-b border-border bg-surface text-foreground font-semibold" />
      ),
      tbody: (props: React.HTMLAttributes<HTMLTableSectionElement>) => (
        <tbody {...props} className="divide-y divide-border/40" />
      ),
      tr: (props: React.HTMLAttributes<HTMLTableRowElement>) => (
        <tr {...props} className="transition-colors hover:bg-hover/40" />
      ),
      th: (props: React.ThHTMLAttributes<HTMLTableHeaderCellElement>) => (
        <th {...props} className="px-3.5 py-2 font-medium text-foreground" />
      ),
      td: (props: React.TdHTMLAttributes<HTMLTableDataCellElement>) => (
        <td {...props} className="px-3.5 py-2 text-foreground-subtle" />
      ),
      hr: () => <hr className="my-6 border-border/60" />,
    }),
    [],
  );

  if (!open) {
    return null;
  }

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-labelledby="manual-modal-title"
      className="fixed inset-0 z-[80] flex items-center justify-center bg-black/45 backdrop-blur-[2px] p-3 sm:p-5 animate-in fade-in duration-200 select-none"
      onClick={onClose}
    >
      <div
        className={cn(
          'flex flex-col overflow-hidden border border-border/80 bg-popover text-foreground shadow-2xl transition-[width,height,border-radius] duration-200',
          isFullscreen
            ? // 全屏同样走项目统一圆角（rounded-2xl）：遮罩自带 p-3/p-5 边距，
              // 圆角在边距内完整可见，与常规态及其它弹层观感一致
              'h-full w-full rounded-2xl'
            : 'h-[min(90vh,860px)] w-[min(95vw,1260px)] rounded-2xl',
        )}
        onClick={(e) => e.stopPropagation()}
      >
        {/* 顶部 Header 导航条 */}
        <header className="flex h-12 shrink-0 items-center justify-between border-b border-border/60 bg-surface/40 px-3.5">
          <div className="flex min-w-0 items-center gap-2">
            <div className="flex size-7 items-center justify-center rounded-lg bg-primary/10 text-primary">
              <BookOpen className="size-4" />
            </div>
            <span id="manual-modal-title" className="font-semibold text-ui-base text-foreground">
              {t('help.manualTitle', 'VeloTerm 使用说明')}
            </span>
            <span className="hidden rounded-full border border-border/60 bg-surface px-2 py-0.5 text-ui-xs font-mono text-foreground-subtle sm:inline-block">
              {flatSections.length} {t('help.sectionsCount', '个功能点')}
            </span>
            {currentItem ? (
              <div className="hidden items-center gap-1.5 text-ui-xs text-foreground-subtle md:flex ml-3 border-l border-border/60 pl-3">
                <span className="truncate max-w-[120px]">{currentItem.chapter.title}</span>
                <span className="text-foreground-subtlest">/</span>
                <span className="truncate max-w-[200px] text-foreground font-medium">
                  {currentItem.section.title}
                </span>
              </div>
            ) : null}
          </div>

          <div className="flex items-center gap-1">
            <ControlHintTooltip
              title={
                isSidebarVisible
                  ? t('help.collapseSidebar', '收起目录')
                  : t('help.expandSidebar', '展开目录')
              }
              side="bottom"
            >
              <Button
                variant="ghost"
                size="icon-md"
                aria-label={t('help.toggleSidebar', '切换目录展示')}
                onClick={() => setIsSidebarVisible((v) => !v)}
              >
                {isSidebarVisible ? (
                  <PanelLeftClose className="size-4" />
                ) : (
                  <PanelLeftOpen className="size-4" />
                )}
              </Button>
            </ControlHintTooltip>

            <ControlHintTooltip
              title={
                isFullscreen
                  ? t('help.exitFullscreen', '还原窗口')
                  : t('help.enterFullscreen', '全屏阅读')
              }
              side="bottom"
            >
              <Button
                variant="ghost"
                size="icon-md"
                aria-label={t('help.toggleFullscreen', '切换全屏')}
                onClick={() => setIsFullscreen((v) => !v)}
              >
                {isFullscreen ? (
                  <Minimize2 className="size-4" />
                ) : (
                  <Maximize2 className="size-4" />
                )}
              </Button>
            </ControlHintTooltip>

            <ControlHintTooltip title={t('common.close', '关闭 (ESC)')} side="bottom">
              <Button
                variant="ghost"
                size="icon-md"
                aria-label={t('common.close', '关闭')}
                onClick={onClose}
              >
                <X className="size-4" />
              </Button>
            </ControlHintTooltip>
          </div>
        </header>

        {/* 电子书主视窗：左侧章节目录 + 分割条 + 右侧正文 */}
        <div className="relative flex min-h-0 flex-1 overflow-hidden">
          {/* 左侧边栏：章节功能点目录树 */}
          {isSidebarVisible ? (
            <aside
              style={{ width: `${sidebarWidth}px` }}
              className="flex shrink-0 flex-col border-r border-border/50 bg-background/50"
            >
              {/* 搜索框 */}
              <div className="border-b border-border/50 p-2.5">
                <div className="relative flex items-center">
                  <Search className="pointer-events-none absolute left-2.5 size-3.5 text-foreground-subtlest" />
                  <input
                    type="text"
                    value={searchQuery}
                    onChange={(e) => setSearchQuery(e.target.value)}
                    placeholder={t('help.searchPlaceholder', '搜索功能、命令或步骤...')}
                    className="h-8 w-full rounded-md border border-border/70 bg-surface/70 pr-7 pl-8 text-ui-xs text-foreground placeholder:text-foreground-subtlest focus:border-border focus:bg-surface focus:outline-none"
                  />
                  {searchQuery ? (
                    <button
                      type="button"
                      onClick={() => setSearchQuery('')}
                      className="absolute right-2 flex size-4 items-center justify-center rounded text-foreground-subtle hover:text-foreground"
                    >
                      <X className="size-3" />
                    </button>
                  ) : null}
                </div>
              </div>

              {/* 章节与小节列表 */}
              <div className="scrollbar-slim min-h-0 flex-1 overflow-y-auto p-2 space-y-1.5">
                {filteredChapters.length === 0 ? (
                  <div className="py-8 text-center text-ui-xs text-foreground-subtle">
                    {t('help.noResults', '未找到匹配的功能点')}
                  </div>
                ) : (
                  filteredChapters.map((chapter) => {
                    const isExpanded = expandedChapters[chapter.id] ?? true;
                    return (
                      <div key={chapter.id} className="space-y-0.5">
                        {/* 一级章节标题条 */}
                        <button
                          type="button"
                          onClick={() => toggleChapter(chapter.id)}
                          className="flex w-full items-center justify-between rounded-lg px-2 py-1.5 text-left text-ui-xs font-semibold text-foreground-subtle transition-colors hover:bg-hover hover:text-foreground"
                        >
                          <div className="flex min-w-0 items-center gap-1.5">
                            {renderChapterIcon(chapter.icon, 'text-foreground-subtle')}
                            <span className="truncate">{chapter.title}</span>
                          </div>
                          <div className="flex items-center gap-1">
                            <span className="rounded-full bg-surface px-1.5 py-0.2 font-mono text-[10px] text-foreground-subtlest">
                              {chapter.sections.length}
                            </span>
                            {isExpanded ? (
                              <ChevronDown className="size-3.5 text-foreground-subtlest" />
                            ) : (
                              <ChevronRight className="size-3.5 text-foreground-subtlest" />
                            )}
                          </div>
                        </button>

                        {/* 二级功能点列表 */}
                        {isExpanded ? (
                          <div className="space-y-0.5 pl-2">
                            {chapter.sections.map((section) => {
                              const isSelected = section.id === activeSectionId;
                              return (
                                <button
                                  key={section.id}
                                  type="button"
                                  onClick={() => handleSelectSection(section.id)}
                                  className={cn(
                                    'flex w-full items-center justify-between rounded-md px-2 py-1.5 text-left text-ui-xs transition-colors',
                                    isSelected
                                      ? 'bg-selected font-medium text-foreground shadow-2xs'
                                      : 'text-foreground-subtle hover:bg-hover hover:text-foreground',
                                  )}
                                >
                                  <span className="truncate pr-1">{section.title}</span>
                                  {section.badge ? (
                                    <span
                                      className={cn(
                                        'shrink-0 rounded px-1.5 py-0.2 font-mono text-[9px] uppercase tracking-wider',
                                        isSelected
                                          ? 'bg-primary/20 text-primary'
                                          : 'bg-surface text-foreground-subtlest',
                                      )}
                                    >
                                      {section.badge}
                                    </span>
                                  ) : null}
                                </button>
                              );
                            })}
                          </div>
                        ) : null}
                      </div>
                    );
                  })
                )}
              </div>
            </aside>
          ) : null}

          {/* 侧栏拖拽手柄 */}
          {isSidebarVisible ? (
            <div
              role="separator"
              aria-orientation="vertical"
              tabIndex={0}
              onPointerDown={handlePointerDown}
              onPointerMove={handlePointerMove}
              onPointerUp={handlePointerUp}
              onPointerCancel={handlePointerUp}
              className={cn(
                'group/handle relative z-10 flex h-full w-1 shrink-0 cursor-ew-resize touch-none items-center justify-center bg-transparent outline-none',
                "after:pointer-events-none after:absolute after:w-0.5 after:rounded-full after:bg-foreground-subtlest/50 after:opacity-0 after:transition-opacity after:content-[''] after:inset-y-4",
                'hover:after:opacity-100 focus-visible:after:opacity-100',
                isResizing && 'after:opacity-100',
              )}
            />
          ) : null}

          {/* 右侧正文排版区 */}
          <main
            ref={contentScrollRef}
            className="scrollbar-slim flex-1 overflow-y-auto bg-popover select-text"
          >
            {currentItem ? (
              <div className="mx-auto max-w-4xl px-6 py-8 sm:px-10">
                {/* 顶部面包屑与章节标签 */}
                <div className="mb-6 flex flex-wrap items-center gap-2 border-b border-border/50 pb-4 select-none">
                  <span className="flex items-center gap-1.5 rounded-md bg-surface px-2.5 py-1 text-ui-xs font-medium text-foreground-subtle">
                    {renderChapterIcon(currentItem.chapter.icon, 'size-3.5')}
                    {currentItem.chapter.title}
                  </span>
                  <span className="text-foreground-subtlest">/</span>
                  <span className="text-ui-xs font-semibold text-foreground">
                    {currentItem.section.title}
                  </span>
                  {currentItem.section.badge ? (
                    <span className="rounded bg-primary/10 px-2 py-0.5 font-mono text-[10px] font-semibold text-primary uppercase">
                      {currentItem.section.badge}
                    </span>
                  ) : null}
                </div>

                {/* 章节标题 */}
                <div className="mb-6">
                  <h1 className="text-2xl font-bold tracking-tight text-foreground sm:text-3xl">
                    {currentItem.section.title}
                  </h1>
                </div>

                {/* Markdown 正文 */}
                <div className="prose prose-invert max-w-none">
                  <ReactMarkdown
                    remarkPlugins={[remarkGfm]}
                    rehypePlugins={[rehypeHighlight]}
                    components={markdownComponents}
                  >
                    {currentItem.section.content}
                  </ReactMarkdown>
                </div>

                {/* 底部分页导航卡片 */}
                <div className="mt-12 flex flex-col gap-3 border-t border-border/60 pt-6 sm:flex-row sm:items-center sm:justify-between select-none">
                  {prevItem ? (
                    <button
                      type="button"
                      onClick={() => handleSelectSection(prevItem.section.id)}
                      className="group flex flex-1 items-center gap-3 rounded-xl border border-border/70 bg-surface/40 p-3.5 text-left transition-colors hover:border-border hover:bg-hover"
                    >
                      <ArrowLeft className="size-4 shrink-0 text-foreground-subtle transition-transform group-hover:-translate-x-0.5 group-hover:text-foreground" />
                      <div className="min-w-0">
                        <div className="text-[11px] font-medium text-foreground-subtle">
                          {t('help.prevSection', '上一节')} (Alt+←)
                        </div>
                        <div className="truncate text-ui-sm font-semibold text-foreground">
                          {prevItem.section.title}
                        </div>
                      </div>
                    </button>
                  ) : (
                    <div className="flex-1" />
                  )}

                  {nextItem ? (
                    <button
                      type="button"
                      onClick={() => handleSelectSection(nextItem.section.id)}
                      className="group flex flex-1 items-center justify-end gap-3 rounded-xl border border-border/70 bg-surface/40 p-3.5 text-right transition-colors hover:border-border hover:bg-hover"
                    >
                      <div className="min-w-0">
                        <div className="text-[11px] font-medium text-foreground-subtle">
                          {t('help.nextSection', '下一节')} (Alt+→)
                        </div>
                        <div className="truncate text-ui-sm font-semibold text-foreground">
                          {nextItem.section.title}
                        </div>
                      </div>
                      <ArrowRight className="size-4 shrink-0 text-foreground-subtle transition-transform group-hover:translate-x-0.5 group-hover:text-foreground" />
                    </button>
                  ) : (
                    <div className="flex-1" />
                  )}
                </div>
              </div>
            ) : null}
          </main>
        </div>
      </div>
    </div>
  );
});
