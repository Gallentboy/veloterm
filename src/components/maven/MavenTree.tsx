/**
 * Maven 依赖解析树（交互式折叠树）：
 * - 数据来自后端 resolver 的 arena 序列化（MavenTreeNode[]，按 parent/children 关联）
 * - 冲突落败节点（loser）灰显 + 删除线 + "omitted for conflict with X" 徽章
 * - 版本被根 dependencyManagement 钉住的节点带 "managed from X" 徽章
 * - 顶层默认展开两层；节点行点击切换折叠；提供全部展开/收拢动作
 * - 实时搜索过滤 / 匹配节点自动展开父链并高亮
 * - 链路溯源（Path-to-Root）：点击任意节点一眼看清完整引入路径及胜出者冲突路径对比
 */
import {
  ChevronRight,
  GitCommitHorizontal,
  Search,
  X,
  ArrowRight,
  CheckCircle2,
  AlertTriangle,
} from 'lucide-react';
import { memo, useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { ControlHintTooltip } from '@/components/ControlHintTooltip';
import type { MavenTreeNode } from '@/services/mavenService';
import { cn } from '@/lib/utils';

interface MavenTreeProps {
  nodes: MavenTreeNode[];
  className?: string;
}

/** 展开态集合（Set<nodeId>）；children 非空的节点才有折叠语义 */
function MavenTreeInner({ nodes, className }: MavenTreeProps) {
  const { t } = useTranslation();
  const byId = useMemo(() => {
    const m = new Map<number, MavenTreeNode>();
    for (const n of nodes) m.set(n.id, n);
    return m;
  }, [nodes]);
  const roots = useMemo(() => nodes.filter((n) => n.parent == null), [nodes]);
  const [expanded, setExpanded] = useState<Set<number>>(new Set());
  const [search, setSearch] = useState('');
  const [selectedId, setSelectedId] = useState<number | null>(null);

  // 数据变化时重置为「展开前两层」
  useEffect(() => {
    const next = new Set<number>();
    for (const r of roots) {
      if (r.children.length > 0) next.add(r.id);
      for (const c of r.children) {
        const cn2 = byId.get(c);
        if (cn2 && cn2.children.length > 0) next.add(cn2.id);
      }
    }
    setExpanded(next);
    setSelectedId(null);
    // eslint-disable-next-line react-hooks/exhaustive-deps -- roots/byId 随 nodes 变化
  }, [nodes]);

  const query = search.trim().toLowerCase();

  // 搜索匹配节点集合
  const matchedIds = useMemo(() => {
    if (!query) return new Set<number>();
    const set = new Set<number>();
    for (const n of nodes) {
      if (
        n.group.toLowerCase().includes(query) ||
        n.artifact.toLowerCase().includes(query) ||
        n.version.toLowerCase().includes(query) ||
        (n.classifier && n.classifier.toLowerCase().includes(query))
      ) {
        set.add(n.id);
      }
    }
    return set;
  }, [nodes, query]);

  // 搜索变动时自动展开所有匹配节点的父链
  useEffect(() => {
    if (matchedIds.size > 0) {
      setExpanded((prev) => {
        const next = new Set(prev);
        for (const mid of matchedIds) {
          let curr = byId.get(mid);
          while (curr && curr.parent != null) {
            next.add(curr.parent);
            curr = byId.get(curr.parent);
          }
        }
        return next;
      });
    }
  }, [matchedIds, byId]);

  const toggle = (id: number) => {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(id)) {
        next.delete(id);
      } else {
        next.add(id);
      }
      return next;
    });
  };

  const expandAll = () => setExpanded(new Set(nodes.filter((n) => n.children.length > 0).map((n) => n.id)));
  const collapseAll = () => setExpanded(new Set());

  const totalLosers = nodes.filter((n) => n.loser).length;

  const selectedNode = selectedId != null ? byId.get(selectedId) : null;

  // 选定节点的 Path-to-Root 链路（从 Root 到当前节点）
  const pathToRoot = useMemo(() => {
    if (!selectedNode) return [];
    const path: MavenTreeNode[] = [];
    let curr: MavenTreeNode | undefined = selectedNode;
    const visited = new Set<number>();
    while (curr) {
      if (visited.has(curr.id)) break;
      visited.add(curr.id);
      path.unshift(curr);
      if (curr.parent == null) break;
      curr = byId.get(curr.parent);
    }
    return path;
  }, [selectedNode, byId]);

  // 若选定节点为落败冲突节点（loser），计算胜出者（winner）的完整链路对比
  const winnerInfo = useMemo(() => {
    if (!selectedNode || !selectedNode.loser) return null;
    const winner = nodes.find(
      (n) =>
        !n.loser &&
        n.group === selectedNode.group &&
        n.artifact === selectedNode.artifact &&
        (!selectedNode.conflictWith || n.version === selectedNode.conflictWith),
    );
    if (!winner) return null;
    const path: MavenTreeNode[] = [];
    let curr: MavenTreeNode | undefined = winner;
    const visited = new Set<number>();
    while (curr) {
      if (visited.has(curr.id)) break;
      visited.add(curr.id);
      path.unshift(curr);
      if (curr.parent == null) break;
      curr = byId.get(curr.parent);
    }
    return { winner, path };
  }, [selectedNode, nodes, byId]);

  // 高亮匹配文字
  const renderHighlighted = (text: string): React.ReactNode => {
    if (!query) return text;
    const lower = text.toLowerCase();
    const idx = lower.indexOf(query);
    if (idx === -1) return text;
    const before = text.slice(0, idx);
    const match = text.slice(idx, idx + query.length);
    const after = text.slice(idx + query.length);
    return (
      <>
        {before}
        <mark className="rounded bg-primary/25 px-0.5 text-primary font-semibold">
          {match}
        </mark>
        {renderHighlighted(after)}
      </>
    );
  };

  const renderNode = (node: MavenTreeNode, isLast: boolean, prefix: string): React.ReactNode => {
    const open = expanded.has(node.id);
    const hasChildren = node.children.length > 0;
    const isSelected = selectedId === node.id;
    const isMatched = query ? matchedIds.has(node.id) : false;
    const coord =
      node.classifier && node.classifier.length > 0
        ? `${node.group}:${node.artifact}:${node.version}:${node.packaging}:${node.classifier}`
        : `${node.group}:${node.artifact}:${node.version}`;

    return (
      <div key={node.id}>
        <div
          onClick={() => setSelectedId(node.id)}
          className={cn(
            'group flex cursor-pointer items-center gap-1 rounded px-1 py-0.5 transition-colors',
            isSelected
              ? 'bg-primary/15 ring-1 ring-primary/40'
              : isMatched
                ? 'bg-primary/10'
                : 'hover:bg-surface/60',
            node.loser && !isSelected && 'opacity-65',
          )}
          style={{ paddingLeft: `${Math.min(prefix.length, 40) * 5 + 4}px` }}
        >
          {hasChildren ? (
            <button
              type="button"
              onClick={(e) => {
                e.stopPropagation();
                toggle(node.id);
              }}
              className="flex size-4 shrink-0 items-center justify-center rounded text-foreground-subtlest hover:bg-hover hover:text-foreground"
              aria-label={open ? t('maven.collapse') : t('maven.expand')}
            >
              <ChevronRight className={cn('size-3 transition-transform', open && 'rotate-90')} />
            </button>
          ) : (
            <span className="size-4 shrink-0" />
          )}
          <div className="min-w-0 flex-1">
            <ControlHintTooltip title={`${coord} (${t('maven.clickToTrace')})`} side="top" align="start" delayMs={200} block>
              <span
                className={cn(
                  'block truncate font-mono text-ui-xs',
                  node.loser ? 'line-through text-foreground-subtlest' : 'text-foreground',
                  isSelected && 'font-medium text-primary',
                )}
              >
                {renderHighlighted(coord)}
              </span>
            </ControlHintTooltip>
          </div>
          <span className="shrink-0 rounded bg-surface/80 px-1 font-mono text-[10px] text-foreground-subtle border border-border/40">
            {node.scope}
          </span>
          {node.managedFrom ? (
            <ControlHintTooltip title={`${t('maven.managedFrom')}: ${node.managedFrom}`} side="top">
              <span className="shrink-0 rounded bg-primary/10 px-1 font-mono text-[10px] text-primary border border-primary/20">
                ⚲ {node.managedFrom}
              </span>
            </ControlHintTooltip>
          ) : null}
          {node.loser ? (
            <span className="shrink-0 rounded bg-warning/10 px-1 font-mono text-[10px] text-warning border border-warning/20">
              ✗ {node.conflictWith ? `conflict ${node.conflictWith}` : 'duplicate'}
            </span>
          ) : null}
        </div>
        {hasChildren && open ? (
          <div>
            {node.children.map((cid, idx) => {
              const child = byId.get(cid);
              if (!child) return null;
              return renderNode(child, idx + 1 === node.children.length, `${prefix}${isLast ? '  ' : '│ '}`);
            })}
          </div>
        ) : null}
      </div>
    );
  };

  if (nodes.length === 0) return null;

  return (
    <div className={cn('flex min-h-0 flex-col', className)}>
      {/* 顶部工具栏：标题、搜索框、操作按钮 */}
      <div className="flex shrink-0 flex-wrap items-center gap-2 border-b border-border/40 px-2 py-1.5 text-ui-xs text-foreground-subtle select-none">
        <span className="font-medium text-foreground">{t('maven.treeTitle')}</span>
        <span>
          {nodes.length} {t('maven.nodes')} · {totalLosers} {t('maven.conflicts')}
        </span>

        {/* 实时搜索过滤框 */}
        <div className="relative flex min-w-44 flex-1 items-center max-w-sm">
          <Search className="absolute left-2 size-3 text-foreground-subtlest" />
          <input
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder={t('maven.searchDependencies')}
            className="h-6 w-full rounded border border-border/60 bg-input pl-6 pr-6 font-mono text-[11px] text-foreground outline-none placeholder:text-foreground-subtlest focus:border-border-hover"
          />
          {search ? (
            <button
              type="button"
              onClick={() => setSearch('')}
              className="absolute right-1.5 flex size-3.5 items-center justify-center rounded text-foreground-subtlest hover:text-foreground"
            >
              <X className="size-2.5" />
            </button>
          ) : null}
        </div>

        {query ? (
          <span className="font-mono text-[11px] text-primary">
            {t('maven.matchesFound', { count: matchedIds.size })}
          </span>
        ) : null}

        <div className="flex items-center gap-1">
          <button
            type="button"
            onClick={expandAll}
            className="rounded px-1.5 py-0.5 hover:bg-hover hover:text-foreground"
          >
            {t('maven.expandAll')}
          </button>
          <button
            type="button"
            onClick={collapseAll}
            className="rounded px-1.5 py-0.5 hover:bg-hover hover:text-foreground"
          >
            {t('maven.collapseAll')}
          </button>
        </div>
      </div>

      {/* 树主体 */}
      <div className="scrollbar-slim min-h-0 flex-1 overflow-auto py-1">
        {roots.map((r, i) => renderNode(r, i + 1 === roots.length, ''))}
      </div>

      {/* 链路溯源面板（Path to Root） */}
      {selectedNode ? (
        <div className="shrink-0 border-t border-border/50 bg-surface/90 p-2 text-ui-xs shadow-sm">
          <div className="mb-1.5 flex items-center justify-between gap-2">
            <div className="flex items-center gap-1.5 font-medium text-foreground">
              <GitCommitHorizontal className="size-3.5 text-primary" />
              <span>{t('maven.pathToRoot')}</span>
              <span className="font-mono text-[11px] text-foreground-subtle">
                ({selectedNode.group}:{selectedNode.artifact}:{selectedNode.version})
              </span>
            </div>
            <button
              type="button"
              onClick={() => setSelectedId(null)}
              className="flex size-4 items-center justify-center rounded text-foreground-subtlest hover:bg-hover hover:text-foreground"
            >
              <X className="size-3" />
            </button>
          </div>

          {/* 当前节点引入链路 */}
          <div className="space-y-1.5">
            <div className="flex flex-wrap items-center gap-1 rounded bg-background/60 p-1.5 border border-border/40 font-mono text-[11px]">
              {selectedNode.loser ? (
                <span className="flex items-center gap-1 font-sans text-warning font-medium mr-1 shrink-0">
                  <AlertTriangle className="size-3" />
                  {t('maven.conflictPath')}:
                </span>
              ) : null}
              {pathToRoot.map((item, idx) => {
                const isTarget = idx === pathToRoot.length - 1;
                return (
                  <div key={item.id} className="flex items-center gap-1">
                    {idx > 0 ? <ArrowRight className="size-2.5 text-foreground-subtlest" /> : null}
                    <button
                      type="button"
                      onClick={() => setSelectedId(item.id)}
                      className={cn(
                        'rounded px-1 py-0.5 transition-colors hover:bg-hover',
                        isTarget
                          ? item.loser
                            ? 'bg-warning/15 font-semibold text-warning line-through'
                            : 'bg-primary/20 font-semibold text-primary'
                          : 'text-foreground-subtle hover:text-foreground',
                      )}
                    >
                      {item.artifact}:{item.version}
                    </button>
                  </div>
                );
              })}
            </div>

            {/* 若是冲突节点，并列展示胜出者链路与决策原因 */}
            {winnerInfo ? (
              <div className="rounded border border-primary/20 bg-primary/5 p-1.5 text-[11px]">
                <div className="mb-1 flex items-center justify-between text-primary">
                  <span className="flex items-center gap-1 font-medium">
                    <CheckCircle2 className="size-3" />
                    {t('maven.winnerPath')} ({winnerInfo.winner.artifact}:{winnerInfo.winner.version})
                  </span>
                  <span className="text-[10px] text-foreground-subtlest">
                    {t('maven.winnerReason')}
                  </span>
                </div>
                <div className="flex flex-wrap items-center gap-1 font-mono">
                  {winnerInfo.path.map((item, idx) => {
                    const isWinner = idx === winnerInfo.path.length - 1;
                    return (
                      <div key={item.id} className="flex items-center gap-1">
                        {idx > 0 ? <ArrowRight className="size-2.5 text-foreground-subtlest" /> : null}
                        <button
                          type="button"
                          onClick={() => setSelectedId(item.id)}
                          className={cn(
                            'rounded px-1 py-0.5 transition-colors hover:bg-hover',
                            isWinner
                              ? 'bg-primary/20 font-semibold text-primary'
                              : 'text-foreground-subtle hover:text-foreground',
                          )}
                        >
                          {item.artifact}:{item.version}
                        </button>
                      </div>
                    );
                  })}
                </div>
              </div>
            ) : null}
          </div>
        </div>
      ) : null}
    </div>
  );
}

export const MavenTree = memo(MavenTreeInner);
