/**
 * JavaScript 安全整数上限（对应 Rust `crate::common::JS_MAX_SAFE_INTEGER` = $2^{53} - 1$）。
 */
export const JS_MAX_SAFE_INTEGER = Number.MAX_SAFE_INTEGER;

/**
 * 创建与 Rust `SafeUsizeCounter` 语义一致的 JS 安全范围自增计数器：
 * 当计数到达 `JS_MAX_SAFE_INTEGER` 极限时自动从 `0` 重新开始循环。
 */
export function createSafeUsizeCounter(initial = 0): () => number {
  let current = initial;
  return () => {
    const value = current;
    current = current >= JS_MAX_SAFE_INTEGER ? 0 : current + 1;
    return value;
  };
}
