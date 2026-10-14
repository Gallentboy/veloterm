import { useSyncExternalStore } from 'react';
import {
  deleteCommand,
  listCommands,
  markCommandUsed,
  saveCommand,
  type CommandRecord,
} from '@/services/commandService';

/**
 * 常用命令库全局状态：useSyncExternalStore 模式（与 transferQueue 同架构）。
 * 任何页面/组件都能读写命令列表，修改后广播刷新。
 */
class CommandStore {
  private records: CommandRecord[] = [];
  private listeners = new Set<() => void>();
  private loaded = false;

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  getSnapshot = (): CommandRecord[] => this.records;

  isLoaded = (): boolean => this.loaded;

  async refresh(): Promise<void> {
    try {
      this.records = await listCommands();
      this.loaded = true;
    } catch {
      this.records = [];
    }
    this.emit();
  }

  async save(record: CommandRecord): Promise<CommandRecord> {
    const saved = await saveCommand(record);
    await this.refresh();
    return saved;
  }

  async remove(id: number): Promise<void> {
    await deleteCommand(id);
    await this.refresh();
  }

  async used(id: number): Promise<void> {
    await markCommandUsed(id);
    // 不刷新全量（避免打断列表滚动），下次 refresh 时同步
  }

  private emit(): void {
    for (const listener of this.listeners) {
      listener();
    }
  }
}

export const commandStore = new CommandStore();

/** React 钩子：订阅全局命令列表 */
export function useCommands(): CommandRecord[] {
  return useSyncExternalStore(commandStore.subscribe, commandStore.getSnapshot);
}
