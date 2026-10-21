/**
 * 终端录制指示胶囊 + 停止后的操作面板。
 *
 * 录制中：左上角浮动胶囊（红点呼吸 + REC mm:ss 计时；搜索栏在右上、ZMODEM
 * 提示在顶部居中，左上无占用）。点击胶囊弹出操作面板（portal 到 body +
 * fixed，useAnchoredPanel 视口夹取，useDismissOnOutsidePointer 多 ref 排除
 * 触发器与弹层）——停止并预览 / 停止并导出 .cast / 停止并导出 .log /
 * 继续录制。停止后胶囊以「已停止」形态保留为面板锚点，面板关闭即卸载。
 */

import { Circle, CircleStop, Eye, FileDown, FileText, Play } from 'lucide-react';
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
  onResume: () => void;
  /** 面板已关且未在录制：父组件卸载胶囊（已停止形态仅作为面板锚点存在） */
  onIdleClose: () => void;
}

export function RecordingPill({
  active,
  elapsedMs,
  onPreview,
  onExportCast,
  onExportLog,
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

  const runAction = (action: () => void) => {
    closeActions();
    action();
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
      ];

  return (
    <>
      <div ref={anchorRef} className="absolute left-3 top-2 z-30">
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
          )}
        >
          {active ? (
            <>
              <Circle className="size-2.5 animate-pulse fill-red-500 text-red-500" />
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
              className="z-50 min-w-44 overflow-hidden rounded-lg border border-border bg-menu p-1 text-ui-sm text-foreground shadow-lg"
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
