/**
 * JS 脚本运行服务：`js_run` / `js_run_cancel` 的薄封装。
 * 后端见 src-tauri/src/js_runtime.rs 的命令层——console 逐行经 Tauri
 * Channel 流式回传（fleet_exec 同款 tag=type 模式），终态（done/error）
 * 亦经同一 Channel；invoke 本身在结束时 resolve。runId 由前端生成，
 * 取消命令按它寻址（dbQuery 取消同型）。
 */

import { Channel, invoke } from '@tauri-apps/api/core';

/** console 行级别（后端 ConsoleLevel 的 camelCase 形态） */
export type JsConsoleLevel = 'log' | 'warn' | 'error';

/** 脚本语言（后端 Typescript 时先经 oxc 转译再执行） */
export type JsScriptLanguage = 'javascript' | 'typescript';

/** 终态错误种类（后端 JsRunError 映射） */
export type JsRunErrorKind = 'timeout' | 'heapLimit' | 'script' | 'cancelled' | 'host';

export type JsRunEvent =
  | { type: 'console'; level: JsConsoleLevel; text: string; position: string | null }
  | { type: 'done'; resultJson: string | null; elapsedMs: number }
  | { type: 'error'; kind: JsRunErrorKind; message: string; elapsedMs: number };

/** 前端聚合的运行终态（done/error 事件后的 UI 状态） */
export interface JsRunFinish {
  ok: boolean;
  /** 末表达式 JSON（ok 时可能有；undefined 为 null） */
  resultJson: string | null;
  /** 失败种类（ok 时为 null） */
  errorKind: JsRunErrorKind | null;
  /** 失败消息（ok 时为 null） */
  errorMessage: string | null;
  elapsedMs: number;
}

/**
 * 运行一段 JS。onEvent 收流式事件；返回的 Promise 在运行结束后 resolve
 * （终态事件先于或同时于 resolve 到达，UI 以终态事件为准）。
 */
export function jsRun(
  runId: string,
  source: string,
  onEvent: (event: JsRunEvent) => void,
  options?: { timeoutMs?: number; heapLimitMb?: number; language?: JsScriptLanguage },
): Promise<void> {
  const channel = new Channel<JsRunEvent>();
  channel.onmessage = onEvent;
  return invoke('js_run', {
    runId,
    source,
    timeoutMs: options?.timeoutMs,
    heapLimitMb: options?.heapLimitMb,
    language: options?.language,
    onEvent: channel,
  });
}

/** 取消一次运行；runId 已结束（不存在）返回 false 而非报错。 */
export function jsRunCancel(runId: string): Promise<boolean> {
  return invoke('js_run_cancel', { runId });
}
