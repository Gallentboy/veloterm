import { useCallback, useEffect, useRef, useState, type CSSProperties } from 'react';
import { ChevronLeft, ChevronRight } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { Button } from '@/components/ui/button';
import { Spinner } from '@/components/ui/spinner';
import { cn } from '@/lib/utils';
import { renderPdfPage } from '@/services/pdfPreviewService';

/** 翻页防抖：连续点击时只渲染最后一次（避免把 PDFium 阻塞池打满） */
const PAGE_CHANGE_DEBOUNCE_MS = 120;

/**
 * 重置字号 token 为基准值。
 *
 * SFTP 面板在根节点用内联 style 覆盖 `--text-ui-*`（缩放功能），弹窗作为
 * 子树会继承——但 PDF 预览是文档阅读界面，工具栏与页码不该随面板缩放变化。
 * 在预览根节点写回基准值即隔离（内联 style 优先级高于继承值）。
 */
const UI_FONT_BASE_PX = 14;
const RESET_FONT_SCALE_STYLE = {
  '--text-ui-xl': `${UI_FONT_BASE_PX + 4}px`,
  '--text-ui-lg': `${UI_FONT_BASE_PX + 2}px`,
  '--text-ui-base': `${UI_FONT_BASE_PX}px`,
  '--text-ui-caption': `${UI_FONT_BASE_PX - 1}px`,
  '--text-ui-sm': `${UI_FONT_BASE_PX - 2}px`,
  '--text-ui-xs': `${UI_FONT_BASE_PX - 4}px`,
} as CSSProperties;

export interface PdfPreviewProps {
  /** 本地 PDF 绝对路径（远端文件需先下载到临时目录） */
  pdfPath: string;
  /**
   * 渲染出的 PNG 临时文件路径上报（每渲染一页回调一次）。
   * 父组件据此在卸载时释放这些文件——否则它们只能等应用退出才清理。
   */
  onRenderedFile?: (path: string) => void;
  className?: string;
}

/**
 * PDF 预览：后端 PDFium 渲染当前页为 PNG，`<img>` 展示 + 翻页。
 *
 * 性能设计：
 * - 后端 spawn_blocking 渲染，不占 tokio worker、不阻塞 UI
 * - 前端翻页防抖 120ms：快速连点时丢弃中间请求，只渲染最终目标页
 * - 请求代次（generation）机制：每次翻页自增，返回时若代次已过期则丢弃结果，
 *   避免慢请求覆盖新页面（竞态）
 * - 渲染结果按页缓存于内存（同一页来回翻不重复渲染）
 */
export function PdfPreview({ pdfPath, onRenderedFile, className }: PdfPreviewProps) {
  const { t } = useTranslation();
  const [pageIndex, setPageIndex] = useState(0);
  const [pageCount, setPageCount] = useState<number | null>(null);
  const [imageUrl, setImageUrl] = useState<string | null>(null);
  const [imageSize, setImageSize] = useState<{ width: number; height: number } | null>(null);
  const [loading, setLoading] = useState(true);
  /**
   * 跳页输入草稿：`null` = 跟随当前页码显示（翻页时自动同步）；
   * 字符串 = 用户正在编辑（不被打断）。
   */
  const [jumpDraft, setJumpDraft] = useState<string | null>(null);
  /** 输入框实际显示值：未编辑时始终是当前页码 */
  const jumpValue = jumpDraft ?? String(pageIndex + 1);
  const [error, setError] = useState<string | null>(null);

  /** 请求代次：只有最新代次的响应才被采纳 */
  const generationRef = useRef(0);
  /** 已渲染页缓存（pageIndex → url），避免来回翻页重复渲染 */
  const cacheRef = useRef<Map<number, { url: string; width: number; height: number }>>(
    new Map(),
  );
  const debounceRef = useRef<number | null>(null);

  const loadPage = useCallback(
    async (index: number) => {
      const cached = cacheRef.current.get(index);
      if (cached) {
        setImageUrl(cached.url);
        setImageSize({ width: cached.width, height: cached.height });
        setLoading(false);
        setError(null);
        return;
      }

      const generation = ++generationRef.current;
      setLoading(true);
      setError(null);
      try {
        const result = await renderPdfPage(pdfPath, index);
        // 代次已过期（用户已翻到别的页）：丢弃这次结果
        if (generation !== generationRef.current) return;
        cacheRef.current.set(index, {
          url: result.url,
          width: result.width,
          height: result.height,
        });
        // 上报临时文件路径：父组件卸载时统一释放（否则只能等应用退出清理）
        onRenderedFile?.(result.path);
        setPageCount(result.pageCount);
        setImageUrl(result.url);
        setImageSize({ width: result.width, height: result.height });
      } catch (e) {
        if (generation !== generationRef.current) return;
        setError(e instanceof Error ? e.message : String(e));
      } finally {
        if (generation === generationRef.current) setLoading(false);
      }
    },
    [pdfPath, onRenderedFile],
  );

  // 首次加载 / 路径变化：重置状态并渲染第 0 页
  useEffect(() => {
    cacheRef.current.clear();
    setPageIndex(0);
    setPageCount(null);
    setImageUrl(null);
    setImageSize(null);
    void loadPage(0);
    return () => {
      if (debounceRef.current != null) window.clearTimeout(debounceRef.current);
      // 使在途请求的结果失效
      generationRef.current += 1;
    };
  }, [loadPage]);

  /** 翻页：更新页码后防抖渲染（快速连点只渲染最后一页） */
  const goToPage = useCallback(
    (next: number) => {
      setPageIndex((current) => {
        const max = (pageCount ?? 1) - 1;
        const clamped = Math.max(0, Math.min(max, next));
        if (clamped === current) return current;
        if (debounceRef.current != null) window.clearTimeout(debounceRef.current);
        debounceRef.current = window.setTimeout(() => {
          void loadPage(clamped);
        }, PAGE_CHANGE_DEBOUNCE_MS);
        return clamped;
      });
    },
    [pageCount, loadPage],
  );

  const canPrev = pageIndex > 0;
  const canNext = pageCount != null && pageIndex < pageCount - 1;

  /**
   * 跳页提交：接受 1 基页码，越界钳制到 [1, pageCount]（goToPage 内部处理）。
   * 提交后清空草稿 → 输入框回到「跟随当前页」状态。
   */
  const commitJump = useCallback(() => {
    const raw = jumpDraft?.trim() ?? '';
    setJumpDraft(null);
    if (!raw || pageCount == null) return;
    const parsed = Number.parseInt(raw, 10);
    if (!Number.isFinite(parsed)) return;
    goToPage(parsed - 1);
  }, [jumpDraft, pageCount, goToPage]);

  return (
    <div
      className={cn('flex min-h-0 flex-1 flex-col', className)}
      style={RESET_FONT_SCALE_STYLE}
    >
      {/* 工具栏：左页码（固定宽度不随缩放/内容变）｜中间跳页输入（严格居中）｜右翻页按钮 */}
      <div className="flex shrink-0 items-center gap-2 border-b border-border/50 px-3 py-1.5">
        <div className="flex flex-1 items-center">
          <span className="font-mono text-ui-xs text-foreground-subtle select-none">
            {pageCount != null ? `${pageIndex + 1} / ${pageCount}` : t('pdf.loading')}
          </span>
        </div>
        {pageCount != null ? (
          <label className="flex items-center gap-1.5 text-ui-sm text-foreground-subtle select-none">
            <span>{t('pdf.jumpTo')}</span>
            <input
              type="number"
              min={1}
              max={pageCount}
              step={1}
              value={jumpValue}
              onChange={(e) => {
                // 记录草稿（不即时钳制：改写 value 会打断上下按钮连点）。
                // 越界值在回车/失焦时由 commitJump 经 goToPage 钳制。
                setJumpDraft(e.target.value);
              }}
              onKeyDown={(e) => {
                if (e.key === 'Enter') {
                  e.preventDefault();
                  commitJump();
                } else if (e.key === 'Escape') {
                  setJumpDraft(null);
                  e.currentTarget.blur();
                }
              }}
              onBlur={() => {
                // 有草稿才提交；无草稿（跟随态）失焦不做事
                if (jumpDraft !== null) commitJump();
              }}
              aria-label={t('pdf.jumpToPage')}
              // 宽度按总页数位数动态给，再放宽 4 倍留出点击与输入余量；
              // 额外留 1.5ch 给右侧原生的上下微调按钮
              style={{ width: `calc(${Math.max(2, String(pageCount).length) * 4}ch + 1.5ch)` }}
              className="h-6 rounded-md border border-border bg-input px-1.5 text-center font-mono text-ui-xs text-foreground outline-none transition-colors placeholder:text-foreground-subtlest focus:border-border-hover"
            />
            {/* 后缀（中文「页」/ 英文空）——空串时不渲染，避免多余空格 */}
            {t('pdf.jumpToSuffix') ? <span>{t('pdf.jumpToSuffix')}</span> : null}
          </label>
        ) : (
          <span className="flex-1" />
        )}
        <div className="flex flex-1 items-center justify-end gap-2">
          <Button
            variant="outline"
            size="lg"
            className="h-7 gap-1 px-2 text-ui-xs"
            disabled={!canPrev || loading}
            onClick={() => goToPage(pageIndex - 1)}
          >
            <ChevronLeft className="size-3.5" />
            {t('pdf.prevPage')}
          </Button>
          <Button
            variant="outline"
            size="lg"
            className="h-7 gap-1 px-2 text-ui-xs"
            disabled={!canNext || loading}
            onClick={() => goToPage(pageIndex + 1)}
          >
            {t('pdf.nextPage')}
            <ChevronRight className="size-3.5" />
          </Button>
        </div>
      </div>

      {/* 页面区域：图片按自然尺寸显示，超出容器时双向滚动。
          居中用 mx-auto（仅水平）：内容比容器宽时左右都能滚到；垂直方向
          从顶部开始（items-start），图片比容器高时顶部不会被裁。 */}
      <div className="relative flex min-h-0 flex-1 items-start overflow-auto bg-background-alt/40 p-4">
        {/* 加载遮罩：覆盖整个页面区域（不是图片容器——首帧渲染时图片容器
            尺寸为 0，绝对定位的 spinner 会不可见）。spinner + 文案居中。 */}
        {loading && !error ? (
          <div className="absolute inset-0 z-20 flex flex-col items-center justify-center gap-2 bg-background-alt/70">
            <Spinner className="size-6 text-foreground-subtle" />
            <span className="text-ui-xs text-foreground-subtlest select-none">
              {t('pdf.rendering')}
            </span>
          </div>
        ) : null}
        {error ? (
          <div className="m-auto flex flex-col items-center gap-2 py-8">
            <span className="max-w-md text-center text-ui-sm text-destructive">{error}</span>
            <Button
              variant="outline"
              size="lg"
              className="h-7 px-3 text-ui-xs"
              onClick={() => void loadPage(pageIndex)}
            >
              {t('pdf.retry')}
            </Button>
          </div>
        ) : (
          <div className="relative mx-auto">
            {imageUrl ? (
              <img
                src={imageUrl}
                alt={t('pdf.pageAlt', { page: pageIndex + 1 })}
                // 用后端返回的真实像素尺寸占位，避免加载过程中布局抖动
                width={imageSize?.width}
                height={imageSize?.height}
                // 不设 max-w-full：按 1600px 自然尺寸渲染，超出容器时由外层
                // overflow-auto 出滚动条（PDF 阅读需要能看清细节，压缩到容器
                // 宽度会让小字号不可读）
                className="shrink-0 rounded-md border border-border/60 bg-white shadow-sm"
                draggable={false}
              />
            ) : null}
          </div>
        )}
      </div>
    </div>
  );
}
