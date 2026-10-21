/**
 * 自建 Monaco 语言支持（nginx / dockerfile / docker-compose）的统一幂等
 * 注册入口。各语言模块自身另有幂等与数据懒加载（独立 async chunk，不打开
 * 对应文件的会话零数据成本）；宿主（编辑器面板 / SFTP 预览 / diff 弹窗）
 * 只需调用本函数一次。bash 补全因需宿主模型关联（registerBashHostModel）
 * 不并入，仍由宿主各自调用。
 */

import type * as MonacoNs from 'monaco-editor';

import { ensureComposeLanguage } from './composeLanguage';
import { ensureDockerfileLanguage } from './dockerfileLanguage';
import { ensureNginxLanguage } from './nginxLanguage';

export function ensureExtendedLanguages(monaco: typeof MonacoNs): void {
  ensureNginxLanguage(monaco);
  ensureDockerfileLanguage(monaco);
  ensureComposeLanguage(monaco);
}
