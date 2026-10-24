/**
 * 执行计划视图（两种形态，工具栏切换）：
 * - 树形（默认）：经典缩进连接符树（├─/└─ 等宽字体），大型计划可读性最好，
 *   行内带 Cost/Rows/Bytes 徽标；
 * - 拓扑图：React Flow 节点/边图，拖拽平移、滚轮缩放、小地图——适合
 *   分支复杂的中等规模计划。
 * 节点表来自后端统一解析（id/parentId 即边表）；标色规则两形态一致：
 * 按本计划内最大 cost/rows 的占比（红 ≥60% 高开销、黄 ≥25% 偏高）。
 */

import { useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import {
  Background,
  BackgroundVariant,
  Controls,
  Handle,
  MiniMap,
  Position,
  ReactFlow,
  type Edge,
  type Node,
  type NodeProps,
} from '@xyflow/react';
import '@xyflow/react/dist/style.css';
import { ListTree, Network } from 'lucide-react';
import type { DbExplainPlan, DbPlanNode } from '@/services/dbConnectionsService';
import { cn } from '@/lib/utils';

type PlanLevel = 'hot' | 'warm' | 'normal';

interface PlanNodeData {
  title: string;
  detail: string;
  cost: number | null;
  rows: number | null;
  bytes: number | null;
  level: PlanLevel;
}

/// ── 共享：节点表 → 树结构与标色 ───────────────────────────────────────

interface PlanTree {
  roots: DbPlanNode[];
  children: Map<number, DbPlanNode[]>;
  levels: Map<number, PlanLevel>;
}

function buildTree(nodes: DbPlanNode[]): PlanTree {
  const byId = new Map<number, DbPlanNode>();
  for (const n of nodes) byId.set(n.id, n);
  const children = new Map<number, DbPlanNode[]>();
  const roots: DbPlanNode[] = [];
  for (const n of nodes) {
    const parent = n.parentId != null ? byId.get(n.parentId) : undefined;
    if (parent && parent.id !== n.id) {
      children.set(parent.id, [...(children.get(parent.id) ?? []), n]);
    } else {
      roots.push(n);
    }
  }
  // 环防护：从根不可达的节点平铺为根（解析侧异常输入不致丢节点）
  const reached = new Set<number>();
  const mark = (id: number) => {
    if (reached.has(id)) return;
    reached.add(id);
    for (const k of children.get(id) ?? []) mark(k.id);
  };
  roots.forEach((r) => mark(r.id));
  for (const n of nodes) {
    if (!reached.has(n.id)) {
      roots.push(n);
      reached.add(n.id);
    }
  }
  const maxCost = Math.max(0, ...nodes.map((n) => n.cost ?? 0));
  const maxRows = Math.max(0, ...nodes.map((n) => n.rows ?? 0));
  const levels = new Map<number, PlanLevel>();
  for (const n of nodes) {
    const ratio = Math.max(
      n.cost != null && maxCost > 0 ? n.cost / maxCost : 0,
      n.rows != null && maxRows > 0 ? n.rows / maxRows : 0,
    );
    levels.set(n.id, ratio >= 0.6 ? 'hot' : ratio >= 0.25 ? 'warm' : 'normal');
  }
  return { roots, children, levels };
}

function fmtNum(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 10_000) return `${Math.round(n / 1_000)}K`;
  if (n >= 100 && Number.isInteger(n)) return n.toLocaleString('en-US');
  return `${parseFloat(n.toFixed(1))}`;
}

function fmtBytes(n: number): string {
  if (n >= 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(1)}MB`;
  if (n >= 1024) return `${(n / 1024).toFixed(1)}KB`;
  return `${n}B`;
}

/// ── 形态一：经典缩进树（默认） ────────────────────────────────────────

function TreeLine({
  node,
  prefix,
  isLast,
  tree,
}: {
  node: DbPlanNode;
  prefix: string;
  isLast: boolean;
  tree: PlanTree;
}) {
  const level = tree.levels.get(node.id) ?? 'normal';
  const kids = tree.children.get(node.id) ?? [];
  const connector = prefix === '' ? '' : isLast ? '└─ ' : '├─ ';
  return (
    <div className="whitespace-pre">
      <span className="text-foreground-subtlest select-none">{prefix + connector}</span>
      <span
        className={cn(
          'font-semibold',
          level === 'hot' && 'text-destructive',
          level === 'warm' && 'text-terminal-yellow',
          level === 'normal' && 'text-foreground',
        )}
      >
        {node.title}
      </span>
      {node.detail ? <span className="ml-1.5 text-foreground-subtle">{node.detail}</span> : null}
      {node.cost != null || node.rows != null || node.bytes != null ? (
        <span className="ml-1.5 text-foreground-subtlest">
          {node.cost != null ? `Cost ${fmtNum(node.cost)}` : ''}
          {node.cost != null && node.rows != null ? ' · ' : ''}
          {node.rows != null ? `Rows ${fmtNum(node.rows)}` : ''}
          {(node.cost != null || node.rows != null) && node.bytes != null ? ' · ' : ''}
          {node.bytes != null ? fmtBytes(node.bytes) : ''}
        </span>
      ) : null}
      {kids.map((k, i) => (
        <TreeLine
          key={k.id}
          node={k}
          prefix={prefix + (prefix === '' ? '' : isLast ? '   ' : '│  ')}
          isLast={i === kids.length - 1}
          tree={tree}
        />
      ))}
    </div>
  );
}

function TreePlanView({ nodes }: { nodes: DbPlanNode[] }) {
  const tree = useMemo(() => buildTree(nodes), [nodes]);
  if (tree.roots.length === 0) return null;
  return (
    <div className="scrollbar-slim h-full overflow-auto p-3 select-text">
      <div className="inline-block min-w-full font-mono text-ui-xs leading-relaxed">
        {tree.roots.map((r, i) => (
          <TreeLine
            key={r.id}
            node={r}
            prefix=""
            isLast={i === tree.roots.length - 1}
            tree={tree}
          />
        ))}
      </div>
    </div>
  );
}

/// ── 形态二：React Flow 拓扑图 ─────────────────────────────────────────

function PlanNodeCard(props: NodeProps) {
  const d = props.data as unknown as PlanNodeData;
  return (
    <div
      className={cn(
        'w-56 rounded-lg border px-2.5 py-1.5 shadow-sm select-none',
        d.level === 'hot' && 'border-destructive/70 bg-destructive/10',
        d.level === 'warm' && 'border-terminal-yellow/60 bg-terminal-yellow/10',
        d.level === 'normal' && 'border-border bg-surface',
      )}
    >
      <Handle type="target" position={Position.Top} className="!bg-foreground-subtlest !border-0" />
      <div className="truncate font-mono text-ui-xs font-semibold text-foreground" title={d.title}>
        {d.title}
      </div>
      {d.detail ? (
        <div
          className="mt-0.5 line-clamp-2 font-mono text-[10px] leading-snug break-all text-foreground-subtle"
          title={d.detail}
        >
          {d.detail}
        </div>
      ) : null}
      {d.cost != null || d.rows != null || d.bytes != null ? (
        <div className="mt-1 flex flex-wrap gap-x-2.5 font-mono text-[10px] text-foreground-subtlest">
          {d.cost != null ? <span>Cost {fmtNum(d.cost)}</span> : null}
          {d.rows != null ? <span>Rows {fmtNum(d.rows)}</span> : null}
          {d.bytes != null ? <span>{fmtBytes(d.bytes)}</span> : null}
        </div>
      ) : null}
      <Handle
        type="source"
        position={Position.Bottom}
        className="!bg-foreground-subtlest !border-0"
      />
    </div>
  );
}

const nodeTypes = { plan: PlanNodeCard };

const NODE_X_GAP = 250;
const NODE_Y_GAP = 140;

/** 节点表 → 树形布局的 React Flow 节点/边（叶子按序占列，父节点居中） */
function buildGraph(nodes: DbPlanNode[]): { rfNodes: Node[]; edges: Edge[] } {
  const { roots, children, levels } = buildTree(nodes);
  const positions = new Map<number, { x: number; y: number }>();
  let leafX = 0;
  const place = (node: DbPlanNode, depth: number): number => {
    const kids = children.get(node.id) ?? [];
    let x: number;
    if (kids.length === 0) {
      x = leafX++;
    } else {
      const xs = kids.map((k) => place(k, depth + 1));
      x = (Math.min(...xs) + Math.max(...xs)) / 2;
    }
    positions.set(node.id, { x: x * NODE_X_GAP, y: depth * NODE_Y_GAP });
    return x;
  };
  roots.forEach((r) => place(r, 0));

  const rfNodes: Node[] = nodes.map((n) => ({
    id: String(n.id),
    type: 'plan' as const,
    position: positions.get(n.id) ?? { x: 0, y: 0 },
    data: {
      title: n.title,
      detail: n.detail,
      cost: n.cost,
      rows: n.rows,
      bytes: n.bytes,
      level: levels.get(n.id) ?? 'normal',
    } satisfies PlanNodeData,
  }));
  const edges: Edge[] = [];
  for (const n of nodes) {
    if (n.parentId == null) continue;
    const parent = nodes.find((p) => p.id === n.parentId);
    if (!parent || parent.id === n.id) continue;
    edges.push({
      id: `${parent.id}-${n.id}`,
      source: String(parent.id),
      target: String(n.id),
      type: 'smoothstep',
    });
  }
  return { rfNodes, edges };
}

function GraphPlanView({ nodes, viewKey }: { nodes: DbPlanNode[]; viewKey: number }) {
  const { rfNodes, edges } = useMemo(() => buildGraph(nodes), [nodes]);
  if (rfNodes.length === 0) return null;
  return (
    <ReactFlow
      key={viewKey}
      nodes={rfNodes}
      edges={edges}
      nodeTypes={nodeTypes}
      fitView
      minZoom={0.05}
      maxZoom={2}
      proOptions={{ hideAttribution: true }}
      nodesDraggable={false}
      nodesConnectable={false}
      elementsSelectable={false}
    >
      <Background variant={BackgroundVariant.Dots} gap={20} size={1} />
      <Controls showInteractive={false} className="!border !border-border !bg-surface" />
      <MiniMap pannable zoomable className="!border !border-border !bg-surface" />
    </ReactFlow>
  );
}

/// ── 顶层：工具栏 + 形态切换 ───────────────────────────────────────────

type PlanViewMode = 'tree' | 'graph';

export function ExplainPlanGraph({
  plans,
  onClose,
}: {
  plans: DbExplainPlan[];
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const [active, setActive] = useState(0);
  const [view, setView] = useState<PlanViewMode>('tree');
  const plan = plans[Math.min(active, plans.length - 1)] ?? plans[0];
  const nodes = plan?.nodes ?? [];
  const hasNodes = nodes.length > 0;

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex h-8 shrink-0 items-center gap-1 border-b border-border/50 bg-surface/30 px-2 select-none">
        {plans.length > 1 ? (
          plans.map((p, i) => (
            <button
              key={i}
              type="button"
              onClick={() => setActive(i)}
              className={cn(
                'flex max-w-52 shrink-0 items-center gap-1 truncate rounded-md border px-2 py-0.5 font-mono text-ui-xs transition-colors',
                i === active
                  ? 'border-border bg-surface font-semibold text-foreground'
                  : 'border-transparent text-foreground-subtle hover:bg-hover hover:text-foreground',
              )}
              title={p.statement}
            >
              #{i + 1}
            </button>
          ))
        ) : (
          <span
            className="max-w-96 truncate font-mono text-ui-xs text-foreground-subtle"
            title={plan?.statement}
          >
            {plan?.statement}
          </span>
        )}
        <div className="flex-1" />
        <div className="flex shrink-0 items-center rounded-md border border-border/70 p-0.5">
          {(
            [
              { mode: 'tree' as const, icon: ListTree, label: t('db.explainViewTree') },
              { mode: 'graph' as const, icon: Network, label: t('db.explainViewGraph') },
            ]
          ).map(({ mode, icon: Icon, label }) => (
            <button
              key={mode}
              type="button"
              onClick={() => setView(mode)}
              className={cn(
                'flex items-center gap-1 rounded px-1.5 py-0.5 text-ui-xs transition-colors',
                view === mode
                  ? 'bg-surface font-semibold text-foreground'
                  : 'text-foreground-subtle hover:text-foreground',
              )}
            >
              <Icon className="size-3" />
              {label}
            </button>
          ))}
        </div>
        <span className="shrink-0 text-[10px] text-foreground-subtlest">
          <span className="mr-1 inline-block size-2 rounded-sm border border-destructive/70 bg-destructive/20 align-middle" />
          {t('db.explainLegendHot')}
          <span className="mx-1 inline-block size-2 rounded-sm border border-terminal-yellow/60 bg-terminal-yellow/20 align-middle" />
          {t('db.explainLegendWarm')}
        </span>
        <button
          type="button"
          onClick={onClose}
          className="rounded p-1 text-foreground-subtlest hover:text-foreground"
          aria-label={t('common.close', '关闭')}
        >
          ✕
        </button>
      </div>
      <div className="min-h-0 flex-1">
        {!hasNodes ? (
          <div className="flex h-full items-center justify-center text-ui-sm text-foreground-subtlest select-none">
            {t('db.explainNoData')}
          </div>
        ) : view === 'tree' ? (
          <TreePlanView nodes={nodes} />
        ) : (
          <GraphPlanView nodes={nodes} viewKey={active} />
        )}
      </div>
    </div>
  );
}
