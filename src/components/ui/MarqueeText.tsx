import { useRef } from 'react';
import { cn } from '@/lib/utils';

interface MarqueeTextProps {
  text: string;
  className?: string;
  /** 追加到原生提示后的附加信息（如重试次数），不参与走马灯正文 */
  titleSuffix?: string;
}

/**
 * 截断文本 + 悬停走马灯（超出容器宽度时滚动展示全文）：
 * - 平时以省略号截断；悬停时测量溢出距离，>1px 才启动滚动动画（起止各留停顿便于阅读）；
 * - 滚动速度约 60px/s（最短 3s），移开即复位；全程保留 `title` 原生提示作降级；
 * - 悬停路径直接操作 DOM（不进 React 状态），高频触发零重渲；
 * - `prefers-reduced-motion` 下禁用动画（见 styles.css），仅保留 title。
 */
export function MarqueeText({ text, className, titleSuffix }: MarqueeTextProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const innerRef = useRef<HTMLSpanElement>(null);

  const start = () => {
    const container = containerRef.current;
    const inner = innerRef.current;
    if (!container || !inner) return;
    const distance = inner.scrollWidth - container.clientWidth;
    if (distance <= 1) return;
    container.style.setProperty('--marquee-distance', `${-distance}px`);
    container.style.setProperty('--marquee-duration', `${Math.max(3, distance / 60)}s`);
    container.classList.add('marquee-active');
  };

  const stop = () => {
    const container = containerRef.current;
    if (!container) return;
    container.classList.remove('marquee-active');
    container.style.removeProperty('--marquee-distance');
    container.style.removeProperty('--marquee-duration');
  };

  return (
    <div
      ref={containerRef}
      className={cn('marquee-text min-w-0', className)}
      onMouseEnter={start}
      onMouseLeave={stop}
      title={titleSuffix ? `${text} ${titleSuffix}` : text}
    >
      <span ref={innerRef} className="marquee-text-inner inline-block">
        {text}
      </span>
    </div>
  );
}
