import { useCallback, useEffect, useRef, useState } from 'react';
import { ChevronLeft, ChevronRight, Loader2 } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { Button } from '@/components/ui/button';
import { cn } from '@/lib/utils';
import { renderPdfPage } from '@/services/pdfPreviewService';

/** 翻页防抖：连续点击时只渲染最后一次（避免把 PDFium 阻塞池打满） */
const PAGE_CHANGE_DEBOUNCE_MS = 120;

export interface PdfPreviewProps {
  /** 本地 PDF 绝对路径（远端文件需先下载到临时目录） */
  pdfPath: string;
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
export function PdfPreview({ pdfPath, className }: PdfPreviewProps) {
  const { t } = useTranslation();
  const [pageIndex, setPageIndex] = useState(0);
  const [pageCount, setPageCount] = useState<number | null>(null);
  const [imageUrl, setImageUrl] = useState<string | null>(null);
  const [imageSize, setImageSize] = useState<{ width: number; height: number } | null>(null);
  const [loading, setLoading] = useState(true);
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
    [pdfPath],
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

  return (
    <div className={cn('flex min-h-0 flex-1 flex-col', className)}>
      {/* 工具栏：页码 + 翻页 */}
      <div className="flex shrink-0 items-center gap-2 border-b border-border/50 px-3 py-1.5">
        <span className="text-ui-sm text-foreground-subtle select-none">
          {pageCount != null
            ? t('pdf.pageIndicator', {
                current: pageIndex + 1,
                total: pageCount,
              })
            : t('pdf.loading')}
        </span>
        <div className="flex-1" />
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

      {/* 页面区域：图片按自然尺寸显示，超出容器时双向滚动。
          居中用 mx-auto（仅水平）：内容比容器宽时左右都能滚到；垂直方向
          从顶部开始（items-start），图片比容器高时顶部不会被裁。 */}
      <div className="flex min-h-0 flex-1 items-start overflow-auto bg-background-alt/40 p-4">
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
            {loading ? (
              <div className="absolute inset-0 z-10 flex items-center justify-center bg-background/60">
                <Loader2 className="size-5 animate-spin text-foreground-subtle" />
              </div>
            ) : null}
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
