import { AlertTriangle, ShieldAlert, Terminal } from 'lucide-react';
import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';

import { Modal } from '@/components/ui/modal';
import type { HostEnvironmentLevel } from '@/hosts/types';
import type { SafetyInspectResult } from '@/services/safetyGuardService';

interface SafetyGuardModalProps {
  isOpen: boolean;
  command: string;
  inspectResult: SafetyInspectResult;
  hostName: string;
  envLevel: HostEnvironmentLevel;
  onConfirm: () => void;
  onCancel: () => void;
}

export function SafetyGuardModal({
  isOpen,
  command,
  inspectResult,
  hostName,
  envLevel,
  onConfirm,
  onCancel,
}: SafetyGuardModalProps) {
  const { t } = useTranslation();
  const [verificationInput, setVerificationInput] = useState('');
  const requireCode = inspectResult.requireVerificationCode;

  // 必须输入的确认码：主机名，或者 'CONFIRM'
  const expectedCode = hostName || 'CONFIRM';
  const isCodeMatched =
    !requireCode ||
    verificationInput.trim() === expectedCode ||
    verificationInput.trim().toUpperCase() === 'CONFIRM';

  // 打开时重置输入
  useEffect(() => {
    if (isOpen) {
      setVerificationInput('');
    }
  }, [isOpen]);

  if (!isOpen) return null;

  const levelColor =
    inspectResult.level === 'critical'
      ? 'text-red-500 bg-red-500/10 border-red-500/30'
      : inspectResult.level === 'high'
      ? 'text-amber-500 bg-amber-500/10 border-amber-500/30'
      : 'text-yellow-500 bg-yellow-500/10 border-yellow-500/30';

  const badgeText =
    inspectResult.level === 'critical'
      ? t('safetyGuard.levelCritical', 'CRITICAL 毁灭级')
      : inspectResult.level === 'high'
      ? t('safetyGuard.levelHigh', 'HIGH 高危级')
      : t('safetyGuard.levelWarning', 'WARNING 警告级');

  return (
    <Modal
      onClose={onCancel}
      title={t('safetyGuard.interceptTitle', '生产环境安全拦截')}
    >
      <div className="space-y-4 text-sm text-foreground">
        <div className="flex items-center gap-2 text-red-500 font-semibold -mt-1 pb-1 border-b border-border/40">
          <ShieldAlert className="w-5 h-5 animate-pulse text-red-500" />
          <span>{t('safetyGuard.interceptAlert', '高危敏感命令阻断警示')}</span>
        </div>
        {/* 顶部风险提示条 */}
        <div className={`p-3 rounded-lg border flex items-start gap-3 ${levelColor}`}>
          <AlertTriangle className="w-5 h-5 shrink-0 mt-0.5" />
          <div className="space-y-1">
            <div className="flex items-center gap-2">
              <span className="font-bold text-xs uppercase px-1.5 py-0.5 rounded border border-current">
                {badgeText}
              </span>
              <span className="font-medium">
                {inspectResult.ruleTitle || t('safetyGuard.unknownRule', '命中安全规则')}
              </span>
            </div>
            <p className="text-xs opacity-90 leading-relaxed">
              {inspectResult.explanation ||
                t('safetyGuard.defaultExplanation', '该命令在生产环境中执行可能导致不可逆的数据丢失或服务中断。')}
            </p>
          </div>
        </div>

        {/* 目标环境与主机信息 */}
        <div className="flex items-center justify-between text-xs px-2 py-1.5 bg-muted/40 rounded border border-border/40">
          <span className="text-muted-foreground">{t('safetyGuard.targetHost', '目标主机与环境')}:</span>
          <div className="flex items-center gap-1.5 font-mono">
            <span
              className={`px-1.5 py-0.2 rounded font-bold uppercase text-[10px] ${
                envLevel === 'prod' ? 'bg-red-500 text-white' : 'bg-amber-500 text-black'
              }`}
            >
              {envLevel.toUpperCase()}
            </span>
            <span className="font-medium text-foreground">{hostName}</span>
          </div>
        </div>

        {/* 待执行命令详情 */}
        <div className="space-y-1.5">
          <div className="flex items-center gap-1.5 text-xs text-muted-foreground font-medium">
            <Terminal className="w-3.5 h-3.5" />
            <span>{t('safetyGuard.commandToRun', '拦截到的命令行')}:</span>
          </div>
          <div className="p-2.5 bg-black/90 text-red-400 font-mono text-xs rounded border border-red-500/20 max-h-36 overflow-y-auto whitespace-pre-wrap break-all select-text">
            {command}
          </div>
        </div>

        {/* Critical 级别的强制确认码输入 */}
        {requireCode && (
          <div className="space-y-1.5 pt-1">
            <label className="text-xs font-medium text-foreground flex items-center justify-between">
              <span>{t('safetyGuard.confirmCodePrompt', '为防止肌肉记忆误触，请输入确认词')}:</span>
              <span className="font-mono text-xs text-red-500 font-semibold select-all">
                {expectedCode}
              </span>
            </label>
            <input
              type="text"
              autoFocus
              value={verificationInput}
              onChange={(e) => setVerificationInput(e.target.value)}
              placeholder={t('safetyGuard.inputPlaceholder', '输入主机名或 CONFIRM 以解锁')}
              className="w-full px-3 py-1.5 bg-background border border-border rounded text-sm font-mono focus:outline-none focus:ring-1 focus:ring-red-500"
              onKeyDown={(e) => {
                if (e.key === 'Enter' && isCodeMatched) {
                  e.preventDefault();
                  onConfirm();
                } else if (e.key === 'Escape') {
                  e.preventDefault();
                  onCancel();
                }
              }}
            />
          </div>
        )}

        {/* 操作按钮区 */}
        <div className="flex items-center justify-end gap-2.5 pt-2 border-t border-border/50">
          <button
            type="button"
            onClick={onCancel}
            className="px-3.5 py-1.5 rounded text-xs font-medium border border-border bg-background hover:bg-muted transition-colors text-foreground"
          >
            {t('safetyGuard.cancelButton', '取消执行 (Esc)')}
          </button>
          <button
            type="button"
            disabled={!isCodeMatched}
            onClick={onConfirm}
            className={`px-4 py-1.5 rounded text-xs font-medium text-white transition-all shadow-sm ${
              isCodeMatched
                ? 'bg-red-600 hover:bg-red-700 active:scale-95'
                : 'bg-red-600/40 cursor-not-allowed opacity-60'
            }`}
          >
            {t('safetyGuard.confirmButton', '确认放行并执行')}
          </button>
        </div>
      </div>
    </Modal>
  );
}
