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
