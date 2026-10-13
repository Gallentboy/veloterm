import { invoke, isTauri } from '@tauri-apps/api/core';
import {
  debug as tauriDebug,
  error as tauriError,
  info as tauriInfo,
  trace as tauriTrace,
  warn as tauriWarn,
  type LogOptions,
} from '@tauri-apps/plugin-log';
import { getDbKv, setDbKv } from '@/services/dbStorage';

export type LogLevel = 'trace' | 'debug' | 'info' | 'warn' | 'error';

export const LOG_LEVELS: readonly LogLevel[] = [
  'trace',
  'debug',
  'info',
  'warn',
  'error',
] as const;

/**
 * 数值越小优先级越高（与 Rust `log::Level` 一致）：
 * Error(1) < Warn(2) < Info(3) < Debug(4) < Trace(5)
 */
const LOG_LEVEL_SEVERITY: Record<LogLevel, number> = {
  error: 1,
  warn: 2,
  info: 3,
  debug: 4,
  trace: 5,
};

const LOG_LEVEL_STORAGE_KEY = 'veloterm:settings:log-level';

function readStoredLogLevel(): LogLevel | null {
  const raw = getDbKv(LOG_LEVEL_STORAGE_KEY);
  if (!raw) return null;
  return normalizeLogLevel(raw);
}

function writeStoredLogLevel(level: LogLevel): void {
  setDbKv(LOG_LEVEL_STORAGE_KEY, level);
}

let currentFrontendLevel: LogLevel =
  typeof window !== 'undefined' ? (readStoredLogLevel() ?? 'info') : 'info';

function normalizeLogLevel(raw: string | null | undefined): LogLevel {
  const lower = raw?.trim().toLowerCase();
  if (
    lower === 'trace' ||
    lower === 'debug' ||
    lower === 'info' ||
    lower === 'warn' ||
    lower === 'error'
  ) {
    return lower;
  }
  return 'info';
}

function shouldEmit(level: LogLevel): boolean {
  return LOG_LEVEL_SEVERITY[level] <= LOG_LEVEL_SEVERITY[currentFrontendLevel];
}

export async function trace(message: string, options?: LogOptions): Promise<void> {
  if (!shouldEmit('trace')) return;
  if (typeof window !== 'undefined' && isTauri()) {
    await tauriTrace(message, options);
  } else {
    console.debug(`[TRACE] ${message}`);
  }
}

export async function debug(message: string, options?: LogOptions): Promise<void> {
  if (!shouldEmit('debug')) return;
  if (typeof window !== 'undefined' && isTauri()) {
    await tauriDebug(message, options);
  } else {
    console.debug(`[DEBUG] ${message}`);
  }
}

export async function info(message: string, options?: LogOptions): Promise<void> {
  if (!shouldEmit('info')) return;
  if (typeof window !== 'undefined' && isTauri()) {
    await tauriInfo(message, options);
  } else {
    console.info(`[INFO] ${message}`);
  }
}

export async function warn(message: string, options?: LogOptions): Promise<void> {
  if (!shouldEmit('warn')) return;
  if (typeof window !== 'undefined' && isTauri()) {
    await tauriWarn(message, options);
  } else {
    console.warn(`[WARN] ${message}`);
  }
}

export async function error(message: string, options?: LogOptions): Promise<void> {
  if (!shouldEmit('error')) return;
  if (typeof window !== 'undefined' && isTauri()) {
    await tauriError(message, options);
  } else {
    console.error(`[ERROR] ${message}`);
  }
}

/**
 * 动态设置前后端统一日志等级（无需重启应用，立即生效，同时持久化到本地配置）。
 */
export async function setLogLevel(level: LogLevel): Promise<void> {
  const normalized = normalizeLogLevel(level);
  currentFrontendLevel = normalized;
  if (typeof window !== 'undefined') {
    writeStoredLogLevel(normalized);
  }
  if (typeof window !== 'undefined' && isTauri()) {
    await invoke('set_log_level', { level: normalized });
  }
}

/**
 * 获取当前生效的日志等级。
 */
export async function getCurrentLogLevel(): Promise<LogLevel> {
  if (typeof window !== 'undefined' && isTauri()) {
    try {
      const level = await invoke<string>('get_current_log_level');
      const normalized = normalizeLogLevel(level);
      currentFrontendLevel = normalized;
      return normalized;
    } catch {
      return currentFrontendLevel;
    }
  }
  return currentFrontendLevel;
}

/**
 * 获取当前本地日志文件的完整磁盘路径（仅 Tauri 桌面端有效）。
 */
export async function getLogFilePath(): Promise<string | null> {
  if (typeof window !== 'undefined' && isTauri()) {
    try {
      return await invoke<string>('get_log_file_path');
    } catch {
      return null;
    }
  }
  return null;
}

/**
 * 应用启动时同步前端已持久化的日志等级到 Rust 后端，或从后端拉取初始等级。
 */
export async function initLoggerLevel(): Promise<LogLevel> {
  const stored = typeof window !== 'undefined' ? readStoredLogLevel() : null;
  if (stored) {
    currentFrontendLevel = stored;
    if (typeof window !== 'undefined' && isTauri()) {
      try {
        await invoke('set_log_level', { level: stored });
      } catch {
        // 忽略初始化失败
      }
    }
    return stored;
  }
  return getCurrentLogLevel();
}

export const logger = {
  trace,
  debug,
  info,
  warn,
  error,
  setLogLevel,
  getCurrentLogLevel,
  getLogFilePath,
  initLoggerLevel,
};
