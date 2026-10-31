/**
 * Maven 依赖解析树（交互式折叠树）：
 * - 数据来自后端 resolver 的 arena 序列化（MavenTreeNode[]，按 parent/children 关联）
 * - 冲突落败节点（loser）灰显 + 删除线 + "omitted for conflict with X" 徽章
 * - 版本被根 dependencyManagement 钉住的节点带 "managed from X" 徽章
 * - 顶层默认展开两层；节点行点击切换折叠；提供全部展开/收拢动作
 */
import { ChevronRight } from 'lucide-react';
import { memo, useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
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
    // eslint-disable-next-line react-hooks/exhaustive-deps -- roots/byId 随 nodes 变化
  }, [nodes]);

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

  const renderNode = (node: MavenTreeNode, isLast: boolean, prefix: string): React.ReactNode => {
    const open = expanded.has(node.id);
    const hasChildren = node.children.length > 0;
    const coord =
      node.classifier && node.classifier.length > 0
        ? `${node.group}:${node.artifact}:${node.version}:${node.packaging}:${node.classifier}`
        : `${node.group}:${node.artifact}:${node.version}`;
    return (
      <div key={node.id}>
        <div
          className={cn(
            'group flex items-center gap-1 rounded px-1 py-px hover:bg-surface/60',
            node.loser && 'opacity-55',
          )}
          style={{ paddingLeft: `${Math.min(prefix.length, 40) * 5 + 4}px` }}
        >
          {hasChildren ? (
            <button
              type="button"
              onClick={() => toggle(node.id)}
              className="flex size-4 shrink-0 items-center justify-center rounded text-foreground-subtlest hover:bg-hover hover:text-foreground"
              aria-label={open ? t('maven.collapse') : t('maven.expand')}
            >
              <ChevronRight className={cn('size-3 transition-transform', open && 'rotate-90')} />
            </button>
          ) : (
            <span className="size-4 shrink-0" />
          )}
          <span
            className={cn(
              'truncate font-mono text-ui-xs',
              node.loser ? 'line-through text-foreground-subtlest' : 'text-foreground',
            )}
            title={coord}
          >
            {coord}
          </span>
          <span className="shrink-0 rounded bg-surface/80 px-1 font-mono text-[10px] text-foreground-subtle border border-border/40">
            {node.scope}
          </span>
          {node.managedFrom ? (
            <span
              className="shrink-0 rounded bg-primary/10 px-1 font-mono text-[10px] text-primary border border-primary/20"
              title={`${t('maven.managedFrom')}: ${node.managedFrom}`}
            >
              ⚲ {node.managedFrom}
            </span>
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
      <div className="flex shrink-0 items-center gap-2 border-b border-border/40 px-2 py-1 text-ui-xs text-foreground-subtle select-none">
        <span className="font-medium text-foreground">{t('maven.treeTitle')}</span>
        <span>
          {nodes.length} {t('maven.nodes')} · {totalLosers} {t('maven.conflicts')}
        </span>
        <span className="flex-1" />
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
      <div className="scrollbar-slim min-h-0 flex-1 overflow-auto py-1">
        {roots.map((r, i) => renderNode(r, i + 1 === roots.length, ''))}
      </div>
    </div>
  );
}

export const MavenTree = memo(MavenTreeInner);
