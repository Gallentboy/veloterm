import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { useTranslation } from 'react-i18next';
import {
  Check,
  ChevronDown,
  ChevronRight,
  Copy,
  Loader2,
  RefreshCw,
  Trash2,
} from 'lucide-react';
import { Button } from '@/components/ui/button';
import { ConfirmDialog } from '@/components/ui/ConfirmDialog';
import { SelectMenu } from '@/components/ui/SelectMenu';
import { ControlHintTooltip } from '@/components/ControlHintTooltip';
import { useHostsStore } from '@/hosts/HostsProvider';
import { writeClipboardText } from '@/lib/clipboard';
import { cn } from '@/lib/utils';
import {
  commandHistoryClear,
  commandHistoryGetEnabled,
  commandHistoryList,
  commandHistorySetEnabled,
  type CommandHistoryRow,
} from '@/services/commandHistoryService';

/** 分页页大小（后端钳制上限 1000；不足一页 = 数据见底，禁用「加载更多」） */
const PAGE_SIZE = 200;

/** 复制按钮回弹时长（毫秒） */
const COPY_RESET_MS = 1500;

/** 主机过滤下拉的三个固定档：全部 / 仅本地（host_id IS NULL，服务端过滤）/ 各主机 */
const HOST_FILTER_ALL = 'all';
const HOST_FILTER_LOCAL = 'local';

/** 启用采集引导：三段 shell 片段（与后端 OSC 7777 解析器协议逐字对应，一字不能改） */
const SNIPPETS: Array<{ shell: 'bash' | 'zsh' | 'fish'; code: string }> = [
  {
    shell: 'bash',
    code: `# VeloTerm 命令历史采集（粘贴到 ~/.bashrc；幂等，可重复粘贴）
__veloterm_report() {
  local rc=$? line cmd enc
  if [[ -n \${__veloterm_skip:-} ]]; then __veloterm_skip=; return; fi
  line=$(HISTTIMEFORMAT= history 1)
  line="\${line#"\${line%%[! ]*}"}"
  line="\${line#"\${line%%[!0-9]*}"}"
  cmd="\${line#"\${line%%[! ]*}"}"
  [[ -n $cmd ]] || return
  enc=\${cmd//%/%25}; enc=\${enc//;/%3B}; enc=\${enc//$'\\x1b'/%1B}; enc=\${enc//$'\\a'/%07}
  enc=\${enc//$'\\n'/%0A}; enc=\${enc//$'\\r'/%0D}
  printf '\\033]7777;cmd;%s;%s\\033\\\\' "$rc" "$enc"
}
case ";\${PROMPT_COMMAND:-};" in *";__veloterm_report;"*) ;; *) PROMPT_COMMAND="__veloterm_report\${PROMPT_COMMAND:+;$PROMPT_COMMAND}";; esac
__veloterm_skip=1`,
  },
  {
    shell: 'zsh',
    code: `# VeloTerm 命令历史采集（粘贴到 ~/.zshrc；幂等，可重复粘贴）
__veloterm_report() {
  local rc=$? cmd enc
  if [[ -n \${__veloterm_skip:-} ]]; then __veloterm_skip=; return; fi
  cmd=\${history[\${#history}]}
  [[ -n $cmd ]] || return
  enc=\${cmd//"%"/%25}; enc=\${enc//;/%3B}; enc=\${enc//$'\\x1b'/%1B}; enc=\${enc//$'\\a'/%07}
  enc=\${enc//$'\\n'/%0A}; enc=\${enc//$'\\r'/%0D}
  printf '\\033]7777;cmd;%s;%s\\033\\\\' "$rc" "$enc"
}
(( \${precmd_functions[(Ie)__veloterm_report]} )) || precmd_functions+=(__veloterm_report)
__veloterm_skip=1`,
  },
  {
    shell: 'fish',
    code: `# VeloTerm 命令历史采集（粘贴到 ~/.config/fish/config.fish；幂等）
function __veloterm_report --on-event fish_prompt
    if set -q __veloterm_skip
        set -e __veloterm_skip
        return
    end
    set -l rc $status
    set -l cmd $history[1]
    test -n "$cmd"; or return
    set -l enc (string replace -a -- '%' '%25' $cmd | string collect)
    set enc (string replace -a -- ';' '%3B' $enc | string collect)
    set enc (string replace -a -- \\x1b '%1B' $enc | string collect)
    set enc (string replace -a -- \\x07 '%07' $enc | string collect)
    set enc (string replace -a -- \\n '%0A' $enc | string collect)
    set enc (string replace -a -- \\r '%0D' $enc | string collect)
    printf '\\e]7777;cmd;%s;%s\\e\\\\' $rc "$enc"
end
set -g __veloterm_skip 1`,
  },
];

/** 本地时区格式化到秒（无 AM/PM，与 SFTP 列表的时间口径一致） */
function formatTime(ms: number): string {
  return new Date(ms).toLocaleString(undefined, {
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
    hour12: false,
  });
}

/** 采集开关的小型自绘 toggle（工程内无通用 Switch，按语义 token 自绘） */
function CaptureToggle({
  checked,
  disabled,
  onToggle,
  label,
}: {
  checked: boolean;
  disabled?: boolean;
  onToggle: () => void;
  label: string;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      disabled={disabled}
      onClick={onToggle}
      className={cn(
        'relative inline-flex h-4.5 w-8 shrink-0 cursor-pointer items-center rounded-full border transition-colors',
        checked ? 'border-transparent bg-primary' : 'border-border bg-input',
        disabled && 'cursor-not-allowed opacity-50',
      )}
    >
      <span
        className={cn(
          'inline-block size-3.5 rounded-full transition-transform duration-150',
          checked ? 'translate-x-4 bg-primary-foreground' : 'translate-x-0.5 bg-foreground-subtle',
        )}
      />
    </button>
  );
}

/** 命令历史页：采集开关 + 主机/失败过滤 + 时间倒序分页列表 + 清空 + 启用采集引导。 */
export function CommandHistoryScreen() {
  const { t } = useTranslation();
  const { hosts } = useHostsStore();

  const [enabled, setEnabled] = useState<boolean | null>(null); // null = 初值读取中
  const [rows, setRows] = useState<CommandHistoryRow[]>([]);
  /** 下一页的 backend offset（按取回量累计，与服务端分页口径一致） */
  const [nextOffset, setNextOffset] = useState(0);
  const [hostFilter, setHostFilter] = useState<string>(HOST_FILTER_ALL);
  const [onlyFailed, setOnlyFailed] = useState(false);
  const [loading, setLoading] = useState(true);
  const [loadingMore, setLoadingMore] = useState(false);
  const [exhausted, setExhausted] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [confirmClear, setConfirmClear] = useState(false);
  const [clearing, setClearing] = useState(false);
  const [helpOpen, setHelpOpen] = useState(false);
  const [copiedShell, setCopiedShell] = useState<string | null>(null);
  const copyTimerRef = useRef<number>(0);

  // 挂载读采集开关初值（失败保持 null：toggle 禁用，避免显示错误状态）
  useEffect(() => {
    let cancelled = false;
    commandHistoryGetEnabled()
      .then((value) => {
        if (!cancelled) setEnabled(value);
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, []);

  // 过滤条件 → 查询参数：「仅本地」走服务端 localOnly 过滤（host_id IS NULL）
  const isLocalFilter = hostFilter === HOST_FILTER_LOCAL;
  const selectedHostId = hostFilter.startsWith('host:') ? Number(hostFilter.slice(5)) : null;
  const queryHostId = isLocalFilter ? null : selectedHostId;
  const isDefaultFilter = hostFilter === HOST_FILTER_ALL && !onlyFailed;

  const loadPage = useCallback(
    async (offset: number, mode: 'replace' | 'append') => {
      if (mode === 'replace') setLoading(true);
      else setLoadingMore(true);
      try {
        const fetched = await commandHistoryList({
          hostId: queryHostId,
          onlyFailed,
          localOnly: isLocalFilter,
          limit: PAGE_SIZE,
          offset,
        });
        setRows((current) => (mode === 'replace' ? fetched : [...current, ...fetched]));
        setNextOffset(offset + fetched.length);
        setExhausted(fetched.length < PAGE_SIZE);
        setError(null);
      } catch (e) {
        setError(String(e));
        if (mode === 'replace') {
          setRows([]);
          setExhausted(true);
        }
      } finally {
        setLoading(false);
        setLoadingMore(false);
      }
    },
    [isLocalFilter, onlyFailed, queryHostId],
  );

  // 过滤条件变化即回到第一页重查
  useEffect(() => {
    void loadPage(0, 'replace');
  }, [loadPage]);

  // 完全无记录（默认过滤下为空）→ 自动展开「启用采集」引导
  useEffect(() => {
    if (!loading && rows.length === 0 && isDefaultFilter) {
      setHelpOpen(true);
    }
  }, [loading, rows.length, isDefaultFilter]);

  const handleToggleEnabled = useCallback(() => {
    if (enabled == null) return;
    const next = !enabled;
    setEnabled(next); // 乐观更新，失败回滚
    commandHistorySetEnabled(next).catch(() => {
      setEnabled(!next);
      setError(String(t('commandHistory.toggleFailed')));
    });
  }, [enabled, t]);

  const handleCopy = useCallback(async (shell: string, code: string) => {
    try {
      await writeClipboardText(code);
      setCopiedShell(shell);
      window.clearTimeout(copyTimerRef.current);
      copyTimerRef.current = window.setTimeout(() => setCopiedShell(null), COPY_RESET_MS);
    } catch {
      // 剪贴板不可用时静默（与终端复制路径一致）
    }
  }, []);

  useEffect(() => () => window.clearTimeout(copyTimerRef.current), []);

  const handleClear = useCallback(async () => {
    setClearing(true);
    try {
      await commandHistoryClear(isLocalFilter ? null : selectedHostId, isLocalFilter);
      setConfirmClear(false);
      await loadPage(0, 'replace');
    } catch (e) {
      setError(String(e));
    } finally {
      setClearing(false);
    }
  }, [isLocalFilter, selectedHostId, loadPage]);

  // hostId → 显示名（null = 本地；已删除的主机按 id 兜底显示）
  const hostNameById = useMemo(() => new Map(hosts.map((host) => [host.id, host.name])), [hosts]);
  const hostOf = useCallback(
    (hostId: number | null): string =>
      hostId == null ? t('commandHistory.localHost') : hostNameById.get(hostId) ?? t('commandHistory.unknownHost', { id: hostId }),
    [hostNameById, t],
  );

  // 清空范围：选了具体主机只清该主机；仅本地只清本地记录；全部 = 全部主机
  const clearTargetName = isLocalFilter
    ? t('commandHistory.filterLocal')
    : selectedHostId != null
      ? hostOf(selectedHostId)
      : null;
  const hostFilterOptions = useMemo(
    () => [
      { value: HOST_FILTER_ALL, label: t('commandHistory.filterAllHosts') },
      { value: HOST_FILTER_LOCAL, label: t('commandHistory.filterLocal') },
      ...hosts.map((host) => ({ value: `host:${host.id}`, label: host.name })),
    ],
    [hosts, t],
  );

  const showGuide = !loading && rows.length === 0 && isDefaultFilter;
  const showFilteredEmpty = !loading && rows.length === 0 && !isDefaultFilter;

  return (
    <div className="flex h-full flex-col overflow-hidden">
      {/* 顶部：页面标题 + 采集开关 */}
      <div className="flex shrink-0 items-center gap-2 border-b border-border/50 px-4 py-2">
        <span className="text-ui-sm font-medium text-foreground select-none">{t('commandHistory.title')}</span>
        <span className="text-ui-xs text-foreground-subtlest select-none">{t('commandHistory.subtitle')}</span>
        <div className="flex-1" />
        <ControlHintTooltip title={t('commandHistory.captureHint')} side="bottom">
          <span className="flex items-center gap-2 select-none">
            <span className="text-ui-xs text-foreground-subtle">{t('commandHistory.captureLabel')}</span>
            <CaptureToggle
              checked={enabled ?? false}
              disabled={enabled == null}
              onToggle={handleToggleEnabled}
              label={t('commandHistory.captureLabel')}
            />
          </span>
        </ControlHintTooltip>
      </div>

      {error ? (
        <div className="flex shrink-0 items-center justify-between gap-2 border-b border-destructive/20 bg-destructive/10 px-4 py-1.5 text-ui-xs text-destructive">
          <span className="min-w-0 flex-1 truncate">{error}</span>
          <button type="button" onClick={() => setError(null)} className="opacity-70 hover:opacity-100">✕</button>
        </div>
      ) : null}

      {/* 过滤行：主机下拉 + 只看失败 + 刷新 + 清空 */}
      <div className="flex shrink-0 flex-wrap items-center gap-2 border-b border-border/50 px-4 py-2">
        <div className="w-44">
          <SelectMenu
            aria-label={t('commandHistory.hostFilter')}
            value={hostFilter}
            onChange={setHostFilter}
            options={hostFilterOptions}
          />
        </div>
        <label className="flex items-center gap-1.5 text-ui-sm text-foreground select-none">
          <input
            type="checkbox"
            checked={onlyFailed}
            onChange={(e) => setOnlyFailed(e.target.checked)}
            className="accent-foreground"
          />
          {t('commandHistory.onlyFailed')}
        </label>
        <div className="flex-1" />
        <ControlHintTooltip title={t('commandHistory.refresh')} side="bottom">
          <Button
            variant="outline"
            size="lg"
            className="h-7 px-2.5 text-ui-xs"
            disabled={loading || loadingMore}
            onClick={() => void loadPage(0, 'replace')}
          >
            <RefreshCw className={cn('size-3.5', loading && 'animate-spin')} />
          </Button>
        </ControlHintTooltip>
        <Button
          variant="outline"
          size="lg"
          className="h-7 gap-1.5 px-2.5 text-ui-xs text-destructive hover:bg-destructive/10"
          disabled={clearing}
          onClick={() => setConfirmClear(true)}
        >
          <Trash2 className="size-3.5" />
          {t('commandHistory.clear')}
        </Button>
      </div>

      {/* 记录列表 */}
      <div className="flex min-h-0 flex-1 flex-col gap-1.5 overflow-y-auto p-3">
        {loading ? (
          <span className="text-ui-sm text-foreground-subtlest select-none">{t('commandHistory.loading')}</span>
        ) : showGuide ? (
          <div className="flex flex-1 flex-col items-center justify-center gap-2 select-none">
            <span className="text-ui-sm text-foreground-subtlest">{t('commandHistory.emptyGuide')}</span>
          </div>
        ) : showFilteredEmpty ? (
          <div className="flex flex-1 flex-col items-center justify-center gap-2 select-none">
            <span className="text-ui-sm text-foreground-subtlest">{t('commandHistory.emptyFiltered')}</span>
          </div>
        ) : (
          rows.map((row) => (
            <div
              key={row.id}
              className="flex items-center gap-3 rounded-lg border border-border px-3 py-2 transition-colors hover:border-border-hover"
            >
              <span className="w-40 shrink-0 font-mono text-ui-xs text-foreground-subtle select-none">
                {formatTime(row.createdAt)}
              </span>
              <span className="w-28 shrink-0 truncate text-ui-sm text-foreground-subtle" title={hostOf(row.hostId)}>
                {hostOf(row.hostId)}
              </span>
              <span
                className="min-w-0 flex-1 truncate font-mono text-ui-sm text-foreground"
                title={row.command}
              >
                {row.command}
              </span>
              <span
                title={t('commandHistory.exitCode')}
                className={cn(
                  'shrink-0 rounded-md border px-1.5 py-0.5 font-mono text-ui-xs select-none',
                  row.exitCode !== 0
                    ? 'border-destructive/30 bg-destructive/10 text-destructive'
                    : 'border-border/60 bg-surface text-foreground-subtlest',
                )}
              >
                {row.exitCode}
              </span>
            </div>
          ))
        )}

        {rows.length > 0 ? (
          <Button
            variant="outline"
            size="lg"
            className="h-7 self-start px-3 text-ui-xs"
            disabled={exhausted || loadingMore}
            onClick={() => void loadPage(nextOffset, 'append')}
          >
            {loadingMore ? <Loader2 className="size-3.5 animate-spin" /> : null}
            {exhausted ? t('commandHistory.noMore') : t('commandHistory.loadMore')}
          </Button>
        ) : null}

        {/* 「启用采集」可折叠帮助区：三段 shell 片段逐字对应后端解析协议。
            外层禁 overflow-hidden——滚动容器里它会退化成「压缩裁剪」而非撑出滚动高度
            （被裁内容不可达）；展开体按仓库惯例（GrpcScreen 同款）限高内部滚动 */}
        <div className="mt-1 shrink-0 rounded-lg border border-border/60">
          <button
            type="button"
            onClick={() => setHelpOpen((current) => !current)}
            aria-expanded={helpOpen}
            className="flex w-full items-center gap-2 px-3 py-2 text-left transition-colors hover:bg-hover select-none"
          >
            {helpOpen ? (
              <ChevronDown className="size-3.5 shrink-0 text-foreground-subtle" />
            ) : (
              <ChevronRight className="size-3.5 shrink-0 text-foreground-subtle" />
            )}
            <span className="text-ui-sm font-medium text-foreground">{t('commandHistory.helpTitle')}</span>
            <span className="min-w-0 flex-1 truncate text-ui-xs text-foreground-subtlest">
              {t('commandHistory.helpSummary')}
            </span>
          </button>
          {helpOpen ? (
            <div className="flex max-h-[46vh] flex-col gap-3 overflow-y-auto border-t border-border/60 p-3">
              <p className="text-ui-xs leading-relaxed text-foreground-subtle">{t('commandHistory.helpDesc')}</p>
              {SNIPPETS.map(({ shell, code }) => (
                <div key={shell} className="flex flex-col gap-1.5">
                  <div className="flex items-center">
                    <span className="font-mono text-ui-xs text-foreground-subtle select-none">{shell}</span>
                    <div className="flex-1" />
                    <Button
                      variant="ghost"
                      size="sm"
                      className="h-6 gap-1 px-1.5 text-ui-xs text-foreground-subtle"
                      onClick={() => void handleCopy(shell, code)}
                    >
                      {copiedShell === shell ? (
                        <Check className="size-3 text-primary" />
                      ) : (
                        <Copy className="size-3" />
                      )}
                      {copiedShell === shell ? t('commandHistory.copied') : t('commandHistory.copy')}
                    </Button>
                  </div>
                  <pre className="overflow-x-auto rounded-md border border-border/60 bg-input/30 p-2 font-mono text-ui-xs leading-relaxed text-foreground-subtle">
                    {code}
                  </pre>
                </div>
              ))}
            </div>
          ) : null}
        </div>
      </div>

      {/* 清空确认（portal 到 document.body） */}
      {confirmClear
        ? createPortal(
            <ConfirmDialog
              open
              title={t('commandHistory.clearTitle')}
              description={t('commandHistory.clearConfirm', {
                scope: clearTargetName ?? t('commandHistory.clearScopeAll'),
              })}
              confirmLabel={t('commandHistory.clear')}
              busy={clearing}
              onConfirm={() => void handleClear()}
              onCancel={() => setConfirmClear(false)}
            />,
            document.body,
          )
        : null}
    </div>
  );
}
