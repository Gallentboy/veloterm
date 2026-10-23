/**
 * 数据库主机管理页（ZkHostsScreen 同型）：连接库列表 + 表单 Modal（引擎/
 * 主机/凭证/SSL/环境级别/只读）+ 测试连接 + 复制/删除。行内「连接」经
 * onConnectConnection 派发 window 事件跳数据库工作台页自动开 tab。
 * 页头另有「Oracle 客户端」设置弹层：Oracle 引擎运行期 dlopen Instant
 * Client，用户可指定库目录（进程级一次初始化，改目录需重启生效）。
 * SQLite 引擎表单以数据库文件路径 + 加密方式（SQLCipher）取代主机/端口/SSL。
 * SSH 隧道（P1）未接入。
 */

import { useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { useTranslation } from 'react-i18next';
import {
  CircleCheck,
  CircleX,
  Database,
  FolderOpen,
  Loader2,
  Pencil,
  Play,
  Plus,
  Trash2,
  Zap,
} from 'lucide-react';

import { Button } from '@/components/ui/button';
import { Modal } from '@/components/ui/modal';
import { SelectMenu } from '@/components/ui/SelectMenu';
import { FormField, PasswordInput, TextInput } from '@/components/hosts/formFields';import { ControlHintTooltip } from '@/components/ControlHintTooltip';
import { useAnchoredPanel } from '@/hooks/useAnchoredPanel';
import { useDismissOnOutsidePointer } from '@/hooks/useDismissOnOutsidePointer';
import {
  dbConnectionDelete,
  dbConnectionList,
  dbConnectionSave,
  dbConnectionTest,
  dbListDatabasesForRecord,
  oracleClientDirGet,
  oracleClientDirPick,
  oracleClientDirSet,
  sqliteFilePick,
  type DbConnectionRecord,
  type DbConnectionTestResult,
  type DbEngineKind,
  type SqliteEncryptionMode,
} from '@/services/dbConnectionsService';
import { cn } from '@/lib/utils';

const ENGINE_DEFAULT_PORTS: Record<DbEngineKind, number> = {
  mysql: 3306,
  postgres: 5432,
  oracle: 1521,
  oceanbase: 2881,
  sqlite: 0,
  dameng: 5236,
};

// 引擎名均为产品名（英文界面也保留原厂名），不走 i18n；达梦沿用其中文产品名
const ENGINE_LABELS: Record<DbEngineKind, string> = {
  mysql: 'MySQL',
  postgres: 'PostgreSQL',
  oracle: 'Oracle',
  oceanbase: 'OceanBase',
  sqlite: 'SQLite',
  dameng: '达梦 DM8',
};

/** 连接条目的引擎显示名：OceanBase 附带租户模式后缀（如「OceanBase · Oracle 模式」） */
function engineDisplayLabel(
  record: Pick<DbConnectionRecord, 'engine' | 'mode'>,
  t: (key: string) => string,
): string {
  if (record.engine === 'oceanbase' && record.mode) {
    return `${ENGINE_LABELS.oceanbase} · ${t(
      record.mode === 'oracle' ? 'dbHosts.obModeOracle' : 'dbHosts.obModeMysql',
    )}`;
  }
  return ENGINE_LABELS[record.engine];
}

/** SQLite 加密方式（sqlcipher-raw 的 password 填 64 位十六进制原始密钥） */
const SQLITE_ENCRYPTION_MODES: Array<{ value: SqliteEncryptionMode; labelKey: string }> = [
  { value: 'none', labelKey: 'dbHosts.encryption_none' },
  { value: 'sqlcipher4', labelKey: 'dbHosts.encryption_sqlcipher4' },
  { value: 'sqlcipher3', labelKey: 'dbHosts.encryption_sqlcipher3' },
  { value: 'sqlcipher2', labelKey: 'dbHosts.encryption_sqlcipher2' },
  { value: 'sqlcipher-raw', labelKey: 'dbHosts.encryption_sqlcipher_raw' },
];

const SSL_MODES: Array<{ value: DbConnectionRecord['sslMode']; labelKey: string }> = [
  { value: 'disable', labelKey: 'dbHosts.ssl_disable' },
  { value: 'prefer', labelKey: 'dbHosts.ssl_prefer' },
  { value: 'require', labelKey: 'dbHosts.ssl_require' },
  { value: 'verify-full', labelKey: 'dbHosts.ssl_verify_full' },
];

const ENV_LEVELS = ['dev', 'test', 'staging', 'prod'] as const;

export function DbHostsScreen({
  onConnectConnection,
}: {
  onConnectConnection: (record: DbConnectionRecord) => void;
}) {
  const { t } = useTranslation();
  const [records, setRecords] = useState<DbConnectionRecord[]>([]);
  const [editing, setEditing] = useState<DbConnectionRecord | null>(null);
  const [deleting, setDeleting] = useState<DbConnectionRecord | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  // Oracle 客户端库目录设置弹层（进程级一次初始化，改目录需重启生效）
  const [clientDirOpen, setClientDirOpen] = useState(false);
  const [clientDir, setClientDir] = useState('');
  const [clientDirSaved, setClientDirSaved] = useState<string | null>(null);

  const refresh = async () => {
    try {
      setRecords(await dbConnectionList());
      setError(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    void refresh();
  }, []);

  const openClientDir = async () => {
    setClientDirOpen(true);
    setClientDirSaved(null);
    try {
      setClientDir((await oracleClientDirGet()) ?? '');
    } catch {
      setClientDir('');
    }
  };

  const saveClientDir = async (dir: string | null) => {
    try {
      await oracleClientDirSet(dir && dir.trim() ? dir.trim() : null);
      setClientDirSaved(dir && dir.trim() ? dir.trim() : '');
    } catch (e) {
      setError(String(e));
    }
  };

  const handleDelete = async () => {
    if (!deleting) return;
    try {
      await dbConnectionDelete(deleting.id);
      setDeleting(null);
      await refresh();
    } catch (e) {
      setError(String(e));
    }
  };

  const newRecord = (): DbConnectionRecord => ({
    id: 0,
    name: '',
    engine: 'mysql',
    mode: null,
    host: '',
    port: 3306,
    database: null,
    serviceName: null,
    dbPath: null,
    encryptionMode: 'none',
    compatibleMode: null,
    username: null,
    password: null,
    sslMode: 'disable',
    sshHostId: null,
    envLevel: 'dev',
    readonly: true,
    color: null,
    createdAt: 0,
    updatedAt: 0,
  });

  return (
    <div className="flex h-full flex-col overflow-hidden">
      <div className="flex shrink-0 items-center justify-between border-b border-border/50 px-4 py-2">
        <span className="text-ui-sm text-foreground-subtle select-none">
          {t('dbHosts.summary', { total: records.length })}
        </span>
        <div className="flex items-center gap-1.5">
          {/* Oracle 客户端库目录（运行期 dlopen；缺库只报错不崩进程） */}
          <Button
            variant="ghost"
            size="lg"
            className="h-7 px-3 text-ui-xs"
            onClick={() => void openClientDir()}
          >
            <FolderOpen className="size-3.5" />
            {t('dbHosts.oracleClientDir')}
          </Button>
          <Button size="lg" className="h-7 px-3 text-ui-xs" onClick={() => setEditing(newRecord())}>
            <Plus className="size-3.5" />
            {t('dbHosts.newConnection')}
          </Button>
        </div>
      </div>

      {error ? (
        <div className="flex shrink-0 items-center justify-between gap-2 border-b border-destructive/20 bg-destructive/10 px-4 py-1.5 text-ui-xs text-destructive">
          <span className="min-w-0 flex-1 truncate">{error}</span>
          <button
            type="button"
            onClick={() => setError(null)}
            className="opacity-70 hover:opacity-100"
          >
            ✕
          </button>
        </div>
      ) : null}

      <div className="flex min-h-0 flex-1 flex-col gap-1.5 overflow-y-auto p-3">
        {loading ? (
          <span className="text-ui-sm text-foreground-subtlest">{t('dbHosts.loading')}</span>
        ) : records.length === 0 ? (
          <div className="flex flex-1 flex-col items-center justify-center gap-2 select-none">
            <span className="text-ui-sm text-foreground-subtlest">{t('dbHosts.empty')}</span>
            <Button variant="outline" size="lg" onClick={() => setEditing(newRecord())}>
              {t('dbHosts.newConnection')}
            </Button>
          </div>
        ) : (
          records.map((record) => (
            <div
              key={record.id}
              className="group flex items-center gap-3 rounded-lg border border-border px-3 py-2 transition-colors hover:border-border-hover"
            >
              <Database
                className={cn(
                  'size-4 shrink-0',
                  record.engine === 'mysql'
                    ? 'text-terminal-cyan'
                    : record.engine === 'postgres'
                      ? 'text-terminal-blue'
                      : record.engine === 'oracle'
                        ? 'text-terminal-red'
                        : 'text-terminal-magenta',
                )}
              />
              <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                <span className="flex items-center gap-1.5 truncate text-ui-sm font-medium text-foreground">
                  {record.name}
                  {record.envLevel === 'prod' || record.envLevel === 'staging' ? (
                    <span
                      className={cn(
                        'rounded px-1 py-px font-mono text-[9px] font-semibold uppercase',
                        record.envLevel === 'prod'
                          ? 'bg-destructive/15 text-destructive'
                          : 'bg-amber-500/15 text-amber-500',
                      )}
                    >
                      {record.envLevel}
                    </span>
                  ) : null}
                  {record.readonly ? (
                    <span className="rounded bg-surface px-1 py-px font-mono text-[9px] text-foreground-subtle uppercase">
                      {t('dbHosts.readonlyBadge')}
                    </span>
                  ) : null}
                  {/* 达梦的兼容模式决定客户端 SQL 方言，列表里值得一眼可见 */}
                  {record.engine === 'dameng' ? (
                    <span className="rounded bg-surface px-1 py-px font-mono text-[9px] text-foreground-subtle uppercase">
                      {record.compatibleMode === 'mysql'
                        ? t('dbHosts.compatBadgeMysql')
                        : t('dbHosts.compatBadgeOracle')}
                    </span>
                  ) : null}
                </span>
                <span className="truncate font-mono text-ui-xs text-foreground-subtle">
                  {record.engine === 'sqlite'
                    ? `${ENGINE_LABELS[record.engine]} · ${record.dbPath ?? ':memory:'}`
                    : `${engineDisplayLabel(record, t)} · ${record.host}:${record.port}${
                        record.database ? `/${record.database}` : ''
                      } · ${t(`dbHosts.ssl_${record.sslMode.replace('-', '_')}`)}`}
                </span>
              </div>
              <span className="flex shrink-0 items-center gap-0.5">
                <ControlHintTooltip title={t('dbHosts.connect')} side="bottom">
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    className="size-7 rounded-lg text-foreground-subtle hover:text-primary"
                    onClick={() => onConnectConnection(record)}
                  >
                    <Play className="size-3.5" />
                  </Button>
                </ControlHintTooltip>
                <ControlHintTooltip title={t('common.edit')} side="bottom">
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    className="size-7 rounded-lg text-foreground-subtle hover:text-foreground"
                    onClick={() => setEditing(record)}
                  >
                    <Pencil className="size-3.5" />
                  </Button>
                </ControlHintTooltip>
                <ControlHintTooltip title={t('common.delete')} side="bottom">
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    className="size-7 rounded-lg text-foreground-subtle hover:text-destructive"
                    onClick={() => setDeleting(record)}
                  >
                    <Trash2 className="size-3.5" />
                  </Button>
                </ControlHintTooltip>
              </span>
            </div>
          ))
        )}
      </div>

      {editing ? (
        <DbConnectionFormDialog
          node={editing.id > 0 ? editing : null}
          onSaved={() => {
            setEditing(null);
            void refresh();
          }}
          onClose={() => setEditing(null)}
        />
      ) : null}

      {deleting ? (
        <Modal
          title={t('dbHosts.deleteTitle')}
          onClose={() => setDeleting(null)}
          className="w-[420px]"
          footer={
            <>
              <Button
                variant="ghost"
                size="lg"
                className="h-8 px-3 text-ui-base"
                onClick={() => setDeleting(null)}
              >
                {t('common.cancel')}
              </Button>
              <Button
                variant="destructive"
                size="lg"
                className="h-8 px-3 text-ui-base"
                onClick={() => void handleDelete()}
              >
                {t('common.delete')}
              </Button>
            </>
          }
        >
          <p className="text-ui-sm text-foreground">
            {t('dbHosts.deleteConfirm', { name: deleting.name })}
          </p>
        </Modal>
      ) : null}

      {/* Oracle 客户端库目录设置：ODPI-C 运行期 dlopen，指定 Instant Client
          目录（留空走系统搜索路径）；进程级一次初始化——已连接过 Oracle 后
          改目录需重启应用生效 */}
      {clientDirOpen ? (
        <Modal
          onClose={() => setClientDirOpen(false)}
          title={t('dbHosts.oracleClientDir')}
          className="w-[520px]"
          footer={
            <>
              <Button variant="outline" className="h-8 px-3 text-ui-base" onClick={() => void saveClientDir(null)}>
                {t('dbHosts.oracleClientDirClear')}
              </Button>
              <Button
                className="h-8 px-3 text-ui-base"
                onClick={async () => {
                  await saveClientDir(clientDir);
                }}
              >
                {t('common.save')}
              </Button>
            </>
          }
        >
          <div className="flex flex-col gap-3">
            <FormField label={t('dbHosts.oracleClientDirLabel')}>
              <div className="flex gap-2">
                <TextInput
                  value={clientDir}
                  onChange={(e) => setClientDir(e.target.value)}
                  placeholder={t('dbHosts.oracleClientDirPlaceholder')}
                  className="min-w-0 flex-1"
                />
                <Button
                  variant="outline"
                  className="h-8 shrink-0 px-3 text-ui-base"
                  onClick={async () => {
                    const picked = await oracleClientDirPick().catch(() => null);
                    if (picked) setClientDir(picked);
                  }}
                >
                  <FolderOpen className="size-3.5" />
                  {t('dbHosts.oracleClientDirBrowse')}
                </Button>
              </div>
            </FormField>
            <p className="text-ui-xs text-foreground-subtle">
              {t('dbHosts.oracleClientDirHint')}
            </p>
            {clientDirSaved != null ? (
              <p className="text-ui-xs text-terminal-green">
                {clientDirSaved
                  ? t('dbHosts.oracleClientDirSaved', { dir: clientDirSaved })
                  : t('dbHosts.oracleClientDirCleared')}
              </p>
            ) : null}
          </div>
        </Modal>
      ) : null}
    </div>
  );
}

/** 数据库字段：手输 + 右侧下拉按钮（用表单当前参数临时建连拉库清单，portal
 * 弹层选择回填；编辑态密码留空时后端自动从库补）——库清单选择器。 */
function DatabasePickerField({
  value,
  onChange,
  canLoad,
  buildRecord,
  connectionId,
  placeholder,
  pickLabel,
}: {
  value: string;
  onChange: (v: string) => void;
  canLoad: boolean;
  buildRecord: () => DbConnectionRecord;
  connectionId: number | undefined;
  /** 输入占位 / 选择按钮 title（按引擎文案） */
  placeholder?: string;
  pickLabel?: string;
}) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [databases, setDatabases] = useState<string[]>([]);
  const { anchorRef, panelRef, position, measure } = useAnchoredPanel(open);
  useDismissOnOutsidePointer(open, [anchorRef, panelRef], () => setOpen(false));

  const load = async () => {
    if (loading || !canLoad) return;
    setLoading(true);
    setError(null);
    try {
      const list = await dbListDatabasesForRecord(buildRecord(), connectionId);
      setDatabases(list);
      if (list.length === 0) {
        setError(t('dbHosts.noDatabases'));
        return;
      }
      // 先量锚点再开面板（AnchoredPanel 按钮锚 + 视口夹取）
      measure();
      setOpen(true);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className="flex flex-col gap-1">
      <div ref={anchorRef} className="flex gap-1.5">
        <TextInput
          value={value}
          onChange={(e) => onChange(e.target.value)}
          placeholder={placeholder ?? t('dbHosts.databasePlaceholder')}
          className="font-mono"
        />
        <Button
          variant="outline"
          size="icon-sm"
          className="size-8 shrink-0 rounded-md"
          disabled={!canLoad || loading}
          onClick={() => void load()}
          aria-label={pickLabel ?? t('dbHosts.pickDatabase')}
          title={pickLabel ?? t('dbHosts.pickDatabase')}
        >
          {loading ? (
            <Loader2 className="size-3.5 animate-spin" />
          ) : (
            <Database className="size-3.5" />
          )}
        </Button>
      </div>
      {error ? (
        <span className="break-all text-ui-xs text-destructive">{error}</span>
      ) : null}
      {open && position
        ? createPortal(
            <div
              ref={panelRef}
              role="listbox"
              style={{ position: 'fixed', top: position.top, left: position.left }}
              className="z-[110] max-h-64 min-w-44 overflow-y-auto rounded-lg border border-border bg-menu p-1 shadow-lg scrollbar-slim"
            >
              {databases.map((db) => (
                <button
                  key={db}
                  type="button"
                  role="option"
                  aria-selected={db === value}
                  onClick={() => {
                    onChange(db);
                    setOpen(false);
                  }}
                  className={cn(
                    'flex w-full items-center gap-1.5 rounded-md px-2 py-1.5 text-left font-mono text-ui-xs transition-colors',
                    db === value
                      ? 'bg-selected font-semibold text-foreground'
                      : 'text-foreground-subtle hover:bg-menu-hover hover:text-foreground',
                  )}
                >
                  <Database className="size-3 shrink-0 text-terminal-cyan" />
                  <span className="min-w-0 flex-1 truncate">{db}</span>
                </button>
              ))}
            </div>,
            document.body,
          )
        : null}
    </div>
  );
}

function DbConnectionFormDialog({
  node,
  onSaved,
  onClose,
}: {
  node: DbConnectionRecord | null;
  onSaved: () => void;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const [name, setName] = useState(node?.name ?? '');
  const [engine, setEngine] = useState<DbEngineKind>(node?.engine ?? 'mysql');
  // OceanBase 专有：租户模式（MySQL / Oracle 二选一）；非 oceanbase 引擎恒 null
  const [mode, setMode] = useState<string | null>(
    node?.engine === 'oceanbase' ? (node.mode === 'oracle' ? 'oracle' : 'mysql') : null,
  );
  const [host, setHost] = useState(node?.host ?? '');
  const [port, setPort] = useState(String(node?.port ?? 3306));
  const [database, setDatabase] = useState(node?.database ?? '');
  const [serviceName, setServiceName] = useState(node?.serviceName ?? '');
  // SQLite：文件路径（留空 = 内存库）+ 加密方式
  const [dbPath, setDbPath] = useState(node?.dbPath ?? '');
  const [encryptionMode, setEncryptionMode] = useState<SqliteEncryptionMode>(
    node?.encryptionMode ?? 'none',
  );
  // 达梦兼容模式：空/未知一律回显 oracle（与后端 dameng_dialect 的默认一致）
  const [compatibleMode, setCompatibleMode] = useState<'oracle' | 'mysql'>(
    node?.compatibleMode === 'mysql' ? 'mysql' : 'oracle',
  );
  const [username, setUsername] = useState(node?.username ?? '');
  // 回显已存密码（默认掩码，点眼睛可查看明文——PasswordInput 自带切换）；
  // 用户不动它保存即原值写回；主动清空保存则保留库中旧值（后端 None=保留语义）
  const [password, setPassword] = useState(node?.password ?? '');
  const [sslMode, setSslMode] = useState<DbConnectionRecord['sslMode']>(node?.sslMode ?? 'disable');
  const [envLevel, setEnvLevel] = useState(node?.envLevel ?? 'dev');
  const [readonly, setReadonly] = useState(node?.readonly ?? true);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<DbConnectionTestResult | null>(null);
  const savingRef = useRef(false);

  const dbLabel =
    engine === 'mysql'
      ? t('dbHosts.databaseLabelMysql')
      : engine === 'postgres'
        ? t('dbHosts.databaseLabelPg')
        : engine === 'oracle'
          ? t('dbHosts.databaseLabelOracle')
          : engine === 'dameng'
            ? t('dbHosts.databaseLabelDameng')
            : mode === 'oracle'
              ? t('dbHosts.databaseLabelObOracle')
              : t('dbHosts.databaseLabelObMysql');

  // Oracle / OB Oracle 租户模式按 Oracle 语义展示 database 字段（初始 Schema）
  const isOracleSemantics = engine === 'oracle' || (engine === 'oceanbase' && mode === 'oracle');
  // OB Oracle 租户模式后端 P0 仅支持非加密连接（disable）
  const obOracleTlsLocked = engine === 'oceanbase' && mode === 'oracle';

  const isSqlite = engine === 'sqlite';

  // SQLite 原始密钥必须 64 位十六进制（前端先挡一层，后端仍会校验）
  const rawKeyOk =
    encryptionMode !== 'sqlcipher-raw' || /^[0-9a-fA-F]{64}$/.test(password.trim());
  // 编辑态留空 = 保留库中旧口令（后端 fill_password_from_store 会补）——
  // 但前提是旧记录本来就有口令（旧记录不加密时库里没有口令可保留，
  // 改成加密后必须重新填，否则保存成功、连接时才报「需要提供口令」）
  const hasStoredKey = node != null && node.encryptionMode !== 'none';
  const sqlitePasswordOk =
    encryptionMode === 'none' || password.length > 0 || (hasStoredKey && password.length === 0);

  // Oracle：服务名必填（连库必需）；database 降为可选的初始 schema。
  // SQLite 无主机/端口概念（dbPath 留空 = 内存库），校验走口令分支。
  const canSave =
    !saving &&
    name.trim().length > 0 &&
    (isSqlite
      ? rawKeyOk && sqlitePasswordOk
      : host.trim().length > 0 && (engine !== 'oracle' || serviceName.trim().length > 0));

  // 测试连接：SQLite 允许内存库（不加密时 dbPath 可留空），其余引擎仍需主机
  const canTest = isSqlite
    ? dbPath.trim().length > 0 || encryptionMode === 'none'
    : host.trim().length > 0;

  const buildRecord = (): DbConnectionRecord => ({
    id: node?.id ?? 0,
    name: name.trim(),
    engine,
    mode: engine === 'oceanbase' ? (mode === 'oracle' ? 'oracle' : 'mysql') : null,
    host: isSqlite ? '' : host.trim(),
    port: isSqlite ? 0 : Number.parseInt(port, 10) || ENGINE_DEFAULT_PORTS[engine],
    database: isSqlite ? null : database.trim() || null,
    serviceName: engine === 'oracle' ? serviceName.trim() || null : null,
    dbPath: isSqlite ? dbPath.trim() || null : null,
    encryptionMode: isSqlite ? encryptionMode : 'none',
    // 仅达梦有意义；其余引擎写 null（后端忽略该列）
    compatibleMode: engine === 'dameng' ? compatibleMode : null,
    username: isSqlite ? null : username.trim() || null,
    // 留空 = 更新时保留旧值（后端语义）；新建时空值即无密码。
    // 例外：SQLite 从加密改回不加密时必须显式清空——否则库里残留的旧口令
    // 会被带去打开明文库，SQLCipher 直接报 NOTADB（「口令错误或不是加密数据库」）
    password:
      isSqlite && encryptionMode === 'none'
        ? node != null
          ? ''
          : null
        : password.length > 0 || node == null
          ? password || null
          : null,
    // SQLite 无 TLS、OB Oracle 租户模式 P0 仅 disable、达梦驱动无 TLS——三者强制归一
    sslMode: isSqlite || obOracleTlsLocked || engine === 'dameng' ? 'disable' : sslMode,
    sshHostId: null,
    envLevel,
    readonly,
    color: null,
    createdAt: 0,
    updatedAt: 0,
  });

  /** 切换 OB 租户模式：端口仍是任一引擎默认值（用户未自定义）时归位 2881；
   * Oracle 租户模式后端 P0 仅支持 disable，切入即强制回落 */
  const switchObMode = (next: 'mysql' | 'oracle') => {
    if (mode === next) return;
    setMode(next);
    const portNum = Number.parseInt(port, 10);
    if (Object.values(ENGINE_DEFAULT_PORTS).includes(portNum)) {
      setPort(String(ENGINE_DEFAULT_PORTS.oceanbase));
    }
    if (next === 'oracle') setSslMode('disable');
  };

  const handleTest = async () => {
    if (!canTest || testing) return;
    setTesting(true);
    setTestResult(null);
    try {
      // 编辑态密码被清空时传 connection_id：后端从库里补上已存密码再测，
      // 测试结果永远反映真实保存状态
      setTestResult(await dbConnectionTest(buildRecord(), node?.id));
    } catch (e) {
      setTestResult({
        ok: false,
        elapsedMs: 0,
        serverVersion: null,
        message: String(e),
      });
    } finally {
      setTesting(false);
    }
  };

  const handleSave = async () => {
    if (!canSave || savingRef.current) return;
    setSaving(true);
    savingRef.current = true;
    try {
      await dbConnectionSave(buildRecord());
      onSaved();
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
      savingRef.current = false;
    }
  };

  return (
    <Modal
      title={node ? t('dbHosts.editTitle', { name: node.name }) : t('dbHosts.newTitle')}
      onClose={onClose}
      className="w-[480px]"
      footer={
        <>
          <Button
            variant="outline"
            size="lg"
            className="h-8 px-3 text-ui-base"
            disabled={!canTest || testing}
            onClick={() => void handleTest()}
          >
            {testing ? (
              <Loader2 className="size-3.5 animate-spin" />
            ) : (
              <Zap className="size-3.5" />
            )}
            {t('dbHosts.testConnection')}
          </Button>
          <div className="flex-1" />
          <Button variant="ghost" size="lg" className="h-8 px-3 text-ui-base" onClick={onClose}>
            {t('common.cancel')}
          </Button>
          <Button
            size="lg"
            className="h-8 px-4 text-ui-base"
            disabled={!canSave}
            onClick={() => void handleSave()}
          >
            {t('common.save')}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-3.5">
        <FormField label={t('dbHosts.nameLabel')}>
          <TextInput
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder={t('dbHosts.namePlaceholder')}
            autoFocus
          />
        </FormField>

        <FormField label={t('dbHosts.engineLabel')}>
          <div className="flex gap-1.5">
            {(['mysql', 'postgres', 'oracle', 'oceanbase', 'sqlite', 'dameng'] as const).map(
              (kind) => (
              <button
                key={kind}
                type="button"
                onClick={() => {
                  setEngine(kind);
                  setPort(String(ENGINE_DEFAULT_PORTS[kind]));
                  // SQLite 无 TLS：切过去时把 SSL 归一为 disable，避免把 require 存进库
                  if (kind === 'sqlite') setSslMode('disable');
                  // 切到 oceanbase 时归一化租户模式（null → 'mysql'，已选则保留）；
                  // 切离时不清空——buildRecord 按 engine 门控，非 oceanbase 恒 null
                  if (kind === 'oceanbase') {
                    setMode((cur) => (cur === 'oracle' ? 'oracle' : 'mysql'));
                  }
                }}
                className={cn(
                  'flex flex-1 items-center justify-center gap-1.5 rounded-md border px-2.5 py-1.5 text-ui-xs transition-colors',
                  engine === kind
                    ? 'border-primary/60 bg-primary/10 font-semibold text-foreground'
                    : 'border-border/70 text-foreground-subtle hover:bg-hover hover:text-foreground',
                )}
                title={ENGINE_LABELS[kind]}
              >
                {ENGINE_LABELS[kind]}
              </button>
            ))}
          </div>
        </FormField>

        {/* OceanBase：租户模式分段选择（MySQL / Oracle 二选一，决定方言与凭据格式）；
            引擎按钮组的缩小版形态 */}
        {engine === 'oceanbase' ? (
          <FormField label={t('dbHosts.obModeLabel')} hint={t('dbHosts.obModeHint')}>
            <div className="flex gap-1.5">
              {(['mysql', 'oracle'] as const).map((m) => (
                <button
                  key={m}
                  type="button"
                  onClick={() => switchObMode(m)}
                  className={cn(
                    'flex flex-1 items-center justify-center rounded-md border px-2 py-1 text-ui-xs transition-colors',
                    (mode ?? 'mysql') === m
                      ? 'border-primary/60 bg-primary/10 font-semibold text-foreground'
                      : 'border-border/70 text-foreground-subtle hover:bg-hover hover:text-foreground',
                  )}
                >
                  {t(m === 'mysql' ? 'dbHosts.obModeMysql' : 'dbHosts.obModeOracle')}
                </button>
              ))}
            </div>
          </FormField>
        ) : null}

        {/* 达梦：兼容模式只影响客户端 SQL 方言，与 dm.ini 的 COMPATIBLE_MODE 无关 */}
        {engine === 'dameng' ? (
          <div className="flex flex-col gap-1.5">
            <FormField label={t('dbHosts.compatibleModeLabel')} className="w-36">
              <SelectMenu
                value={compatibleMode}
                onChange={(v) => setCompatibleMode(v as 'oracle' | 'mysql')}
                options={[
                  { value: 'oracle', label: t('dbHosts.compatibleModeOracle') },
                  { value: 'mysql', label: t('dbHosts.compatibleModeMysql') },
                ]}
              />
            </FormField>
            <p className="text-ui-xs text-foreground-subtlest">
              {t('dbHosts.compatibleModeHint')}
            </p>
          </div>
        ) : null}

        {isSqlite ? (
          <>
            <FormField label={t('dbHosts.dbPathLabel')} hint={t('dbHosts.dbPathMemoryHint')}>
              <div className="flex gap-1.5">
                <TextInput
                  value={dbPath}
                  onChange={(e) => setDbPath(e.target.value)}
                  placeholder={t('dbHosts.dbPathPlaceholder')}
                  className="font-mono"
                />
                <Button
                  variant="outline"
                  size="icon-sm"
                  className="size-8 shrink-0 rounded-md"
                  onClick={() => {
                    void sqliteFilePick()
                      .then((picked) => {
                        if (picked) setDbPath(picked);
                      })
                      .catch(() => undefined);
                  }}
                  aria-label={t('dbHosts.dbPathBrowse')}
                  title={t('dbHosts.dbPathPickTitle')}
                >
                  <FolderOpen className="size-3.5" />
                </Button>
              </div>
            </FormField>

            <div className="flex gap-3">
              <div className="min-w-0 flex-1">
                <FormField label={t('dbHosts.encryptionLabel')}>
                  <SelectMenu
                    value={encryptionMode}
                    onChange={(v) => setEncryptionMode(v as SqliteEncryptionMode)}
                    options={SQLITE_ENCRYPTION_MODES.map((m) => ({
                      value: m.value,
                      label: t(m.labelKey),
                    }))}
                  />
                </FormField>
              </div>
              {/* 不加密时无需口令（条件渲染而非 disabled，语义更直接） */}
              {encryptionMode !== 'none' ? (
                <div className="min-w-0 flex-1">
                  <FormField
                    label={t('dbHosts.sqlitePasswordLabel')}
                    hint={
                      hasStoredKey && password.length === 0
                        ? t('dbHosts.sqlitePasswordKeepHint')
                        : t('dbHosts.sqlitePasswordHint')
                    }
                  >
                    <PasswordInput
                      value={password}
                      onChange={(e) => setPassword(e.target.value)}
                      placeholder={
                        hasStoredKey && password.length === 0
                          ? t('dbHosts.passwordKeep')
                          : '••••••'
                      }
                    />
                  </FormField>
                </div>
              ) : null}
            </div>

            {/* 与 canSave 同一判定：加密但既没填、库里也没旧口令时提示 */}
            {encryptionMode !== 'none' && !sqlitePasswordOk ? (
              <span className="text-ui-xs text-destructive">
                {t('dbHosts.sqlitePasswordRequired')}
              </span>
            ) : null}
            {encryptionMode === 'sqlcipher-raw' && password.length > 0 && !rawKeyOk ? (
              <span className="text-ui-xs text-destructive">
                {t('dbHosts.sqliteRawKeyInvalid')}
              </span>
            ) : null}
            <p className="text-ui-xs text-foreground-subtlest">{t('dbHosts.sqliteFileHint')}</p>
          </>
        ) : (
          <>
            <div className="flex gap-3">
              <div className="min-w-0 flex-[2]">
                <FormField label={t('dbHosts.hostLabel')}>
                  <TextInput
                    value={host}
                    onChange={(e) => setHost(e.target.value)}
                    placeholder="10.0.0.5"
                    className="font-mono"
                  />
                </FormField>
              </div>
              <div className={cn('shrink-0', engine === 'oceanbase' ? 'w-32' : 'w-24')}>
                <FormField label={t('dbHosts.portLabel')}>
                  <TextInput
                    value={port}
                    onChange={(e) => setPort(e.target.value)}
                    placeholder={String(ENGINE_DEFAULT_PORTS[engine])}
                    className="font-mono"
                  />
                </FormField>
              </div>
            </div>
            {/* OceanBase：直连 2881 / 经 obproxy 2883 的端口提示——窄端口列
                放不下整句，作整行提示置于 host/port 行下方（sqliteFileHint
                同型） */}
            {engine === 'oceanbase' ? (
              <p className="text-ui-xs text-foreground-subtlest select-none">
                {t('dbHosts.obPortHint')}
              </p>
            ) : null}
          </>
        )}

        {/* Oracle：服务名行（必填，连库必需；SSL 同行）；MySQL/PG 无此行 */}
        {engine === 'oracle' ? (
          <div className="flex gap-3">
            <div className="min-w-0 flex-1">
              <FormField label={t('dbHosts.serviceNameLabel')}>
                <TextInput
                  value={serviceName}
                  onChange={(e) => setServiceName(e.target.value)}
                  placeholder="ORCLPDB1"
                  className="font-mono"
                />
              </FormField>
            </div>
            <div className="w-36 shrink-0">
              <FormField label={t('dbHosts.sslLabel')}>
                <SelectMenu
                  value={sslMode}
                  onChange={(v) => setSslMode(v as DbConnectionRecord['sslMode'])}
                  options={SSL_MODES.map((m) => ({ value: m.value, label: t(m.labelKey) }))}
                />
              </FormField>
            </div>
          </div>
        ) : null}

        {isSqlite ? null : (
          <div className="flex gap-3">
            <div className="min-w-0 flex-1">
              <FormField label={dbLabel}>
                <DatabasePickerField
                  value={database}
                  onChange={setDatabase}
                  canLoad={
                    engine === 'oracle'
                      ? host.trim().length > 0 && serviceName.trim().length > 0
                      : host.trim().length > 0
                  }
                  buildRecord={() => buildRecord()}
                  connectionId={node?.id}
                  placeholder={
                    isOracleSemantics || engine === 'dameng'
                      ? t('dbHosts.schemaPlaceholder')
                      : t('dbHosts.databasePlaceholder')
                  }
                  pickLabel={
                    isOracleSemantics || engine === 'dameng'
                      ? t('dbHosts.pickSchema')
                      : t('dbHosts.pickDatabase')
                  }
                />
              </FormField>
            </div>
            {engine !== 'oracle' ? (
              <div className="w-36 shrink-0">
                {/* 达梦只有登录加密、没有 TLS 链路；OB Oracle 租户模式 P0 仅
                    disable——两者的下拉都禁用并固定 disable；提示作下拉下方
                    独立行（达梦同型）——不用 FormField 的 hint（长文案会把
                    label 行撑高折行，与左列错位） */}
                <FormField label={t('dbHosts.sslLabel')}>
                  <SelectMenu
                    value={obOracleTlsLocked || engine === 'dameng' ? 'disable' : sslMode}
                    onChange={(v) => setSslMode(v as DbConnectionRecord['sslMode'])}
                    options={SSL_MODES.map((m) => ({ value: m.value, label: t(m.labelKey) }))}
                    disabled={obOracleTlsLocked || engine === 'dameng'}
                  />
                </FormField>
                {obOracleTlsLocked || engine === 'dameng' ? (
                  <p className="mt-1 text-ui-xs text-foreground-subtlest">
                    {t(
                      obOracleTlsLocked
                        ? 'dbHosts.obOracleTlsHint'
                        : 'dbHosts.sslUnsupportedDameng',
                    )}
                  </p>
                ) : null}
              </div>
            ) : null}
          </div>
        )}

        {/* SQLite 无用户名/密码凭证：口令字段已在上面的加密区块接管 */}
        {isSqlite ? null : (
          <div className="flex gap-3">
            <div className="min-w-0 flex-1">
              <FormField label={t('dbHosts.userLabel')}>
                <TextInput
                  value={username}
                  onChange={(e) => setUsername(e.target.value)}
                  placeholder={engine === 'oceanbase' ? t('dbHosts.obUserPlaceholder') : 'root'}
                  className="font-mono"
                />
              </FormField>
            </div>
            <div className="min-w-0 flex-1">
              <FormField
                label={t('dbHosts.passwordLabel')}
                hint={
                  node != null && password.length === 0
                    ? t('dbHosts.passwordClearedHint')
                    : node != null
                      ? t('dbHosts.passwordSavedHint')
                      : undefined
                }
              >
                <PasswordInput
                  value={password}
                  onChange={(e) => setPassword(e.target.value)}
                  placeholder={
                    node != null && password.length === 0 ? t('dbHosts.passwordKeep') : '••••••'
                  }
                />
              </FormField>
            </div>
          </div>
        )}

        <div className="flex gap-3">
          <div className="w-36 shrink-0">
            <FormField label={t('dbHosts.envLabel')}>
              <SelectMenu
                value={envLevel}
                onChange={setEnvLevel}
                options={ENV_LEVELS.map((level) => ({
                  value: level,
                  label: t(`dbHosts.env_${level}`),
                }))}
              />
            </FormField>
          </div>
          <div className="flex min-w-0 flex-1 items-end pb-1.5">
            <button
              type="button"
              onClick={() => setReadonly((v) => !v)}
              className="flex items-center gap-2 text-ui-xs text-foreground-subtle select-none hover:text-foreground"
            >
              <span
                className={cn(
                  'relative h-4 w-7 rounded-full transition-colors',
                  readonly ? 'bg-primary' : 'bg-border',
                )}
              >
                <span
                  className={cn(
                    'absolute top-0.5 size-3 rounded-full bg-white transition-all',
                    readonly ? 'left-3.5' : 'left-0.5',
                  )}
                />
              </span>
              {t('dbHosts.readonlyToggle')}
            </button>
          </div>
        </div>

        {testResult ? (
          <div
            className={cn(
              'flex items-start gap-2 rounded-md border px-2.5 py-2 text-ui-xs',
              testResult.ok
                ? 'border-terminal-green/30 bg-terminal-green/10 text-terminal-green'
                : 'border-destructive/30 bg-destructive/10 text-destructive',
            )}
          >
            {testResult.ok ? (
              <CircleCheck className="mt-0.5 size-3.5 shrink-0" />
            ) : (
              <CircleX className="mt-0.5 size-3.5 shrink-0" />
            )}
            <div className="min-w-0 flex-1">
              {testResult.ok ? (
                <span className="break-all">
                  {t('dbHosts.testOk', { ms: testResult.elapsedMs })}
                  {testResult.serverVersion ? ` · ${testResult.serverVersion}` : ''}
                </span>
              ) : (
                <span className="break-all">{testResult.message}</span>
              )}
            </div>
          </div>
        ) : null}

        {error ? (
          <div className="rounded-md border border-destructive/30 bg-destructive/10 px-2.5 py-2 text-ui-xs text-destructive">
            <span className="break-all">{error}</span>
          </div>
        ) : null}
      </div>
    </Modal>
  );
}
