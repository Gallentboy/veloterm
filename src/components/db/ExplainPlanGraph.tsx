/**
 * 执行计划节点图（React Flow / @xyflow/react）：后端解析好的 PlanNode 节点表
 * （id/parentId 即边表）→ 树形布局的节点/边图。拖拽平移、滚轮缩放、小地图、
 * 缩放控制条；高开销/大数据量节点按本计划内最大值的占比标色（红 ≥60%、
 * 黄 ≥25%）。多语句时顶部 Tab 逐条切换。节点位置固定（不做拖拽挪动），
 * 画布本身可自由平移缩放。
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
  const positions = new Map<number, { x: number; y: number }>();
  let leafX = 0;
  const place = (node: DbPlanNode, depth: number): number => {
    reached.add(node.id);
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
  const orphans = nodes.filter((n) => !reached.has(n.id));
  orphans.forEach((n) => {
    positions.set(n.id, { x: leafX++ * NODE_X_GAP, y: 0 });
    roots.push(n);
  });

  const maxCost = Math.max(0, ...nodes.map((n) => n.cost ?? 0));
  const maxRows = Math.max(0, ...nodes.map((n) => n.rows ?? 0));
  const levelOf = (n: DbPlanNode): PlanLevel => {
    const ratio = Math.max(
      n.cost != null && maxCost > 0 ? n.cost / maxCost : 0,
      n.rows != null && maxRows > 0 ? n.rows / maxRows : 0,
    );
    if (ratio >= 0.6) return 'hot';
    if (ratio >= 0.25) return 'warm';
    return 'normal';
  };

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
      level: levelOf(n),
    } satisfies PlanNodeData,
  }));
  const edges: Edge[] = [];
  for (const n of nodes) {
    if (n.parentId == null) continue;
    const parent = byId.get(n.parentId);
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

export function ExplainPlanGraph({
  plans,
  onClose,
}: {
  plans: DbExplainPlan[];
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const [active, setActive] = useState(0);
  const plan = plans[Math.min(active, plans.length - 1)] ?? plans[0];
  const { rfNodes, edges } = useMemo(() => buildGraph(plan?.nodes ?? []), [plan]);

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
          <span className="max-w-96 truncate font-mono text-ui-xs text-foreground-subtle" title={plan?.statement}>
            {plan?.statement}
          </span>
        )}
        <div className="flex-1" />
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
        {rfNodes.length === 0 ? (
          <div className="flex h-full items-center justify-center text-ui-sm text-foreground-subtlest select-none">
            {t('db.explainNoData')}
          </div>
        ) : (
          <ReactFlow
            key={active}
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
        )}
      </div>
    </div>
  );
}
