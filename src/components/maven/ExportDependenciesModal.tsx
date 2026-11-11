/**
 * Maven 依赖清单导出弹窗：
 * 支持 Markdown 表格、CSV 合规审计、Gradle 依赖块、纯 GAV 清单。
 * 提供实时预览、一键复制到剪贴板、另存为本地文件。
 */
import { Check, Copy, Download } from 'lucide-react';
import { useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Button } from '@/components/ui/button';
import { Modal } from '@/components/ui/modal';
import {
  exportToCsv,
  exportToGavList,
  exportToGradle,
  exportToMarkdown,
} from '@/components/maven/exportUtils';
import type { MavenTreeNode } from '@/services/mavenService';

export type ExportFormat = 'markdown' | 'csv' | 'gradle' | 'gav';

interface ExportDependenciesModalProps {
  nodes: MavenTreeNode[];
  isOpen: boolean;
  onClose: () => void;
}

export function ExportDependenciesModal({
  nodes,
  isOpen,
  onClose,
}: ExportDependenciesModalProps) {
  const { t } = useTranslation();
  const [format, setFormat] = useState<ExportFormat>('markdown');
  const [copied, setCopied] = useState(false);

  const previewContent = useMemo(() => {
    switch (format) {
      case 'markdown':
        return exportToMarkdown(nodes);
      case 'csv':
        return exportToCsv(nodes);
      case 'gradle':
        return exportToGradle(nodes);
      case 'gav':
        return exportToGavList(nodes);
    }
  }, [nodes, format]);

  if (!isOpen) return null;

  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(previewContent);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch (err) {
      console.error('Failed to copy', err);
    }
  };

  const handleDownload = () => {
    let filename = 'dependencies.txt';
    let mime = 'text/plain;charset=utf-8';
    switch (format) {
      case 'markdown':
        filename = 'dependencies.md';
        mime = 'text/markdown;charset=utf-8';
        break;
      case 'csv':
        filename = 'dependencies.csv';
        mime = 'text/csv;charset=utf-8';
        break;
      case 'gradle':
        filename = 'build.gradle.kts';
        mime = 'text/plain;charset=utf-8';
        break;
      case 'gav':
        filename = 'dependencies-gav.txt';
        mime = 'text/plain;charset=utf-8';
        break;
    }

    const blob = new Blob([previewContent], { type: mime });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = filename;
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    URL.revokeObjectURL(url);
  };

  const formats: Array<{ key: ExportFormat; label: string; ext: string }> = [
    { key: 'markdown', label: 'Markdown 表格', ext: '.md' },
    { key: 'csv', label: 'CSV 合规审计表', ext: '.csv' },
    { key: 'gradle', label: 'Gradle 依赖块', ext: '.gradle' },
    { key: 'gav', label: '纯 GAV 坐标清单', ext: '.txt' },
  ];

  return (
    <Modal
      title={t('maven.exportModalTitle')}
      onClose={onClose}
      className="w-[780px] max-w-full h-[82vh]"
      footer={
        <div className="flex w-full items-center justify-between">
          <span className="text-[11px] text-foreground-subtler">
            {t('maven.exportCountHint', { count: nodes.filter((n) => !n.loser).length })}
          </span>
          <div className="flex items-center gap-2">
            <Button variant="outline" size="sm" onClick={onClose}>
              {t('common.cancel')}
            </Button>
            <Button variant="outline" size="sm" onClick={() => void handleCopy()} className="gap-1.5">
              {copied ? <Check className="size-3.5 text-terminal-green" /> : <Copy className="size-3.5" />}
              {copied ? t('maven.copied') : t('maven.copyToClipboard')}
            </Button>
            <Button size="sm" onClick={handleDownload} className="gap-1.5">
              <Download className="size-3.5" />
              {t('maven.downloadFile')}
            </Button>
          </div>
        </div>
      }
    >
      <div className="flex h-full flex-col gap-3 min-h-0">
        {/* 格式选择 Tab 栏 */}
        <div className="flex shrink-0 items-center gap-1.5 rounded-lg border border-border/60 bg-surface/40 p-1">
          {formats.map((f) => (
            <Button
              key={f.key}
              variant={format === f.key ? 'default' : 'ghost'}
              size="xs"
              onClick={() => setFormat(f.key)}
              className="text-ui-xs"
            >
              {f.label}
            </Button>
          ))}
        </div>

        {/* 预览区域 */}
        <div className="scrollbar-slim min-h-0 flex-1 overflow-auto rounded-lg border border-border/50 bg-input/40 p-3">
          <pre className="font-mono text-ui-xs leading-relaxed text-foreground select-text whitespace-pre-wrap break-all">
            {previewContent}
          </pre>
        </div>
      </div>
    </Modal>
  );
}
