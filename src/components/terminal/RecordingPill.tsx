/**
 * 终端录制指示胶囊 + 停止后的操作面板。
 *
 * 录制中：左上角浮动胶囊（红点呼吸 + REC mm:ss 计时；搜索栏在右上、ZMODEM
 * 提示在顶部居中，左上无占用）。点击胶囊弹出操作面板（portal 到 body +
 * fixed，useAnchoredPanel 视口夹取，useDismissOnOutsidePointer 多 ref 排除
 * 触发器与弹层）——停止并预览 / 停止并导出 .cast / 停止并导出 .log /
 * 停止并导出 GIF（后端 agg 渲染，秒级耗时：面板启动即关，进度反馈走该
 * 终端的全屏 busy 遮罩，见 TerminalSession）/ 停止并导出 MP4（先渲染
 * 临时 GIF 再转码，两阶段进度同走 busy 遮罩）/ 继续录制。停止后胶囊以
 * 「已停止」形态保留 3s 作面板锚点：期间点开面板则取消自动隐藏（面板
 * 关闭即卸载）；超时自动淡出隐藏，上次录制仍可从终端右键菜单
 * 「回放上次录制」进入。
 */

import { CircleStop, Eye, FileDown, FileText, Film, Play, Video } from 'lucide-react';
import { useEffect, useRef, useState, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { useTranslation } from 'react-i18next';

import { useAnchoredPanel } from '@/hooks/useAnchoredPanel';
import { useDismissOnOutsidePointer } from '@/hooks/useDismissOnOutsidePointer';
import { cn } from '@/lib/utils';

export function formatRecordElapsed(ms: number): string {
  const totalSec = Math.floor(ms / 1000);
  const h = Math.floor(totalSec / 3600);
  const m = Math.floor((totalSec % 3600) / 60);
  const s = totalSec % 60;
  const mm = String(m).padStart(2, '0');
  const ss = String(s).padStart(2, '0');
  return h > 0 ? `${h}:${mm}:${ss}` : `${mm}:${ss}`;
}

interface RecordingPillProps {
  /** 录制进行中（false = 已停止、仅作操作面板锚点） */
  active: boolean;
  elapsedMs: number;
  onPreview: () => void;
  onExportCast: () => void;
  onExportLog: () => void;
  /** 导出 GIF（后端 agg 渲染，秒级耗时，反馈走该终端的全屏 busy 遮罩） */
  onExportGif: () => Promise<boolean>;
  /** 导出 MP4（两阶段链：渲染临时 GIF → libgif2mp4 转码，反馈同上） */
  onExportMp4: () => Promise<boolean>;
  onResume: () => void;
  /** 面板已关且未在录制，或停止后 3s 超时：父组件卸载胶囊（已停止形态仅作为面板锚点存在） */
  onIdleClose: () => void;
}

/** 已停止形态的保留时长与淡出时长（ms）：3s 后开始淡出，淡完卸载 */
const STOPPED_HOLD_MS = 3000;
const STOPPED_FADE_MS = 300;

export function RecordingPill({
  active,
  elapsedMs,
  onPreview,
  onExportCast,
  onExportLog,
  onExportGif,
  onExportMp4,
  onResume,
  onIdleClose,
}: RecordingPillProps) {
  const { t } = useTranslation();
  const [actionsOpen, setActionsOpen] = useState(false);
  const { anchorRef, panelRef, position, measure } = useAnchoredPanel(actionsOpen);
  const closeActions = () => setActionsOpen(false);
  useDismissOnOutsidePointer(actionsOpen, [anchorRef, panelRef], closeActions);

  // 「已停止」形态仅作为面板锚点存在：面板从开到关且未在录制时才卸载。
  // （刚停止、面板尚未打开过时不触发——否则胶囊闪现即逝，用户没机会打开面板）
  const panelWasOpenRef = useRef(false);
  useEffect(() => {
    if (actionsOpen) {
      panelWasOpenRef.current = true;
      return;
    }
    if (panelWasOpenRef.current) {
      panelWasOpenRef.current = false;
      if (!active) onIdleClose();
    }
  }, [active, actionsOpen, onIdleClose]);

  // 停止后 3s 自动淡出隐藏：面板打开（用户在交互）或恢复录制则取消；
  // 淡出期间胶囊仍可点击，点开面板即拉回不透明并停表
  const [fading, setFading] = useState(false);
  useEffect(() => {
    if (active || actionsOpen) {
      setFading(false);
      return;
    }
    const fadeTimer = window.setTimeout(() => setFading(true), STOPPED_HOLD_MS);
    const hideTimer = window.setTimeout(() => onIdleClose(), STOPPED_HOLD_MS + STOPPED_FADE_MS);
    return () => {
      window.clearTimeout(fadeTimer);
      window.clearTimeout(hideTimer);
    };
  }, [active, actionsOpen, onIdleClose]);

  const runAction = (action: () => void) => {
    closeActions();
    action();
  };

  const gifItem = {
    key: 'exportGif',
    icon: <Film className="size-3.5" />,
    label: t('terminal.recording.exportGif'),
    onClick: () => runAction(() => void onExportGif()),
  };
  const mp4Item = {
    key: 'exportMp4',
    icon: <Video className="size-3.5" />,
    label: t('terminal.recording.exportMp4'),
    onClick: () => runAction(() => void onExportMp4()),
  };

  const items: { key: string; icon: ReactNode; label: string; onClick: () => void }[] = active
    ? [
        {
          key: 'preview',
          icon: <Eye className="size-3.5" />,
          label: t('terminal.recording.preview'),
          onClick: () => runAction(onPreview),
        },
        {
          key: 'exportCast',
          icon: <FileDown className="size-3.5" />,
          label: t('terminal.recording.exportCast'),
          onClick: () => runAction(onExportCast),
        },
        {
          key: 'exportLog',
          icon: <FileText className="size-3.5" />,
          label: t('terminal.recording.exportLog'),
          onClick: () => runAction(onExportLog),
        },
        gifItem,
        mp4Item,
        {
          key: 'resume',
          icon: <Play className="size-3.5" />,
          label: t('terminal.recording.resume'),
          onClick: () => runAction(onResume),
        },
      ]
    : [
        {
          key: 'preview',
          icon: <Eye className="size-3.5" />,
          label: t('terminal.recording.preview'),
          onClick: () => runAction(onPreview),
        },
        {
          key: 'exportCast',
          icon: <FileDown className="size-3.5" />,
          label: t('terminal.recording.exportCast'),
          onClick: () => runAction(onExportCast),
        },
        {
          key: 'exportLog',
          icon: <FileText className="size-3.5" />,
          label: t('terminal.recording.exportLog'),
          onClick: () => runAction(onExportLog),
        },
        gifItem,
        mp4Item,
      ];

  return (
    <>
      <div
        ref={anchorRef}
        className={cn('absolute left-3 top-2 z-30 transition-opacity duration-300', fading && 'opacity-0')}
      >
        <button
          type="button"
          aria-label={t('terminal.recording.pillAria')}
          onClick={() => {
            if (actionsOpen) {
              closeActions();
            } else {
              measure();
              setActionsOpen(true);
            }
          }}
          className={cn(
            'flex h-7 items-center gap-1.5 rounded-full border border-border bg-menu px-2.5 font-mono text-ui-xs shadow-md transition-colors hover:bg-menu-hover',
            active ? 'text-foreground' : 'text-foreground-subtle',
            fading && 'opacity-0',
          )}
        >
          {active ? (
            <>
              <span className="relative flex size-2.5 items-center justify-center">
                <span className="absolute inline-flex size-full rounded-full bg-red-500 status-pulse-rose opacity-90" />
                <span className="relative inline-flex size-2 rounded-full bg-red-500" />
              </span>
              <span>
                {t('terminal.recording.pillRecording')}
                {formatRecordElapsed(elapsedMs)}
              </span>
            </>
          ) : (
            <>
              <CircleStop className="size-3 text-foreground-subtlest" />
              <span>{t('terminal.recording.pillStopped')}</span>
            </>
          )}
        </button>
      </div>

      {actionsOpen && position
        ? createPortal(
            <div
              ref={panelRef}
              role="menu"
              style={{ position: 'fixed', top: position.top, left: position.left }}
              className="z-50 overflow-hidden rounded-lg border border-border bg-menu p-1 text-ui-sm text-foreground shadow-lg"
            >
              {items.map((item) => (
                <button
                  key={item.key}
                  type="button"
                  role="menuitem"
                  onClick={item.onClick}
                  className="flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-left transition-colors hover:bg-menu-hover"
                >
                  {item.icon}
                  <span>{item.label}</span>
                </button>
              ))}
            </div>,
            document.body,
          )
        : null}
    </>
  );
}
