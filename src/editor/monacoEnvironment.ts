/**
 * Monaco worker 环境初始化（模块级一次）。
 *
 * worker 走 Rspack 原生的 `new Worker(new URL(...), import.meta.url)` 模式编译为独立 chunk。
 * 注意两点：
 * - `new URL(...)` 必须内联在 `new Worker(...)` 里（外提成辅助函数会退化为按 asset 原样拷贝，
 *   产物里的相对 import 在运行期 404）
 * - 子路径用包 exports 映射（`./*.js` → `./esm/vs/*.js`）展开后的形态：
 *   带 `esm/vs/` 前缀的子路径会被二次映射而无法解析（Rspack 遵循 exports 映射）
 *
 * 首个 Monaco 宿主（编辑器面板 MonacoDocumentCard、SFTP 预览弹窗）挂载时调用；
 * 后续实例共享同一 worker 池。
 */

let monacoEnvironmentReady = false;

export function setupMonacoEnvironment(): void {
  if (monacoEnvironmentReady) return;
  monacoEnvironmentReady = true;
  self.MonacoEnvironment = {
    getWorker(_workerId, label) {
      switch (label) {
        case 'json':
          return new Worker(
            new URL('monaco-editor/language/json/json.worker.js', import.meta.url),
            { type: 'module' },
          );
        case 'typescript':
        case 'javascript':
          return new Worker(
            new URL('monaco-editor/language/typescript/ts.worker.js', import.meta.url),
            { type: 'module' },
          );
        default:
          return new Worker(
            new URL('monaco-editor/editor/editor.worker.js', import.meta.url),
            { type: 'module' },
          );
      }
    },
  };
}
