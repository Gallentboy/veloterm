/**
 * PDF 预览服务：后端 PDFium 渲染成 PNG，前端经 asset:// 展示。
 *
 * 渲染在 Rust 侧（spawn_blocking），前端只负责请求-展示-翻页。
 */
import { invoke, convertFileSrc } from '@tauri-apps/api/core';

export interface PdfPageImage {
  /** 临时 PNG 绝对路径 */
  path: string;
  /** 文档总页数 */
  pageCount: number;
  width: number;
  height: number;
}

/**
 * 渲染指定页。`pageIndex` 是 0 基。
 * 返回带 asset URL 的结果（已 convertFileSrc 转换，可直接给 <img src>）。
 */
export async function renderPdfPage(
  pdfPath: string,
  pageIndex: number,
): Promise<PdfPageImage & { url: string }> {
  const result = await invoke<PdfPageImage>('render_pdf_page', { pdfPath, pageIndex });
  return { ...result, url: convertFileSrc(result.path) };
}
