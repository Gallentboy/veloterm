import * as monaco from 'monaco-editor';

/**
 * 为 Monaco 编辑器实例挂载跨平台查找 (Ctrl+F / ⌘F) 与替换 (Ctrl+R / ⌘R) 快捷键支持。
 * - 查找：macOS ⌘F，Windows/Linux Ctrl+F（同时容忍外接键盘 Ctrl+F）
 * - 替换：Windows/Linux Ctrl+R，macOS ⌘R（同时容忍外接键盘 Ctrl+R 与 VS Code 原生 Ctrl+H）
 * - 原理：触发 Monaco 内置 actions.find 与 editor.action.startFindReplaceAction
 */
export function attachFindReplaceCommands(editor: monaco.editor.IStandaloneCodeEditor): void {
  const triggerFind = () => {
    const action = editor.getAction('actions.find');
    if (action && action.isSupported()) {
      void action.run();
    } else {
      editor.trigger('keyboard', 'actions.find', null);
    }
  };

  const triggerReplace = () => {
    const action = editor.getAction('editor.action.startFindReplaceAction');
    if (action && action.isSupported()) {
      void action.run();
    } else {
      editor.trigger('keyboard', 'editor.action.startFindReplaceAction', null);
    }
  };

  // 1. 查找 (Find)
  editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyF, triggerFind);
  editor.addCommand(monaco.KeyMod.WinCtrl | monaco.KeyCode.KeyF, triggerFind);

  // 2. 替换 (Replace: 支持 Ctrl+R, ⌘R, 以及 Ctrl+H)
  editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyR, triggerReplace);
  editor.addCommand(monaco.KeyMod.WinCtrl | monaco.KeyCode.KeyR, triggerReplace);
  editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyH, triggerReplace);
}

/**
 * 编辑器 DOM 容器的键盘捕获监听器（Capture 阶段）：
 * 强力阻断 WebKit / WebView2 在按 Ctrl+R 或 ⌘R 时触发页面刷新，
 * 并转为直接唤起 Monaco 替换面板。
 */
export function handleEditorKeyDownCapture(
  event: React.KeyboardEvent | KeyboardEvent,
  editor: monaco.editor.IStandaloneCodeEditor | null,
): void {
  const key = event.key.toLowerCase();
  const isPrimary = event.metaKey || event.ctrlKey;
  if (isPrimary && !event.altKey && !event.shiftKey) {
    if (key === 'r') {
      event.preventDefault();
      event.stopPropagation();
      if (editor) {
        const action = editor.getAction('editor.action.startFindReplaceAction');
        if (action && action.isSupported()) {
          void action.run();
        } else {
          editor.trigger('keyboard', 'editor.action.startFindReplaceAction', null);
        }
      }
    } else if (key === 'f') {
      event.preventDefault();
      event.stopPropagation();
      if (editor) {
        const action = editor.getAction('actions.find');
        if (action && action.isSupported()) {
          void action.run();
        } else {
          editor.trigger('keyboard', 'actions.find', null);
        }
      }
    }
  }
}
