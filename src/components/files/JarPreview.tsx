import {
  Archive,
  ChevronDown,
  ChevronRight,
  Copy,
  Check,
  FileCode,
  FileText,
  Folder,
  FolderOpen,
  Info,
  Search,
  X,
} from 'lucide-react';
import React, { memo, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import * as monaco from 'monaco-editor';
import { Button } from '@/components/ui/button';
import { Spinner } from '@/components/ui/spinner';
import { cn } from '@/lib/utils';
import {
  decompilerGetStatus,
  decompilerOpenJar,
  decompilerReadEntry,
  type DecompilerStatus,
  type JarEntryNode,
  type JarStructureOverview,
} from '@/services/decompilerService';
import { setupMonacoEnvironment } from '@/editor/monacoEnvironment';
import { initMonacoTheme, VELOTERM_MONACO_THEME } from '@/editor/monacoTheme';

export interface JarPreviewProps {
  filePath: string;
  fileName: string;
  className?: string;
}

const MONACO_OPTIONS: monaco.editor.IStandaloneEditorConstructionOptions = {
  theme: VELOTERM_MONACO_THEME,
  automaticLayout: true,
  fontFamily: 'var(--font-mono)',
  fontLigatures: true,
  fontSize: 13,
  minimap: { enabled: false },
  scrollBeyondLastLine: false,
  wordWrap: 'on',
  renderWhitespace: 'selection',
  smoothScrolling: true,
  readOnly: true,
  fixedOverflowWidgets: true,
  padding: { top: 10, bottom: 10 },
  scrollbar: {
    vertical: 'auto',
    horizontal: 'auto',
    verticalScrollbarSize: 8,
    horizontalScrollbarSize: 8,
    useShadows: false,
  },
};

/** 递归检查树节点是否匹配搜索关键字 */
function filterTreeNode(node: JarEntryNode, query: string): JarEntryNode | null {
  const lowerQuery = query.toLowerCase();
  const nameMatch = node.name.toLowerCase().includes(lowerQuery) || node.path.toLowerCase().includes(lowerQuery);

  if (!node.is_dir) {
    return nameMatch ? node : null;
  }

  const filteredChildren: JarEntryNode[] = [];
  if (node.children) {
    for (const child of node.children) {
      const matched = filterTreeNode(child, query);
      if (matched) {
        filteredChildren.push(matched);
      }
    }
  }

  if (nameMatch || filteredChildren.length > 0) {
    return {
      ...node,
      children: filteredChildren,
    };
  }

  return null;
}

/** 格式化字节大小 */
function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

interface TreeItemProps {
  node: JarEntryNode;
  selectedPath: string;
  onSelect: (node: JarEntryNode) => void;
  expandedPaths: Set<string>;
  onToggleExpand: (path: string) => void;
  depth: number;
}

const TreeItem = memo(function TreeItem({
  node,
  selectedPath,
  onSelect,
  expandedPaths,
  onToggleExpand,
  depth,
}: TreeItemProps) {
  const isExpanded = expandedPaths.has(node.path);
  const isSelected = selectedPath === node.path;

  const handleClick = (e: React.MouseEvent) => {
    e.stopPropagation();
    if (node.is_dir) {
      onToggleExpand(node.path);
    } else {
      onSelect(node);
    }
  };

  return (
    <div>
      <div
        onClick={handleClick}
        style={{ paddingLeft: `${Math.max(6, depth * 14)}px` }}
        className={cn(
          'group flex h-6.5 cursor-pointer items-center gap-1.5 pr-2 rounded-sm text-ui-xs font-mono select-none transition-colors',
          isSelected
            ? 'bg-selected text-foreground font-medium'
            : 'text-foreground-subtle hover:bg-hover hover:text-foreground',
          node.is_inner_class && 'opacity-70 text-ui-2xs',
        )}
      >
        {node.is_dir ? (
          <span
            className="flex size-4 shrink-0 items-center justify-center text-foreground-subtlest group-hover:text-foreground"
            onClick={(e) => {
              e.stopPropagation();
              onToggleExpand(node.path);
            }}
          >
            {isExpanded ? <ChevronDown className="size-3" /> : <ChevronRight className="size-3" />}
          </span>
        ) : (
          <span className="size-4 shrink-0" />
        )}

        {node.is_dir ? (
          isExpanded ? (
            <FolderOpen className="size-3.5 shrink-0 text-terminal-yellow" />
          ) : (
            <Folder className="size-3.5 shrink-0 text-terminal-yellow" />
          )
        ) : node.is_class ? (
          <FileCode
            className={cn(
              'size-3.5 shrink-0',
              node.is_inner_class ? 'text-terminal-blue/80' : 'text-terminal-blue',
            )}
          />
        ) : (
          <FileText className="size-3.5 shrink-0 text-foreground-subtlest" />
        )}

        <span className="truncate flex-1" title={node.path}>
          {node.name}
        </span>

        {!node.is_dir && node.size > 0 && (
          <span className="text-[10px] text-foreground-subtlest opacity-0 group-hover:opacity-100 transition-opacity">
            {formatSize(node.size)}
          </span>
        )}
      </div>

      {node.is_dir && isExpanded && node.children && (
        <div>
          {node.children.map((child) => (
            <TreeItem
              key={child.path}
              node={child}
              selectedPath={selectedPath}
              onSelect={onSelect}
              expandedPaths={expandedPaths}
              onToggleExpand={onToggleExpand}
              depth={depth + 1}
            />
          ))}
        </div>
      )}
    </div>
  );
});

export const JarPreview = memo(function JarPreview({
  filePath,
  className,
}: JarPreviewProps) {
  const { t } = useTranslation();

  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [overview, setOverview] = useState<JarStructureOverview | null>(null);
  const [decompilerStatus, setDecompilerStatus] = useState<DecompilerStatus | null>(null);

  // 选中的文件条目与内容
  const [selectedNode, setSelectedNode] = useState<JarEntryNode | null>(null);
  const [contentLoading, setContentLoading] = useState(false);
  const [copied, setCopied] = useState(false);
  const [copiedCoords, setCopiedCoords] = useState(false);

  // 搜索关键字与树展开状态
  const [searchQuery, setSearchQuery] = useState('');
  const [expandedPaths, setExpandedPaths] = useState<Set<string>>(new Set());

  // 内存缓存已读取/反编译的内容，避免重复请求
  const contentCacheRef = useRef<Map<string, string>>(new Map());

  // Monaco 编辑器容器
  const editorContainerRef = useRef<HTMLDivElement>(null);
  const editorRef = useRef<monaco.editor.IStandaloneCodeEditor | null>(null);

  // 初始化 Monaco 环境与主题
  useEffect(() => {
    setupMonacoEnvironment();
    initMonacoTheme();
  }, []);

  // 1. 加载 Jar 概览结构与守护进程状态
  useEffect(() => {
    let active = true;
    setLoading(true);
    setError(null);

    Promise.all([decompilerOpenJar(filePath), decompilerGetStatus()])
      .then(([resOverview, status]) => {
        if (!active) return;
        setOverview(resOverview);
        setDecompilerStatus(status);

        // 默认展开顶层目录
        const defaultExpanded = new Set<string>();
        for (const item of resOverview.tree) {
          if (item.is_dir) {
            defaultExpanded.add(item.path);
            if (item.children) {
              for (const sub of item.children) {
                if (sub.is_dir) defaultExpanded.add(sub.path);
              }
            }
          }
        }
        setExpandedPaths(defaultExpanded);

        // 优先选中 MANIFEST.MF 或第一个类
        let candidate: JarEntryNode | null = null;
        const findFirst = (nodes: JarEntryNode[]) => {
          for (const n of nodes) {
            if (!n.is_dir) {
              if (n.path === 'META-INF/MANIFEST.MF') {
                candidate = n;
                return;
              }
              if (!candidate && n.is_class && !n.is_inner_class) {
                candidate = n;
              }
            } else if (n.children) {
              findFirst(n.children);
            }
          }
        };
        findFirst(resOverview.tree);
        if (candidate) {
          setSelectedNode(candidate);
        }
      })
      .catch((err) => {
        if (!active) return;
        setError(err instanceof Error ? err.message : String(err));
      })
      .finally(() => {
        if (active) setLoading(false);
      });

    return () => {
      active = false;
    };
  }, [filePath]);

  // 2. 初始化与销毁 Monaco 实例
  useEffect(() => {
    if (!editorContainerRef.current) return;

    const editor = monaco.editor.create(editorContainerRef.current, {
      ...MONACO_OPTIONS,
      value: '// 正在加载或选择文件...',
      language: 'java',
    });
    editorRef.current = editor;

    return () => {
      editor.dispose();
      editorRef.current = null;
    };
  }, []);

  // 3. 读取选中条目内容并渲染到 Monaco
  useEffect(() => {
    if (!selectedNode || selectedNode.is_dir) return;

    const entryPath = selectedNode.path;
    const isClass = selectedNode.is_class;
    const language = isClass ? 'java' : entryPath.endsWith('.xml') ? 'xml' : entryPath.endsWith('.json') ? 'json' : 'plaintext';

    // 命中缓存直接更新
    const cached = contentCacheRef.current.get(entryPath);
    if (cached !== undefined) {
      if (editorRef.current) {
        editorRef.current.setValue(cached);
        const model = editorRef.current.getModel();
        if (model) monaco.editor.setModelLanguage(model, language);
        editorRef.current.setScrollTop(0);
      }
      return;
    }

    setContentLoading(true);

    let active = true;
    decompilerReadEntry(filePath, entryPath)
      .then((content) => {
        if (!active) return;
        contentCacheRef.current.set(entryPath, content);
        if (editorRef.current) {
          editorRef.current.setValue(content);
          const model = editorRef.current.getModel();
          if (model) monaco.editor.setModelLanguage(model, language);
          editorRef.current.setScrollTop(0);
        }
      })
      .catch((err) => {
        if (!active) return;
        const msg = err instanceof Error ? err.message : String(err);
        if (editorRef.current) {
          editorRef.current.setValue(`// 读取/反编译失败:\n// ${msg}`);
          const model = editorRef.current.getModel();
          if (model) monaco.editor.setModelLanguage(model, 'plaintext');
        }
      })
      .finally(() => {
        if (active) setContentLoading(false);
      });

    return () => {
      active = false;
    };
  }, [filePath, selectedNode]);

  // 过滤后的树形列表
  const filteredTree = useMemo(() => {
    if (!overview) return [];
    if (!searchQuery.trim()) return overview.tree;

    const results: JarEntryNode[] = [];
    for (const node of overview.tree) {
      const filtered = filterTreeNode(node, searchQuery.trim());
      if (filtered) results.push(filtered);
    }
    return results;
  }, [overview, searchQuery]);

  // 展开 / 折叠全部
  const handleExpandAll = useCallback(() => {
    if (!overview) return;
    const all = new Set<string>();
    const collectDirs = (nodes: JarEntryNode[]) => {
      for (const n of nodes) {
        if (n.is_dir) {
          all.add(n.path);
          if (n.children) collectDirs(n.children);
        }
      }
    };
    collectDirs(overview.tree);
    setExpandedPaths(all);
  }, [overview]);

  const handleCollapseAll = useCallback(() => {
    setExpandedPaths(new Set());
  }, []);

  const handleToggleExpand = useCallback((path: string) => {
    setExpandedPaths((prev) => {
      const next = new Set(prev);
      if (next.has(path)) {
        next.delete(path);
      } else {
        next.add(path);
      }
      return next;
    });
  }, []);

  const handleCopySource = useCallback(() => {
    if (!editorRef.current) return;
    const value = editorRef.current.getValue();
    void navigator.clipboard.writeText(value);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  }, []);

  const handleCopyCoords = useCallback(() => {
    if (!overview?.metadata.group_id) return;
    const coords = `${overview.metadata.group_id}:${overview.metadata.artifact_id || ''}:${overview.metadata.version || ''}`;
    void navigator.clipboard.writeText(coords);
    setCopiedCoords(true);
    setTimeout(() => setCopiedCoords(false), 2000);
  }, [overview]);

  if (loading) {
    return (
      <div className={cn('flex flex-col items-center justify-center h-full gap-3 text-foreground-subtle', className)}>
        <Spinner className="size-6 text-foreground-subtle" />
        <span className="text-ui-base font-medium">{t('files.decompiler.loading', { defaultValue: '正在解析 JAR 归档结构与字节码…' })}</span>
      </div>
    );
  }

  if (error) {
    return (
      <div className={cn('flex flex-col items-center justify-center h-full gap-3 p-6 text-center', className)}>
        <div className="flex size-10 items-center justify-center rounded-full bg-destructive/10 text-destructive">
          <Archive className="size-5" />
        </div>
        <p className="text-ui-base font-medium text-foreground">{t('files.decompiler.openFailed', { defaultValue: '打开 JAR 文件失败' })}</p>
        <p className="max-w-md font-mono text-ui-xs text-destructive">{error}</p>
      </div>
    );
  }

  const meta = overview?.metadata;
  const hasMaven = Boolean(meta?.group_id && meta?.artifact_id);

  return (
    <div className={cn('flex h-full w-full overflow-hidden bg-background text-foreground select-none', className)}>
      {/* ── 左侧面板：搜索 + 元信息 + 目录树 ── */}
      <div className="flex w-72 sm:w-80 shrink-0 flex-col border-r border-border/70 bg-surface/30">
        {/* 顶部搜索框 */}
        <div className="p-2 border-b border-border/50">
          <div className="relative flex items-center">
            <Search className="pointer-events-none absolute left-2.5 size-3.5 text-foreground-subtlest" />
            <input
              type="text"
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              placeholder={t('files.decompiler.searchPlaceholder', { defaultValue: '搜索类名或路径…' })}
              className="h-7.5 w-full rounded-md border border-border/60 bg-surface/60 pl-8 pr-7 text-ui-xs text-foreground placeholder:text-foreground-subtlest focus:border-border focus:outline-none focus:ring-1 focus:ring-border"
            />
            {searchQuery && (
              <button
                type="button"
                onClick={() => setSearchQuery('')}
                className="absolute right-2 text-foreground-subtlest hover:text-foreground"
              >
                <X className="size-3.5" />
              </button>
            )}
          </div>

          <div className="mt-1.5 flex items-center justify-between px-0.5 text-[11px] text-foreground-subtlest">
            <span>
              {meta?.total_classes ?? 0} {t('files.decompiler.classes', { defaultValue: '类' })} / {meta?.total_files ?? 0} {t('files.decompiler.files', { defaultValue: '文件' })}
            </span>
            <div className="flex items-center gap-2">
              <button
                type="button"
                onClick={handleExpandAll}
                className="hover:text-foreground transition-colors"
              >
                {t('files.decompiler.expandAll', { defaultValue: '展开' })}
              </button>
              <span>·</span>
              <button
                type="button"
                onClick={handleCollapseAll}
                className="hover:text-foreground transition-colors"
              >
                {t('files.decompiler.collapseAll', { defaultValue: '折叠' })}
              </button>
            </div>
          </div>
        </div>

        {/* 元信息卡片（Maven GAV / JDK / Main-Class） */}
        {meta && (
          <div className="p-2.5 border-b border-border/50 bg-surface/40 space-y-1.5 text-ui-xs">
            {hasMaven && (
              <div className="flex items-center justify-between gap-1.5 rounded bg-surface/80 p-1.5 border border-border/40">
                <div className="truncate font-mono text-[11px]">
                  <span className="text-foreground-subtle">{meta.group_id}:</span>
                  <span className="font-semibold text-foreground">{meta.artifact_id}</span>
                  {meta.version && <span className="text-terminal-blue font-medium">:{meta.version}</span>}
                </div>
                <Button
                  type="button"
                  variant="ghost"
                  size="icon-xs"
                  onClick={handleCopyCoords}
                  title={t('files.decompiler.copyCoords', { defaultValue: '复制 Maven 坐标' })}
                  className="size-5 shrink-0 text-foreground-subtler hover:text-foreground"
                >
                  {copiedCoords ? <Check className="size-3 text-terminal-green" /> : <Copy className="size-3" />}
                </Button>
              </div>
            )}

            <div className="grid grid-cols-2 gap-1 text-[11px] text-foreground-subtle">
              {meta.build_jdk && (
                <div className="truncate" title={`Build JDK: ${meta.build_jdk}`}>
                  <span className="text-foreground-subtlest">JDK: </span>
                  <span className="font-mono text-foreground">{meta.build_jdk}</span>
                </div>
              )}
              {meta.main_class && (
                <div className="col-span-2 truncate" title={`Main-Class: ${meta.main_class}`}>
                  <span className="text-foreground-subtlest">Main: </span>
                  <span className="font-mono text-foreground">{meta.main_class}</span>
                </div>
              )}
            </div>
          </div>
        )}

        {/* 树状导航列表 */}
        <div className="flex-1 overflow-y-auto p-1.5">
          {filteredTree.length === 0 ? (
            <div className="flex flex-col items-center justify-center p-6 text-center text-ui-xs text-foreground-subtlest">
              <span>{t('files.decompiler.noMatch', { defaultValue: '未找到匹配条目' })}</span>
            </div>
          ) : (
            filteredTree.map((node) => (
              <TreeItem
                key={node.path}
                node={node}
                selectedPath={selectedNode?.path || ''}
                onSelect={setSelectedNode}
                expandedPaths={expandedPaths}
                onToggleExpand={handleToggleExpand}
                depth={0}
              />
            ))
          )}
        </div>
      </div>

      {/* ── 右侧主区域：工具栏 + Monaco 源码展示 ── */}
      <div className="flex flex-1 flex-col overflow-hidden bg-background">
        {/* 条目头部信息栏 */}
        <div className="flex h-9 shrink-0 items-center justify-between border-b border-border/70 px-3 bg-surface/20">
          <div className="flex items-center gap-2 truncate font-mono text-ui-xs">
            {selectedNode?.is_class ? (
              <FileCode className="size-4 shrink-0 text-terminal-blue" />
            ) : (
              <FileText className="size-4 shrink-0 text-foreground-subtle" />
            )}
            <span className="font-medium text-foreground truncate">
              {selectedNode ? selectedNode.path : t('files.decompiler.noSelection', { defaultValue: '未选择条目' })}
            </span>
            {selectedNode && selectedNode.size > 0 && (
              <span className="text-foreground-subtlest text-[11px]">
                ({formatSize(selectedNode.size)})
              </span>
            )}
            {contentLoading && (
              <span className="flex items-center gap-1 text-[11px] text-terminal-blue">
                <Spinner className="size-3" />
                <span>{t('files.decompiler.decompiling', { defaultValue: '反编译中…' })}</span>
              </span>
            )}
          </div>

          <div className="flex items-center gap-2">
            {selectedNode && (
              <Button
                type="button"
                variant="outline"
                size="sm"
                onClick={handleCopySource}
                className="h-6 gap-1 px-2 text-ui-xs font-normal"
              >
                {copied ? <Check className="size-3 text-terminal-green" /> : <Copy className="size-3" />}
                <span>{copied ? t('common.copied', { defaultValue: '已复制' }) : t('files.decompiler.copySource', { defaultValue: '复制代码' })}</span>
              </Button>
            )}
          </div>
        </div>

        {/* JRE 运行环境警告（如果未就绪） */}
        {decompilerStatus && !decompilerStatus.jre_found && (
          <div className="flex items-center gap-2 border-b border-warning/30 bg-warning/10 px-3 py-1.5 text-ui-xs text-warning">
            <Info className="size-3.5 shrink-0" />
            <span>{t('files.decompiler.jreNotReady', { defaultValue: '系统未检测到 Java 17+ 运行环境，无法进行类反编译（文本文件可正常查看）。' })}</span>
          </div>
        )}

        {/* Monaco 编辑器展示区域 */}
        <div className="relative flex-1 overflow-hidden">
          <div ref={editorContainerRef} className="absolute inset-0 h-full w-full" />
        </div>
      </div>
    </div>
  );
});
