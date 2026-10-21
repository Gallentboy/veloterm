import { useCallback, useEffect, useMemo, useRef, useState, type CSSProperties } from 'react';
import { Loader2, Maximize2, Minus, Plus, RotateCcw } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { Button } from '@/components/ui/button';
import { cn } from '@/lib/utils';
import { prepareImagePreview } from '@/services/imagePreviewService';

/** 缩放步进（每次点击 +/- 的倍率增量） */
const ZOOM_STEP = 0.25;
const ZOOM_MIN = 0.1;
const ZOOM_MAX = 8;

/**
 * 重置字号 token 为基准值（与 PdfPreview 同款）。
 *
 * SFTP 面板的缩放功能在根节点覆盖 `--text-ui-*`，弹窗作为子树会继承；
 * 图片预览是文档阅读界面，工具栏不该随面板缩放变化。
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

export interface ImagePreviewProps {
  /** 本地图片绝对路径（远端文件需先下载到临时目录） */
  imagePath: string;
  className?: string;
}

/**
 * 图片预览：`<img>` 展示 + 工具栏缩放（按钮，不用手势）+ 双向滚动。
 *
 * 性能设计：
 * - 原生格式零转码（WebView 自己解码，GPU 加速、流式读取）
 * - 非原生格式由后端转 PNG 一次（结果缓存在临时文件，同图重开不再转）
 * - 图片按缩放倍率设置 CSS 尺寸，超出容器时外层 overflow-auto 双向滚动
 */
export function ImagePreview({ imagePath, className }: ImagePreviewProps) {
  const { t } = useTranslation();
  const [url, setUrl] = useState<string | null>(null);
  const [naturalSize, setNaturalSize] = useState<{ width: number; height: number } | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  /** 缩放倍率（1 = 适应容器宽度的初始大小） */
  const [zoom, setZoom] = useState(1);
  const scrollRef = useRef<HTMLDivElement | null>(null);

  // 加载：准备预览（原生直传 / 后端转码）
  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);
    setUrl(null);
    setZoom(1);
    void prepareImagePreview(imagePath)
      .then((result) => {
        if (cancelled) return;
        setUrl(result.url);
        setNaturalSize(
          result.width > 0 && result.height > 0
            ? { width: result.width, height: result.height }
            : null,
        );
      })
      .catch((e) => {
        if (cancelled) return;
        setError(e instanceof Error ? e.message : String(e));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [imagePath]);

  const zoomBy = useCallback((delta: number) => {
    setZoom((z) => Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, Number((z + delta).toFixed(2)))));
  }, []);

  const resetZoom = useCallback(() => setZoom(1), []);

  /** 适应容器：按容器可用宽度算出刚好铺满的倍率 */
  const fitToWidth = useCallback(() => {
    const container = scrollRef.current;
    if (!container || !naturalSize) return;
    // 容器 padding 32px（p-4 两侧）
    const available = container.clientWidth - 32;
    if (available <= 0 || naturalSize.width <= 0) return;
    setZoom(Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, Number((available / naturalSize.width).toFixed(2)))));
  }, [naturalSize]);

  /** 当前显示尺寸：按缩放倍率直接算 px，避免 CSS transform 缩放带来的模糊 */
  const displaySize = useMemo(() => {
    if (!naturalSize) return null;
    return {
      width: Math.round(naturalSize.width * zoom),
      height: Math.round(naturalSize.height * zoom),
    };
  }, [naturalSize, zoom]);

  const zoomPercent = Math.round(zoom * 100);

  return (
    <div
      className={cn('flex min-h-0 flex-1 flex-col', className)}
      style={RESET_FONT_SCALE_STYLE}
    >
      {/* 工具栏：缩放控制（按钮，不依赖手势） */}
      <div className="flex shrink-0 items-center gap-2 border-b border-border/50 px-3 py-1.5">
        <span className="flex flex-1 items-center gap-1.5 font-mono text-ui-xs text-foreground-subtle select-none">
          {naturalSize ? `${naturalSize.width} × ${naturalSize.height}` : ''}
        </span>
        <div className="flex items-center gap-1">
          <Button
            variant="outline"
            size="lg"
            className="h-7 w-7 p-0"
            disabled={loading || !!error || zoom <= ZOOM_MIN}
            onClick={() => zoomBy(-ZOOM_STEP)}
            aria-label={t('image.zoomOut')}
            title={t('image.zoomOut')}
          >
            <Minus className="size-3.5" />
          </Button>
          <span className="w-12 text-center font-mono text-ui-xs text-foreground select-none">
            {zoomPercent}%
          </span>
          <Button
            variant="outline"
            size="lg"
            className="h-7 w-7 p-0"
            disabled={loading || !!error || zoom >= ZOOM_MAX}
            onClick={() => zoomBy(ZOOM_STEP)}
            aria-label={t('image.zoomIn')}
            title={t('image.zoomIn')}
          >
            <Plus className="size-3.5" />
          </Button>
          <Button
            variant="outline"
            size="lg"
            className="h-7 gap-1 px-2 text-ui-xs"
            disabled={loading || !!error}
            onClick={fitToWidth}
            title={t('image.fitWidth')}
          >
            <Maximize2 className="size-3.5" />
            {t('image.fitWidth')}
          </Button>
          <Button
            variant="outline"
            size="lg"
            className="h-7 gap-1 px-2 text-ui-xs"
            disabled={loading || !!error || zoom === 1}
            onClick={resetZoom}
            title={t('image.actualSize')}
          >
            <RotateCcw className="size-3.5" />
            {t('image.actualSize')}
          </Button>
        </div>
      </div>

      {/* 图片区域：按缩放后的实际像素尺寸显示，超出容器双向滚动。
          居中用 mx-auto（仅水平）：内容比容器宽时左右都能滚到；
          垂直从顶部开始（items-start），图片比容器高时顶部不会被裁。 */}
      <div
        ref={scrollRef}
        className="relative flex min-h-0 flex-1 items-start overflow-auto bg-background-alt/40 p-4"
      >
        {loading && !error ? (
          <div className="absolute inset-0 z-20 flex flex-col items-center justify-center gap-2 bg-background-alt/70">
            <Loader2 className="size-6 animate-spin text-foreground-subtle" />
            <span className="text-ui-xs text-foreground-subtlest select-none">
              {t('image.loading')}
            </span>
          </div>
        ) : null}
        {error ? (
          <div className="m-auto flex flex-col items-center gap-2 py-8">
            <span className="max-w-md text-center text-ui-sm text-destructive">{error}</span>
          </div>
        ) : (
          <div className="mx-auto">
            {url ? (
              <img
                src={url}
                alt={t('image.alt')}
                // 按缩放倍率设实际像素尺寸（不用 CSS transform：避免缩放模糊）
                style={
                  displaySize
                    ? { width: `${displaySize.width}px`, height: `${displaySize.height}px` }
                    : undefined
                }
                className="shrink-0 rounded-md border border-border/60 shadow-sm"
                draggable={false}
                // 原生格式的图可能很大，交给浏览器按显示尺寸解码
                decoding="async"
              />
            ) : null}
          </div>
        )}
      </div>
    </div>
  );
}
