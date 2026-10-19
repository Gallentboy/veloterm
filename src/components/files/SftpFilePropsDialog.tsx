import { AlertCircle, ChevronDown, File, Folder, Link2, Loader2 } from 'lucide-react';
import { useCallback, useEffect, useId, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Button } from '@/components/ui/button';
import { cn } from '@/lib/utils';
import {
  chmodRemote,
  chownRemote,
  listUsersGroupsRemote,
  type RemoteFileItem,
  type RemoteIdName,
} from '@/services/sftpService';

/**
 * SFTP 文件属性弹窗：权限九宫格 + 场景预设 + 八进制双向联动 + 特殊位 +
 * 属主/属组可搜下拉 + 递归（智能 X 符号模式默认开）。
 *
 * - 后端走远端 chmod/chown（模板 + 位置参数，无注入面），错误为稳定前缀码，
 *   本组件映射为 i18n 文案（EXEC_* / PERMISSION / INVALID_ID / REMOTE:）；
 * - 符号链接整体禁改：Linux 忽略链接自身权限位，chmod/chown 默认作用于目标，
 *   拿到的 attrs 又是链接自身的（lstat 语义），改了也不达预期；
 * - 应用顺序固定先 chmod 后 chown——先把属主转走会丧失后续 chmod 的权限；
 * - 用户/组清单 getent 拉取失败（exec 禁用等）降级为手输模式。
 */

interface SftpFilePropsDialogProps {
  sessionId: string;
  item: RemoteFileItem;
  onClose: () => void;
  /** 应用成功（父层刷新目录列表） */
  onApplied: () => void;
  /** exec 通道确认不可用（父层联动置灰入口） */
  onExecUnavailable?: () => void;
}

/** 权限位掩码：三组（owner/group/other）各 r/w/x，高位为 SUID/SGID/Sticky */
const CLASS_BITS: ReadonlyArray<{ r: number; w: number; x: number }> = [
  { r: 0o400, w: 0o200, x: 0o100 },
  { r: 0o040, w: 0o020, x: 0o010 },
  { r: 0o004, w: 0o002, x: 0o001 },
];
const SUID = 0o4000;
const SGID = 0o2000;
const STICKY = 0o1000;

/** 九宫格模式 → 符号串（rwxr-xr-x 形态；特殊位在有执行位时小写 s/t、无则大写） */
function formatSymbolic(mode: number): string {
  const special = [mode & SUID, mode & SGID, mode & STICKY];
  return CLASS_BITS.map(({ r, w, x }, i) => {
    let s = `${mode & r ? 'r' : '-'}${mode & w ? 'w' : '-'}`;
    const hasX = (mode & x) !== 0;
    if (i === 0) s += special[0] ? (hasX ? 's' : 'S') : hasX ? 'x' : '-';
    else if (i === 1) s += special[1] ? (hasX ? 's' : 'S') : hasX ? 'x' : '-';
    else s += special[2] ? (hasX ? 't' : 'T') : hasX ? 'x' : '-';
    return s;
  }).join('');
}

function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return '-';
  if (bytes < 1024) return `${bytes} B`;
  const units = ['KB', 'MB', 'GB', 'TB'];
  let value = bytes;
  let unit = -1;
  do {
    value /= 1024;
    unit += 1;
  } while (value >= 1024 && unit < units.length - 1);
  return `${value >= 100 ? Math.round(value) : value.toFixed(1)} ${units[unit]}`;
}

export function SftpFilePropsDialog(props: SftpFilePropsDialogProps) {
  const { sessionId, item, onClose, onApplied, onExecUnavailable } = props;
  const { t } = useTranslation();

  const initialMode = item.permissions & 0o7777;
  const initialOwner = item.owner ?? '';
  const initialGroup = item.group ?? '';

  const [mode, setMode] = useState(initialMode);
  /** 八进制输入框文本（与 mode 双向同步；非法输入保留原文待改） */
  const [modeText, setModeText] = useState(initialMode.toString(8).padStart(3, '0'));
  const [owner, setOwner] = useState(initialOwner);
  const [group, setGroup] = useState(initialGroup);
  const [recursive, setRecursive] = useState(false);
  const [smart, setSmart] = useState(true);
  const [showSpecial, setShowSpecial] = useState(false);

  const [idLists, setIdLists] = useState<{ users: RemoteIdName[]; groups: RemoteIdName[] } | null>(
    null,
  );
  const [idListFailed, setIdListFailed] = useState(false);
  const [applying, setApplying] = useState(false);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);

  const usersListId = useId();
  const groupsListId = useId();

  const editingDisabled = item.is_symlink === true;

  /* 用户/组清单：弹窗打开即拉取（会话级缓存，实际零重复 exec）；失败降级手输 */
  useEffect(() => {
    let cancelled = false;
    listUsersGroupsRemote(sessionId)
      .then(([users, groups]) => {
        if (!cancelled) setIdLists({ users, groups });
      })
      .catch(() => {
        if (!cancelled) {
          setIdListFailed(true);
          // EXEC_DISABLED / EXEC_CHANNEL 同样意味着属性修改不可用
          onExecUnavailable?.();
        }
      });
    return () => {
      cancelled = true;
    };
  }, [sessionId, onExecUnavailable]);

  const dirty = useMemo(
    () =>
      mode !== initialMode ||
      owner.trim() !== initialOwner.trim() ||
      group.trim() !== initialGroup.trim(),
    [mode, owner, group, initialMode, initialOwner, initialGroup],
  );

  const applyMode = useCallback(
    (updater: (prev: number) => number) => {
      setMode((prev) => {
        const next = updater(prev) & 0o7777;
        setModeText(next.toString(8).padStart(next >= 0o1000 ? 4 : 3, '0'));
        return next;
      });
    },
    [],
  );

  /** 前缀码错误 → i18n 文案（REMOTE: 原样透出服务器输出尾部） */
  const mapError = useCallback(
    (err: unknown): string => {
      const raw = err instanceof Error ? err.message : String(err);
      if (raw === 'EXEC_DISABLED' || raw.startsWith('EXEC_CHANNEL')) {
        onExecUnavailable?.();
        return t('files.propsErrExecDisabled');
      }
      if (raw === 'PERMISSION') return t('files.propsErrPermission');
      if (raw === 'INVALID_ID') return t('files.propsErrInvalidId');
      if (raw.startsWith('REMOTE:')) return raw.slice('REMOTE:'.length);
      return raw;
    },
    [t, onExecUnavailable],
  );

  const apply = useCallback(async () => {
    if (!dirty || applying) return;
    setApplying(true);
    setErrorMsg(null);
    try {
      const modeChanged = mode !== initialMode;
      const ownerChanged = owner.trim() !== '' && owner.trim() !== initialOwner.trim();
      const groupChanged = group.trim() !== '' && group.trim() !== initialGroup.trim();
      const recursiveDir = item.is_dir && recursive;
      // 先 chmod 后 chown：先把属主转走会丧失后续 chmod 的权限
      if (modeChanged) {
        await chmodRemote(sessionId, item.path, mode, recursiveDir, recursiveDir && smart);
      }
      if (ownerChanged || groupChanged) {
        await chownRemote(
          sessionId,
          item.path,
          ownerChanged ? owner.trim() : null,
          groupChanged ? group.trim() : null,
          recursiveDir,
        );
      }
      onApplied();
      onClose();
    } catch (err) {
      setErrorMsg(mapError(err));
      setApplying(false);
    }
  }, [
    dirty,
    applying,
    mode,
    owner,
    group,
    initialMode,
    initialOwner,
    initialGroup,
    item.path,
    item.is_dir,
    recursive,
    smart,
    sessionId,
    mapError,
    onApplied,
    onClose,
  ]);

  const typeIcon = item.is_dir ? (
    <Folder className="size-3.5 shrink-0 text-foreground-subtle" />
  ) : item.is_symlink ? (
    <Link2 className="size-3.5 shrink-0 text-foreground-subtle" />
  ) : (
    <File className="size-3.5 shrink-0 text-foreground-subtle" />
  );
  const typeLabel = item.is_dir
    ? t('files.propsTypeDir')
    : item.is_symlink
      ? t('files.propsTypeSymlink')
      : t('files.propsTypeFile');

  const presetButtons: Array<{ mode: number; label: string }> = [
    { mode: 0o644, label: t('files.preset644') },
    { mode: 0o755, label: t('files.preset755') },
    { mode: 0o600, label: t('files.preset600') },
    { mode: 0o777, label: t('files.preset777') },
  ];

  const gridRows: Array<{ label: string; bit: (c: { r: number; w: number; x: number }) => number }> =
    [
      { label: t('files.permRead'), bit: (c) => c.r },
      { label: t('files.permWrite'), bit: (c) => c.w },
      {
        label: item.is_dir ? t('files.permExecDir') : t('files.permExec'),
        bit: (c) => c.x,
      },
    ];
  const classLabels = [t('files.permClassOwner'), t('files.permClassGroup'), t('files.permClassOther')];

  const specialBits: Array<{ label: string; bit: number; desc: string }> = [
    { label: 'SUID', bit: SUID, desc: '4' },
    { label: 'SGID', bit: SGID, desc: '2' },
    { label: 'Sticky', bit: STICKY, desc: '1' },
  ];

  const idInputClass =
    'h-7 w-full rounded-md border border-border/70 bg-background px-2 font-mono text-ui-xs text-foreground outline-none transition-colors focus:border-border disabled:opacity-40';

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-label={t('files.props')}
      onClick={editingDisabled || !dirty ? onClose : undefined}
      className="fixed inset-0 z-[70] flex items-center justify-center bg-black/25 p-6 animate-in fade-in duration-150"
    >
      <div
        onClick={(event) => event.stopPropagation()}
        className="flex max-h-[min(85vh,760px)] w-[min(90vw,560px)] max-w-full flex-col overflow-hidden rounded-2xl border border-border/80 bg-popover shadow-2xl animate-in zoom-in-95 duration-150"
      >
        {/* 头部：类型图标 + 名称 */}
        <div className="flex shrink-0 items-center gap-2 border-b border-border/60 px-4 py-2.5">
          {typeIcon}
          <span className="truncate font-mono text-ui-sm text-foreground select-none">
            {item.name}
          </span>
          <span className="ml-auto shrink-0 text-ui-xs text-foreground-subtle select-none">
            {t('files.props')}
          </span>
        </div>

        <div className="scrollbar-slim min-h-0 flex-1 overflow-y-auto px-4 py-3 space-y-4">
          {/* 基本信息 */}
          <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-ui-xs">
            <dt className="text-foreground-subtle select-none">{t('files.propsType')}</dt>
            <dd className="truncate text-foreground">{typeLabel}</dd>
            <dt className="text-foreground-subtle select-none">{t('files.propsSize')}</dt>
            <dd className="text-foreground">
              {item.is_dir ? '-' : formatBytes(Number(item.size))}
            </dd>
            <dt className="text-foreground-subtle select-none">{t('files.propsModified')}</dt>
            <dd className="text-foreground">
              {item.modified_at > 0
                ? new Date(item.modified_at * 1000).toLocaleString()
                : '-'}
            </dd>
          </dl>

          {editingDisabled ? (
            <p className="flex items-start gap-1.5 rounded-md bg-surface/60 px-2.5 py-2 text-ui-xs text-foreground-subtle">
              <AlertCircle className="mt-0.5 size-3.5 shrink-0 text-warning" />
              <span>{t('files.propsSymlinkHint')}</span>
            </p>
          ) : (
            <>
              {/* 权限九宫格 + 八进制联动 */}
              <section className="space-y-2">
                <div className="flex items-center justify-between gap-2">
                  <span className="text-ui-xs font-medium text-foreground select-none">
                    {t('files.permTitle')}
                  </span>
                  <div className="flex items-center gap-2">
                    <span className="font-mono text-ui-xs text-foreground-subtle select-none">
                      {formatSymbolic(mode)}
                    </span>
                    <input
                      value={modeText}
                      onChange={(event) => {
                        const text = event.target.value;
                        setModeText(text);
                        if (/^[0-7]{3,4}$/.test(text)) {
                          setMode(parseInt(text, 8));
                        }
                      }}
                      aria-label={t('files.permOctal')}
                      className="h-6 w-14 rounded-md border border-border/70 bg-background px-1.5 text-center font-mono text-ui-xs text-foreground outline-none focus:border-border"
                    />
                  </div>
                </div>
                <div className="grid grid-cols-[5rem_repeat(3,1fr)] items-center gap-x-2 gap-y-1.5">
                  <span />
                  {classLabels.map((label) => (
                    <span
                      key={label}
                      className="text-center text-ui-xs text-foreground-subtle select-none"
                    >
                      {label}
                    </span>
                  ))}
                  {gridRows.map((row) => (
                    <div key={row.label} className="col-span-4 grid grid-cols-[5rem_repeat(3,1fr)] items-center gap-x-2">
                      <span className="text-ui-xs text-foreground-subtle select-none">
                        {row.label}
                      </span>
                      {CLASS_BITS.map((classBits, classIndex) => {
                        const bit = row.bit(classBits);
                        const checked = (mode & bit) !== 0;
                        return (
                          <label
                            key={classIndex}
                            className="flex justify-center"
                          >
                            <input
                              type="checkbox"
                              checked={checked}
                              onChange={(event) =>
                                applyMode((prev) =>
                                  event.target.checked ? prev | bit : prev & ~bit,
                                )
                              }
                              className="size-3.5 accent-foreground"
                            />
                          </label>
                        );
                      })}
                    </div>
                  ))}
                </div>

                {/* 特殊位（折叠） */}
                <div>
                  <button
                    type="button"
                    onClick={() => setShowSpecial((v) => !v)}
                    className="flex items-center gap-1 text-ui-xs text-foreground-subtle transition-colors hover:text-foreground select-none"
                  >
                    <ChevronDown
                      className={cn('size-3 transition-transform', showSpecial && 'rotate-180')}
                    />
                    <span>{t('files.permAdvanced')}</span>
                  </button>
                  {showSpecial ? (
                    <div className="mt-1.5 flex items-center gap-4 rounded-md bg-surface/40 px-2.5 py-2">
                      {specialBits.map(({ label, bit, desc }) => (
                        <label key={label} className="flex items-center gap-1.5 select-none">
                          <input
                            type="checkbox"
                            checked={(mode & bit) !== 0}
                            onChange={(event) =>
                              applyMode((prev) =>
                                event.target.checked ? prev | bit : prev & ~bit,
                              )
                            }
                            className="size-3.5 accent-foreground"
                          />
                          <span className="text-ui-xs text-foreground">{label}</span>
                          <span className="font-mono text-ui-xs text-foreground-subtlest">{desc}</span>
                        </label>
                      ))}
                      <span className="ml-auto text-ui-xs text-foreground-subtlest">
                        {t('files.permSpecialHint')}
                      </span>
                    </div>
                  ) : null}
                </div>

                {/* 场景预设 */}
                <div className="flex flex-wrap items-center gap-1.5">
                  <span className="text-ui-xs text-foreground-subtle select-none">
                    {t('files.permPresets')}
                  </span>
                  {presetButtons.map(({ mode: preset, label }) => (
                    <Button
                      key={preset}
                      type="button"
                      size="xs"
                      variant="outline"
                      onClick={() => applyMode(() => preset)}
                      className={cn(
                        'font-mono',
                        mode === preset && 'border-primary/60 text-primary',
                        preset === 0o777 && 'hover:border-destructive/60 hover:text-destructive',
                      )}
                    >
                      {label}
                    </Button>
                  ))}
                  {mode === 0o777 ? (
                    <span className="text-ui-xs text-warning">{t('files.presetOpenWarning')}</span>
                  ) : null}
                </div>
              </section>

              {/* 属主 / 属组 */}
              <section className="grid grid-cols-[1fr_1fr] gap-3">
                <div className="space-y-1">
                  <label
                    htmlFor={`${usersListId}-input`}
                    className="block text-ui-xs text-foreground-subtle select-none"
                  >
                    {t('files.propsOwner')}
                  </label>
                  <input
                    id={`${usersListId}-input`}
                    list={usersListId}
                    value={owner}
                    onChange={(event) => setOwner(event.target.value)}
                    placeholder={initialOwner || 'root'}
                    className={idInputClass}
                  />
                  <datalist id={usersListId}>
                    {(idLists?.users ?? []).map((entry) => (
                      <option key={`u${entry.id}`} value={entry.name}>
                        {`(${entry.id})`}
                      </option>
                    ))}
                  </datalist>
                </div>
                <div className="space-y-1">
                  <label
                    htmlFor={`${groupsListId}-input`}
                    className="block text-ui-xs text-foreground-subtle select-none"
                  >
                    {t('files.propsGroup')}
                  </label>
                  <input
                    id={`${groupsListId}-input`}
                    list={groupsListId}
                    value={group}
                    onChange={(event) => setGroup(event.target.value)}
                    placeholder={initialGroup || 'root'}
                    className={idInputClass}
                  />
                  <datalist id={groupsListId}>
                    {(idLists?.groups ?? []).map((entry) => (
                      <option key={`g${entry.id}`} value={entry.name}>
                        {`(${entry.id})`}
                      </option>
                    ))}
                  </datalist>
                </div>
                <p className="col-span-2 text-ui-xs text-foreground-subtlest select-none">
                  {idListFailed
                    ? t('files.idListUnavailable')
                    : t('files.propsOwnerHint')}
                </p>
              </section>

              {/* 递归（仅目录） */}
              {item.is_dir ? (
                <section className="space-y-1.5 rounded-md bg-surface/40 px-2.5 py-2">
                  <label className="flex items-center gap-2 text-ui-xs text-foreground select-none">
                    <input
                      type="checkbox"
                      checked={recursive}
                      onChange={(event) => setRecursive(event.target.checked)}
                      className="size-3.5 accent-foreground"
                    />
                    <span>{t('files.recursiveLabel')}</span>
                  </label>
                  <label
                    className={cn(
                      'flex items-center gap-2 pl-5 text-ui-xs select-none transition-opacity',
                      !recursive && 'pointer-events-none opacity-40',
                    )}
                  >
                    <input
                      type="checkbox"
                      checked={smart}
                      disabled={!recursive}
                      onChange={(event) => setSmart(event.target.checked)}
                      className="size-3.5 accent-foreground"
                    />
                    <span className="text-foreground-subtle">{t('files.smartRecursiveLabel')}</span>
                  </label>
                </section>
              ) : null}
            </>
          )}
        </div>

        {/* 底部：错误 + 操作 */}
        <div className="flex shrink-0 items-center justify-between gap-3 border-t border-border/60 px-4 py-2.5">
          <div className="min-w-0 flex-1">
            {errorMsg != null ? (
              <p className="flex items-start gap-1.5 text-ui-xs text-warning">
                <AlertCircle className="mt-0.5 size-3.5 shrink-0" />
                <span className="break-all">{errorMsg}</span>
              </p>
            ) : null}
          </div>
          <div className="flex shrink-0 items-center gap-1.5">
            <Button type="button" size="sm" variant="ghost" onClick={onClose}>
              {t('common.cancel')}
            </Button>
            <Button
              type="button"
              size="sm"
              disabled={!dirty || applying || editingDisabled}
              onClick={() => void apply()}
            >
              {applying ? <Loader2 className="size-3 animate-spin" /> : null}
              <span>{t('files.propsApply')}</span>
            </Button>
          </div>
        </div>
      </div>
    </div>
  );
}
