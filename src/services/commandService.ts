import { invoke } from '@tauri-apps/api/core';

/** 命令参数（占位符说明，与 Rust CommandParam 对齐） */
export interface CommandParam {
  name: string;
  label: string;
  description: string;
  required: boolean;
  default: string;
  example: string;
}

export type DangerLevel = 'safe' | 'confirm' | 'destructive';

/** 常用命令记录（与 Rust CommandRecord 对齐，camelCase） */
export interface CommandRecord {
  id: number;
  title: string;
  command: string;
  description: string;
  purpose: string;
  category: string;
  tags: string[];
  /** 适配系统多选（win/linux/mac 子集；空 = 通用全部平台，旧数据/旧导入后端补默认） */
  platforms: string[];
  params: CommandParam[];
  dangerLevel: DangerLevel;
  exampleOutput: string;
  usageCount: number;
  lastUsedAt: number | null;
  isPinned: boolean;
  createdAt: number;
  updatedAt: number;
}

/** 全字段模糊搜索（空 keyword 返回全量） */
export function listCommands(keyword?: string): Promise<CommandRecord[]> {
  return invoke<CommandRecord[]>('command_list', { keyword: keyword ?? null });
}

export function saveCommand(record: CommandRecord): Promise<CommandRecord> {
  return invoke<CommandRecord>('command_save', { record });
}

export function deleteCommand(id: number): Promise<void> {
  return invoke<void>('command_delete', { id });
}

export function markCommandUsed(id: number): Promise<void> {
  return invoke<void>('command_used', { id });
}

export function exportCommands(): Promise<string> {
  return invoke<string>('command_export');
}

export function importCommands(json: string): Promise<number> {
  return invoke<number>('command_import', { json });
}

/** 导出文件格式：json 明文（可读） / vtcmd 二进制（带校验） */
export type ExportFileFormat = 'json' | 'vtcmd';

/** 导入冲突处理策略：skip 跳过已存在 / overwrite 覆盖已存在 / duplicate 全部作为副本导入 */
export type ImportStrategy = 'skip' | 'overwrite' | 'duplicate';

/** command_export_file 结果；用户在保存对话框取消时后端返回 null */
export interface ExportFileResult {
  path: string;
  count: number;
}

/** 导入预览：dry-run 比对出的 新增 / 冲突 / 无效 三分类 */
export interface ImportPreview {
  newCount: number;
  conflicts: Array<{ uuid: string | null; title: string; command: string }>;
  invalid: Array<{ index: number; title: string | null; reason: string }>;
}

/** command_import_start 结果（打开对话框选定文件 + 解析 + 预览）；取消返回 null */
export interface ImportStartResult {
  path: string;
  preview: ImportPreview;
}

/** command_import_apply 结果 */
export interface ImportApplyResult {
  inserted: number;
  updated: number;
  skipped: number;
  invalid: number;
}

/** 导出命令库到文件（后端弹保存对话框，用户取消返回 null） */
export function exportCommandsToFile(format: ExportFileFormat): Promise<ExportFileResult | null> {
  return invoke<ExportFileResult | null>('command_export_file', { format });
}

/** 导入第一步：后端弹打开对话框并解析预览（用户取消返回 null） */
export function startImportCommands(): Promise<ImportStartResult | null> {
  return invoke<ImportStartResult | null>('command_import_start');
}

/** 导入第二步：按策略落地（path 来自 startImportCommands 返回值） */
export function applyImportCommands(path: string, strategy: ImportStrategy): Promise<ImportApplyResult> {
  return invoke<ImportApplyResult>('command_import_apply', { path, strategy });
}

/** 从命令模板中提取 {{占位符}} 名称列表 */
export function extractPlaceholders(command: string): string[] {
  const matches = command.match(/\{\{([^}]+)\}\}/g) ?? [];
  return [...new Set(matches.map((m) => m.slice(2, -2).trim()))];
}

/** 用参数值替换命令模板中的 {{占位符}} */
export function resolveCommand(
  command: string,
  values: Record<string, string>,
): string {
  return command.replace(/\{\{([^}]+)\}\}/g, (_, name: string) => values[name.trim()] ?? `{{${name}}}`);
}
