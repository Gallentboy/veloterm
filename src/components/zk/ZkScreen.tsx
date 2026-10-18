import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import {
  ChevronRight,
  Database,
  FilePlus,
  Plus,
  RefreshCw,
  Trash2,
  Unplug,
  X,
} from 'lucide-react';
import { Button } from '@/components/ui/button';
import { Modal } from '@/components/ui/modal';
import { SelectMenu } from '@/components/ui/SelectMenu';
import { FormField, PasswordInput, TextArea, TextInput } from '@/components/hosts/formFields';
import { ControlHintTooltip } from '@/components/ControlHintTooltip';
import {
  ACL_ALL,
  ACL_BITS,
  listenZkWatch,
  zkAuth,
  zkConnect,
  zkCreate,
  zkDelete,
  zkDigestId,
  zkDisconnect,
  zkGet,
  zkList,
  zkSetAcl,
  zkSetData,
  type ZkAclRule,
  type ZkNode,
} from '@/services/zkService';
import { relayList, type RelayConfig } from '@/services/relayConfigService';
import { cn } from '@/lib/utils';

interface TreeNode {
  name: string;
  path: string;
  depth: number;
  expanded: boolean;
  loaded: boolean;
  children: string[];
  ephemeral: boolean;
}

const PERM_KEYS = ['read', 'write', 'create', 'delete', 'admin'] as const;
type PermKey = (typeof PERM_KEYS)[number];

function timeOf(ms: number): string {
  if (ms === 0) return '—';
  return new Date(ms).toLocaleString();
}

/** ZooKeeper 管理页：多 tab 外壳（会话视图常驻，切页/切 tab 状态均保留）。
 * tab 双击重命名；关 tab = 断开会话；加号新建连接。 */
export function ZkScreen({ isVisible = true }: { isVisible?: boolean }) {
  const { t } = useTranslation();
  const [tabs, setTabs] = useState<Array<{ key: number; customTitle: string | null; baseTitle: string | null }>>([
    { key: 1, customTitle: null, baseTitle: null },
  ]);
  const [activeKey, setActiveKey] = useState(1);
  const nextKeyRef = useRef(2);
  const [renamingKey, setRenamingKey] = useState<number | null>(null);
  const [renameValue, setRenameValue] = useState('');

  const addTab = () => {
    const key = nextKeyRef.current;
    nextKeyRef.current += 1;
    setTabs((current) => [...current, { key, customTitle: null, baseTitle: null }]);
    setActiveKey(key);
  };

  const closeTab = (key: number) => {
    setTabs((current) => {
      const idx = current.findIndex((tab) => tab.key === key);
      const next = current.filter((tab) => tab.key !== key);
      if (key === activeKey) {
        const neighbor = next[Math.min(idx, next.length - 1)];
        setActiveKey(neighbor ? neighbor.key : 0);
      }
      return next;
    });
  };

  const commitRename = () => {
    if (renamingKey == null) return;
    const key = renamingKey;
    const value = renameValue.trim();
    setTabs((current) =>
      current.map((tab) => (tab.key === key ? { ...tab, customTitle: value.length > 0 ? value : null } : tab)),
    );
    setRenamingKey(null);
  };

  const setBaseTitle = (key: number, title: string | null) => {
    setTabs((current) => current.map((tab) => (tab.key === key ? { ...tab, baseTitle: title } : tab)));
  };

  if (!isVisible && tabs.length === 0) {
    // 不可见且无 tab：不渲染任何内容（避免后台空转）
    return null;
  }

  return (
    <div className="flex h-full flex-col overflow-hidden">
      {/* tab 栏 */}
      <div className="flex h-9 shrink-0 items-center gap-1 border-b border-border/50 px-2">
        <div className="flex min-w-0 flex-1 items-center gap-1 overflow-x-auto">
          {tabs.map((tab) => (
            <div
              key={tab.key}
              className={cn(
                'group/tab flex h-7 shrink-0 items-center gap-1 rounded-lg border px-2 transition-colors',
                tab.key === activeKey
                  ? 'border-border-hover bg-surface'
                  : 'border-transparent text-foreground-subtle hover:bg-surface-hover',
              )}
              onClick={() => setActiveKey(tab.key)}
              onDoubleClick={() => {
                setRenamingKey(tab.key);
                setRenameValue(tab.customTitle ?? tab.baseTitle ?? '');
              }}
            >
              <Database className="size-3.5 shrink-0" />
              {renamingKey === tab.key ? (
                <input
                  autoFocus
                  value={renameValue}
                  onChange={(e) => setRenameValue(e.target.value)}
                  onBlur={commitRename}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter') commitRename();
                    if (e.key === 'Escape') setRenamingKey(null);
                  }}
                  onClick={(e) => e.stopPropagation()}
                  className="w-28 rounded border border-border bg-background px-1 text-ui-xs text-foreground outline-none"
                />
              ) : (
                <span className="max-w-40 truncate text-ui-sm select-none">
                  {tab.customTitle ?? tab.baseTitle ?? t('zk.newTab')}
                </span>
              )}
              <button
                type="button"
                className="shrink-0 rounded p-0.5 text-foreground-subtlest opacity-0 transition-opacity group-hover/tab:opacity-100 hover:text-foreground"
                onClick={(e) => {
                  e.stopPropagation();
                  closeTab(tab.key);
                }}
              >
                <X className="size-3" />
              </button>
            </div>
          ))}
          <button
            type="button"
            className="flex size-7 shrink-0 items-center justify-center rounded-lg text-foreground-subtle transition-colors hover:bg-hover hover:text-foreground"
            onClick={addTab}
            title={t('zk.newTab')}
          >
            <Plus className="size-4" />
          </button>
        </div>
      </div>

      {/* 会话视图（常驻隐藏，状态保留；卸载即断开会话） */}
      {tabs.length === 0 ? (
        <div className="flex flex-1 items-center justify-center gap-2 select-none">
          <span className="text-ui-sm text-foreground-subtlest">{t('zk.noTabs')}</span>
          <Button variant="outline" size="lg" className="h-7 px-2.5 text-ui-xs" onClick={addTab}>
            {t('zk.newTab')}
          </Button>
        </div>
      ) : (
        tabs.map((tab) => (
          <div key={tab.key} className={cn('flex min-h-0 flex-1 flex-col', tab.key !== activeKey && 'hidden')}>
            <ZkSessionView onBaseTitle={(title) => setBaseTitle(tab.key, title)} />
          </div>
        ))
      )}
    </div>
  );
}

/** 单个 ZK 连接视图：连接（直连/经中继）+ 树 + 数据/ACL 编辑，全部状态内部留存。 */
function ZkSessionView({ onBaseTitle }: { onBaseTitle: (title: string | null) => void }) {
  const { t } = useTranslation();
  const [sessionId, setSessionId] = useState<string | null>(null);
  const [connecting, setConnecting] = useState(false);
  const [endpoints, setEndpoints] = useState('127.0.0.1:2181');
  const [relayEnabled, setRelayEnabled] = useState(false);
  const [relayConfigs, setRelayConfigs] = useState<RelayConfig[]>([]);
  const [relayPickedId, setRelayPickedId] = useState<number | null>(null);
  const [relayName, setRelayName] = useState<string | null>(null);

  // 中继库（侧栏「中继」页）加载一次，供下拉选择；只存 id，字段不回显前端
  useEffect(() => {
    void relayList().then(setRelayConfigs).catch(() => {
      // 库不可用时保持空列表，只能直连
    });
  }, []);

  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const [nodes, setNodes] = useState<Map<string, TreeNode>>(new Map());
  const [selected, setSelected] = useState('/');
  const [detail, setDetail] = useState<ZkNode | null>(null);
  const [detailLoading, setDetailLoading] = useState(false);
  const [draftData, setDraftData] = useState('');
  const [draftEncoding, setDraftEncoding] = useState<'utf8' | 'base64'>('utf8');
  const [draftAcls, setDraftAcls] = useState<ZkAclRule[]>([]);
  const [dirty, setDirty] = useState(false);
  const [aclDirty, setAclDirty] = useState(false);

  const [creating, setCreating] = useState(false);
  const [newName, setNewName] = useState('');
  const [newData, setNewData] = useState('');
  const [newEphemeral, setNewEphemeral] = useState(false);
  const [newSequential, setNewSequential] = useState(false);
  const [deleting, setDeleting] = useState<string | null>(null);
  const [authing, setAuthing] = useState(false);
  const [authUser, setAuthUser] = useState('');
  const [authPass, setAuthPass] = useState('');
  const [aclAddScheme, setAclAddScheme] = useState<string>('world');
  const [aclAddId, setAclAddId] = useState('anyone');
  const [aclAddDigestUser, setAclAddDigestUser] = useState('');
  const [aclAddDigestPass, setAclAddDigestPass] = useState('');

  const buildRelayId = (): number | null => {
    if (!relayEnabled) return null;
    return relayPickedId;
  };

  const loadChildren = useCallback(
    async (sid: string, path: string) => {
      const res = await zkList(sid, path);
      setNodes((current) => {
        const next = new Map(current);
        const existing = next.get(path);
        const childNames = [...res.children].sort();
        const ephemeralOwner = res.stat.ephemeralOwner !== 0;
        next.set(path, {
          name: path === '/' ? '/' : (path.split('/').pop() ?? path),
          path,
          depth: path === '/' ? 0 : path.split('/').length - 1,
          expanded: existing?.expanded ?? true,
          loaded: true,
          children: childNames,
          ephemeral: ephemeralOwner,
        });
        // 子节点占位（未加载）
        for (const name of childNames) {
          const childPath = path === '/' ? `/${name}` : `${path}/${name}`;
          if (!next.has(childPath)) {
            next.set(childPath, {
              name,
              path: childPath,
              depth: path === '/' ? 1 : path.split('/').length,
              expanded: false,
              loaded: false,
              children: [],
              ephemeral: false,
            });
          }
        }
        return next;
      });
    },
    [],
  );

  const selectNode = useCallback(
    async (sid: string, path: string) => {
      setSelected(path);
      setDetailLoading(true);
      setDetail(null);
      setDirty(false);
      setAclDirty(false);
      try {
        const node = await zkGet(sid, path);
        setDetail(node);
        setDraftData(node.data);
        setDraftEncoding(node.encoding);
        setDraftAcls(node.acls.map((a) => ({ ...a })));
      } catch (e) {
        setError(String(e));
      } finally {
        setDetailLoading(false);
      }
    },
    [],
  );

  const handleConnect = async () => {
    const relayId = buildRelayId();
    if (relayEnabled && relayId == null) {
      setError(t('zk.relayPickRequired'));
      return;
    }
    setConnecting(true);
    setError(null);
    setNotice(null);
    try {
      const res = await zkConnect(endpoints, relayId);
      setSessionId(res.sessionId);
      setRelayName(
        relayId != null ? (relayConfigs.find((r) => r.id === relayId)?.name ?? null) : null,
      );
      setNodes(new Map());
      setSelected('/');
      await loadChildren(res.sessionId, '/');
      await selectNode(res.sessionId, '/');
      onBaseTitle(endpoints);
    } catch (e) {
      setError(String(e));
    } finally {
      setConnecting(false);
    }
  };

  const handleDisconnect = async () => {
    if (sessionId) {
      try {
        await zkDisconnect(sessionId);
      } catch {
        // 幂等
      }
    }
    setSessionId(null);
    setRelayName(null);
    setNodes(new Map());
    setDetail(null);
    onBaseTitle(null);
    setNotice(t('zk.disconnected'));
  };

  useEffect(() => {
    return () => {
      if (sessionId) {
        void zkDisconnect(sessionId);
      }
    };
  }, [sessionId]);

  // watch 驱动自动刷新：children 事件重载对应树节点；data 事件在选中且
  // 未编辑时重载详情（编辑中不覆写，提示外部变更）
  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | null = null;
    void listenZkWatch((msg) => {
      if (disposed || msg.sessionId !== sessionId) return;
      if (msg.kind === 'children') {
        void loadChildren(msg.sessionId, msg.path).catch(() => {
          // 节点可能已被删除：从树上移除该分支
          setNodes((current) => {
            const next = new Map(current);
            for (const key of next.keys()) {
              if (key === msg.path || key.startsWith(`${msg.path}/`)) {
                next.delete(key);
              }
            }
            return next;
          });
        });
      } else if (msg.path === selected) {
        if (dirty || aclDirty) {
          setNotice(t('zk.externallyChanged', { path: msg.path }));
        } else {
          void selectNode(msg.sessionId, msg.path);
        }
      }
    }).then((fn) => {
      if (disposed) fn();
      else unlisten = fn;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
    // 事件回调内读取的 dirty/aclDirty/selected 取自当次渲染闭包，均为最新值
  }, [sessionId, selected, dirty, aclDirty, t, loadChildren, selectNode]);

  const orderedPaths = useMemo(() => {
    const list: TreeNode[] = [];
    const walk = (path: string) => {
      const node = nodes.get(path);
      if (!node) return;
      list.push(node);
      if (node.expanded && node.loaded) {
        for (const child of node.children) {
          walk(path === '/' ? `/${child}` : `${path}/${child}`);
        }
      }
    };
    walk('/');
    return list;
  }, [nodes]);

  const toggleExpand = async (node: TreeNode) => {
    if (!sessionId) return;
    setNodes((current) => {
      const next = new Map(current);
      const existing = next.get(node.path);
      if (existing) {
        next.set(node.path, { ...existing, expanded: !existing.expanded });
      }
      return next;
    });
    if (!node.loaded) {
      try {
        await loadChildren(sessionId, node.path);
      } catch (e) {
        setError(String(e));
      }
    }
  };

  const refresh = async () => {
    if (!sessionId) return;
    setError(null);
    try {
      await loadChildren(sessionId, selected === '/' ? '/' : selected.split('/').slice(0, -1).join('/') || '/');
      await loadChildren(sessionId, selected);
      await selectNode(sessionId, selected);
    } catch (e) {
      setError(String(e));
    }
  };

  const saveData = async () => {
    if (!sessionId || !detail) return;
    setError(null);
    try {
      await zkSetData(sessionId, selected, draftData, draftEncoding, detail.stat.version);
      await selectNode(sessionId, selected);
      setNotice(t('zk.dataSaved', { path: selected }));
    } catch (e) {
      setError(String(e));
    }
  };

  const saveAcls = async () => {
    if (!sessionId || !detail) return;
    setError(null);
    try {
      await zkSetAcl(sessionId, selected, draftAcls, detail.stat.aversion);
      await selectNode(sessionId, selected);
      setNotice(t('zk.aclSaved', { path: selected }));
    } catch (e) {
      setError(String(e));
    }
  };

  const handleCreate = async () => {
    if (!sessionId || !newName.trim()) return;
    setError(null);
    try {
      const res = await zkCreate(
        sessionId,
        selected,
        newName,
        newData,
        newEphemeral,
        newSequential,
      );
      setCreating(false);
      setNewName('');
      setNewData('');
      setNewEphemeral(false);
      setNewSequential(false);
      setNotice(t('zk.created', { path: res.createdPath }));
      // 刷新父节点并选中新节点
      await loadChildren(sessionId, selected);
      const parent = nodes.get(selected);
      if (parent && !parent.expanded) {
        setNodes((current) => {
          const next = new Map(current);
          const p = next.get(selected);
          if (p) next.set(selected, { ...p, expanded: true });
          return next;
        });
      }
      await loadChildren(sessionId, selected);
      await selectNode(sessionId, res.createdPath);
    } catch (e) {
      setError(String(e));
    }
  };

  const handleDelete = async () => {
    if (!sessionId || !deleting) return;
    setError(null);
    try {
      const target = deleting;
      setDeleting(null);
      await zkDelete(sessionId, target);
      setNotice(t('zk.deleted', { path: target }));
      const parentPath = target.split('/').slice(0, -1).join('/') || '/';
      await loadChildren(sessionId, parentPath);
      await selectNode(sessionId, parentPath);
    } catch (e) {
      setError(String(e));
    }
  };

  const handleAuth = async () => {
    if (!sessionId) return;
    setError(null);
    try {
      await zkAuth(sessionId, authUser, authPass);
      setAuthing(false);
      setNotice(t('zk.authOk'));
    } catch (e) {
      setError(String(e));
    }
  };

  const addAclRule = async () => {
    setError(null);
    try {
      let id = aclAddId.trim();
      if (aclAddScheme === 'digest') {
        if (!aclAddDigestUser || !aclAddDigestPass) {
          setError(t('zk.aclDigestRequired'));
          return;
        }
        id = await zkDigestId(aclAddDigestUser, aclAddDigestPass);
      } else if (aclAddScheme === 'auth') {
        id = '';
      } else if (aclAddScheme === 'world') {
        id = 'anyone';
      }
      setDraftAcls((current) => [
        ...current,
        { scheme: aclAddScheme, id, perms: ACL_ALL },
      ]);
      setAclDirty(true);
      setAclAddId(aclAddScheme === 'ip' ? '' : 'anyone');
      setAclAddDigestUser('');
      setAclAddDigestPass('');
    } catch (e) {
      setError(String(e));
    }
  };

  const toggleAclPerm = (index: number, key: PermKey) => {
    setDraftAcls((current) =>
      current.map((rule, i) =>
        i === index ? { ...rule, perms: rule.perms ^ ACL_BITS[key] } : rule,
      ),
    );
    setAclDirty(true);
  };

  const removeAclRule = (index: number) => {
    setDraftAcls((current) => current.filter((_, i) => i !== index));
    setAclDirty(true);
  };

  return (
    <div className="flex h-full flex-col overflow-hidden">
      {/* 连接工具栏 */}
      <div className="flex shrink-0 flex-wrap items-center gap-2 border-b border-border/50 px-4 py-2">
        {sessionId ? (
          <>
            <span className="flex items-center gap-1.5 text-ui-sm text-primary select-none">
              <Database className="size-3.5" />
              <span className="font-mono">{endpoints}</span>
              {relayName ? (
                <span className="text-ui-xs text-foreground-subtle">
                  {t('zk.viaRelay', { name: relayName })}
                </span>
              ) : null}
            </span>
            <div className="flex-1" />
            <ControlHintTooltip title={t('zk.authTitle')} side="bottom">
              <Button variant="outline" size="lg" className="h-7 px-2.5 text-ui-xs" onClick={() => setAuthing(true)}>
                {t('zk.auth')}
              </Button>
            </ControlHintTooltip>
            <ControlHintTooltip title={t('zk.refresh')} side="bottom">
              <Button variant="outline" size="lg" className="h-7 px-2.5 text-ui-xs" onClick={() => void refresh()}>
                <RefreshCw className="size-3.5" />
              </Button>
            </ControlHintTooltip>
            <ControlHintTooltip title={t('zk.disconnect')} side="bottom">
              <Button variant="outline" size="lg" className="h-7 px-2.5 text-ui-xs" onClick={() => void handleDisconnect()}>
                <Unplug className="size-3.5" />
              </Button>
            </ControlHintTooltip>
          </>
        ) : (
          <>
            <TextInput
              value={endpoints}
              onChange={(e) => setEndpoints(e.target.value)}
              placeholder="host:2181"
              className="w-64 font-mono"
              spellCheck={false}
            />
            <label className="flex items-center gap-1.5 text-ui-sm text-foreground select-none">
              <input
                type="checkbox"
                checked={relayEnabled}
                onChange={(e) => setRelayEnabled(e.target.checked)}
                className="accent-foreground"
              />
              {t('zk.useRelay')}
            </label>
            {relayEnabled ? (
              relayConfigs.length > 0 ? (
                <div className="w-48">
                  <SelectMenu
                    aria-label={t('zk.relayPicker')}
                    value={relayPickedId != null ? String(relayPickedId) : ''}
                    onChange={(next) => {
                      const config = relayConfigs.find((r) => String(r.id) === next);
                      setRelayPickedId(config ? config.id : null);
                    }}
                    options={[
                      { value: '', label: t('zk.relayPickPlaceholder') },
                      ...relayConfigs.map((r) => ({
                        value: String(r.id),
                        label: r.name,
                        description: `${r.host}:${r.port} · ${r.transport.toUpperCase()}`,
                      })),
                    ]}
                  />
                </div>
              ) : (
                <span className="text-ui-xs text-foreground-subtle">{t('zk.relayEmptyHint')}</span>
              )
            ) : null}
            <Button
              size="lg"
              className="h-7 px-3 text-ui-xs"
              disabled={connecting}
              onClick={() => void handleConnect()}
            >
              {connecting ? t('zk.connecting') : t('zk.connect')}
            </Button>
          </>
        )}
      </div>

      {(error || notice) && (
        <div
          className={cn(
            'flex shrink-0 items-center gap-2 border-b px-4 py-1.5 text-ui-xs',
            error
              ? 'border-destructive/20 bg-destructive/10 text-destructive'
              : 'border-border/50 bg-surface/40 text-foreground-subtle',
          )}
        >
          <span className="min-w-0 flex-1 truncate">{error ?? notice}</span>
          <button
            type="button"
            onClick={() => {
              setError(null);
              setNotice(null);
            }}
            className="opacity-70 hover:opacity-100"
          >
            ✕
          </button>
        </div>
      )}

      {/* 主体：树 + 详情 */}
      {sessionId ? (
        <div className="flex min-h-0 flex-1">
          {/* 左：znode 树 */}
          <div className="flex w-80 shrink-0 flex-col border-r border-border/50">
            <div className="flex items-center justify-between px-3 py-1.5">
              <span className="text-ui-sm text-foreground-subtle select-none">{t('zk.treeTitle')}</span>
              <ControlHintTooltip title={t('zk.createTitle')} side="bottom">
                <Button
                  variant="ghost"
                  size="lg"
                  className="h-7 px-2 text-ui-xs"
                  onClick={() => setCreating(true)}
                >
                  <FilePlus className="size-3.5" />
                </Button>
              </ControlHintTooltip>
            </div>
            <div className="min-h-0 flex-1 overflow-y-auto px-1.5 pb-2">
              {orderedPaths.map((node) => (
                <div
                  key={node.path}
                  className={cn(
                    'group flex h-7 items-center gap-1 rounded-lg pr-1 transition-colors',
                    selected === node.path ? 'bg-selected' : 'hover:bg-surface-hover',
                  )}
                  style={{ paddingLeft: `${node.depth * 14 + 4}px` }}
                >
                  <button
                    type="button"
                    className="flex h-full min-w-0 flex-1 items-center gap-1 text-left"
                    onClick={() => void selectNode(sessionId, node.path)}
                  >
                    <span className="w-3.5 shrink-0 text-foreground-subtle">
                      {node.children.length > 0 || !node.loaded ? (
                        <button
                          type="button"
                          className="flex size-3.5 items-center justify-center hover:text-foreground"
                          onClick={(e) => {
                            e.stopPropagation();
                            void toggleExpand(node);
                          }}
                        >
                          <ChevronRight
                            className={cn('size-3 transition-transform', node.expanded && 'rotate-90')}
                          />
                        </button>
                      ) : null}
                    </span>
                    <span
                      className={cn(
                        'truncate font-mono text-ui-sm',
                        node.ephemeral ? 'text-primary' : 'text-foreground',
                      )}
                      title={node.path}
                    >
                      {node.name}
                    </span>
                  </button>
                  {node.path !== '/' ? (
                    <button
                      type="button"
                      className="shrink-0 p-1 text-foreground-subtlest opacity-0 transition-opacity group-hover:opacity-100 hover:text-destructive"
                      onClick={() => setDeleting(node.path)}
                      title={t('zk.deleteTitle')}
                    >
                      <Trash2 className="size-3" />
                    </button>
                  ) : null}
                </div>
              ))}
            </div>
          </div>

          {/* 右：节点详情（数据 / 信息 / ACL） */}
          <div className="flex min-w-0 flex-1 flex-col overflow-y-auto">
            {detailLoading ? (
              <div className="p-4 text-ui-sm text-foreground-subtlest">{t('zk.loading')}</div>
            ) : detail ? (
              <div className="flex flex-col gap-4 p-4">
                <div className="flex items-center justify-between gap-2">
                  <span className="truncate font-mono text-ui-base text-foreground" title={selected}>
                    {selected}
                  </span>
                  <span className="shrink-0 text-ui-xs text-foreground-subtle select-none">
                    {t('zk.stat.version')}: {detail.stat.version} ·{' '}
                    {t('zk.stat.children')}: {detail.stat.numChildren} ·{' '}
                    {t('zk.stat.bytes')}: {detail.stat.dataLength}
                    {detail.stat.ephemeralOwner !== 0 ? ` · ${t('zk.stat.ephemeral')}` : ''}
                  </span>
                </div>

                {/* 数据 */}
                <div className="flex flex-col gap-1.5">
                  <div className="flex items-center justify-between">
                    <div className="flex items-center gap-2">
                      <span className="text-ui-sm text-foreground-subtle select-none">{t('zk.dataTitle')}</span>
                      {draftEncoding === 'base64' ? (
                        <span className="rounded-md border border-primary/50 bg-primary/15 px-1.5 py-0.5 text-ui-xs text-foreground select-none">
                          {t('zk.binaryBadge')}
                        </span>
                      ) : null}
                    </div>
                    <Button
                      size="lg"
                      className="h-7 px-2.5 text-ui-xs"
                      disabled={!dirty}
                      onClick={() => void saveData()}
                    >
                      {t('common.save')}
                    </Button>
                  </div>
                  <TextArea
                    value={draftData}
                    onChange={(e) => {
                      setDraftData(e.target.value);
                      setDirty(true);
                    }}
                    spellCheck={false}
                    className={cn('h-40 font-mono text-ui-sm', draftEncoding === 'base64' && 'break-all')}
                  />
                  {draftEncoding === 'base64' ? (
                    <p className="text-ui-xs text-foreground-subtlest select-none">{t('zk.binaryHint')}</p>
                  ) : null}
                </div>

                {/* 节点信息 */}
                <div className="flex flex-col gap-1">
                  <span className="text-ui-sm text-foreground-subtle select-none">{t('zk.statTitle')}</span>
                  <div className="grid grid-cols-2 gap-x-6 gap-y-0.5 rounded-lg border border-border p-2.5 text-ui-xs md:grid-cols-3">
                    {[
                      ['czxid', String(detail.stat.czxid)],
                      ['mzxid', String(detail.stat.mzxid)],
                      ['ctime', timeOf(detail.stat.ctime)],
                      ['mtime', timeOf(detail.stat.mtime)],
                      ['version', String(detail.stat.version)],
                      ['cversion', String(detail.stat.cversion)],
                      ['aversion', String(detail.stat.aversion)],
                      [
                        'ephemeralOwner',
                        detail.stat.ephemeralOwner === 0 ? '—' : String(detail.stat.ephemeralOwner),
                      ],
                      ['dataLength', String(detail.stat.dataLength)],
                      ['numChildren', String(detail.stat.numChildren)],
                    ].map(([k, v]) => (
                      <div key={k} className="flex items-center gap-2">
                        <span className="w-28 shrink-0 font-mono text-foreground-subtle">{k}</span>
                        <span className="min-w-0 truncate font-mono text-foreground">{v}</span>
                      </div>
                    ))}
                  </div>
                </div>

                {/* ACL */}
                <div className="flex flex-col gap-1.5">
                  <div className="flex items-center justify-between">
                    <span className="text-ui-sm text-foreground-subtle select-none">{t('zk.aclTitle')}</span>
                    <Button
                      size="lg"
                      className="h-7 px-2.5 text-ui-xs"
                      disabled={!aclDirty}
                      onClick={() => void saveAcls()}
                    >
                      {t('common.save')}
                    </Button>
                  </div>
                  <div className="flex flex-col gap-1">
                    {draftAcls.map((rule, index) => (
                      <div
                        key={`${rule.scheme}:${rule.id}:${index}`}
                        className="flex items-center gap-2 rounded-lg border border-border px-2.5 py-1.5"
                      >
                        <span className="w-44 shrink-0 truncate font-mono text-ui-sm text-foreground">
                          {rule.scheme}:{rule.id || '—'}
                        </span>
                        <div className="flex flex-1 flex-wrap gap-1">
                          {PERM_KEYS.map((key) => (
                            <button
                              key={key}
                              type="button"
                              onClick={() => toggleAclPerm(index, key)}
                              className={cn(
                                'rounded-md border px-1.5 py-0.5 text-ui-xs transition-colors',
                                rule.perms & ACL_BITS[key]
                                  ? 'border-primary/50 bg-primary/15 text-foreground'
                                  : 'border-border text-foreground-subtlest hover:text-foreground-subtle',
                              )}
                            >
                              {t(`zk.aclPerm.${key}`)}
                            </button>
                          ))}
                        </div>
                        <span className="w-8 shrink-0 text-center font-mono text-ui-xs text-foreground-subtle">
                          {rule.perms}
                        </span>
                        <button
                          type="button"
                          className="shrink-0 text-foreground-subtlest hover:text-destructive"
                          onClick={() => removeAclRule(index)}
                        >
                          <Trash2 className="size-3" />
                        </button>
                      </div>
                    ))}
                  </div>
                  {/* 新增规则 */}
                  <div className="flex flex-wrap items-center gap-2 rounded-lg border border-dashed border-border px-2.5 py-2">
                    <div className="w-32 shrink-0">
                      <SelectMenu
                        aria-label={t('zk.aclScheme')}
                        value={aclAddScheme}
                        onChange={setAclAddScheme}
                        options={[
                          { value: 'world', label: 'world' },
                          { value: 'auth', label: 'auth' },
                          { value: 'digest', label: 'digest' },
                          { value: 'ip', label: 'ip' },
                        ]}
                      />
                    </div>
                    {aclAddScheme === 'digest' ? (
                      <>
                        <div className="w-32">
                          <TextInput
                            value={aclAddDigestUser}
                            onChange={(e) => setAclAddDigestUser(e.target.value)}
                            placeholder={t('zk.aclUser')}
                            spellCheck={false}
                          />
                        </div>
                        <div className="w-36">
                          <PasswordInput
                            value={aclAddDigestPass}
                            onChange={(e) => setAclAddDigestPass(e.target.value)}
                            autoComplete="new-password"
                          />
                        </div>
                      </>
                    ) : aclAddScheme === 'ip' ? (
                      <div className="w-44">
                        <TextInput
                          value={aclAddId}
                          onChange={(e) => setAclAddId(e.target.value)}
                          placeholder="192.168.1.0/24"
                          spellCheck={false}
                          className="font-mono"
                        />
                      </div>
                    ) : null}
                    <Button
                      variant="outline"
                      size="lg"
                      className="h-7 px-2.5 text-ui-xs"
                      onClick={() => void addAclRule()}
                    >
                      {t('zk.aclAdd')}
                    </Button>
                    <span className="text-ui-xs text-foreground-subtlest select-none">
                      {t('zk.aclAddHint')}
                    </span>
                  </div>
                </div>
              </div>
            ) : (
              <div className="p-4 text-ui-sm text-foreground-subtlest">{t('zk.selectNode')}</div>
            )}
          </div>
        </div>
      ) : (
        <div className="flex flex-1 items-center justify-center text-ui-sm text-foreground-subtlest select-none">
          {t('zk.notConnected')}
        </div>
      )}

      {/* 新建节点 */}
      {creating ? (
        <Modal
          title={t('zk.createTitle')}
          onClose={() => setCreating(false)}
          className="w-[480px]"
          footer={
            <>
              <Button variant="ghost" size="lg" className="h-8 px-3 text-ui-base" onClick={() => setCreating(false)}>
                {t('common.cancel')}
              </Button>
              <Button size="lg" className="h-8 px-3 text-ui-base" onClick={() => void handleCreate()}>
                {t('common.save')}
              </Button>
            </>
          }
        >
          <div className="flex flex-col gap-3.5">
            <FormField label={t('zk.nodeName')}>
              <TextInput
                value={newName}
                onChange={(e) => setNewName(e.target.value)}
                placeholder="mynode"
                spellCheck={false}
                className="font-mono"
              />
            </FormField>
            <FormField label={t('zk.dataTitle')}>
              <TextArea
                value={newData}
                onChange={(e) => setNewData(e.target.value)}
                spellCheck={false}
                className="h-24 font-mono text-ui-sm"
              />
            </FormField>
            <div className="flex gap-4">
              <label className="flex items-center gap-1.5 text-ui-sm text-foreground select-none">
                <input
                  type="checkbox"
                  checked={newEphemeral}
                  onChange={(e) => setNewEphemeral(e.target.checked)}
                  className="accent-foreground"
                />
                {t('zk.ephemeral')}
              </label>
              <label className="flex items-center gap-1.5 text-ui-sm text-foreground select-none">
                <input
                  type="checkbox"
                  checked={newSequential}
                  onChange={(e) => setNewSequential(e.target.checked)}
                  className="accent-foreground"
                />
                {t('zk.sequential')}
              </label>
            </div>
          </div>
        </Modal>
      ) : null}

      {/* 删除确认 */}
      {deleting ? (
        <Modal
          title={t('zk.deleteTitle')}
          onClose={() => setDeleting(null)}
          className="w-[420px]"
          footer={
            <>
              <Button variant="ghost" size="lg" className="h-8 px-3 text-ui-base" onClick={() => setDeleting(null)}>
                {t('common.cancel')}
              </Button>
              <Button variant="destructive" size="lg" className="h-8 px-3 text-ui-base" onClick={() => void handleDelete()}>
                {t('common.delete')}
              </Button>
            </>
          }
        >
          <p className="text-ui-sm text-foreground">{t('zk.deleteConfirm', { path: deleting })}</p>
        </Modal>
      ) : null}

      {/* digest 认证 */}
      {authing ? (
        <Modal
          title={t('zk.authTitle')}
          onClose={() => setAuthing(false)}
          className="w-[420px]"
          footer={
            <>
              <Button variant="ghost" size="lg" className="h-8 px-3 text-ui-base" onClick={() => setAuthing(false)}>
                {t('common.cancel')}
              </Button>
              <Button size="lg" className="h-8 px-3 text-ui-base" onClick={() => void handleAuth()}>
                {t('zk.auth')}
              </Button>
            </>
          }
        >
          <div className="flex flex-col gap-3.5">
            <p className="text-ui-xs text-foreground-subtlest">{t('zk.authHint')}</p>
            <FormField label={t('zk.aclUser')}>
              <TextInput value={authUser} onChange={(e) => setAuthUser(e.target.value)} spellCheck={false} />
            </FormField>
            <FormField label={t('zk.authPassword')}>
              <PasswordInput
                value={authPass}
                onChange={(e) => setAuthPass(e.target.value)}
                autoComplete="new-password"
              />
            </FormField>
          </div>
        </Modal>
      ) : null}
    </div>
  );
}
