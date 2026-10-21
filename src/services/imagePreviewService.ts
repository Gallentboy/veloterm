/**
 * 图片预览服务：原生格式直传，其余后端转码为 PNG。
 */
import { invoke, convertFileSrc } from '@tauri-apps/api/core';

export interface ImagePreviewResult {
  /** 可直接给 <img src> 的本地路径 */
  path: string;
  /** 是否经过后端转码 */
  converted: boolean;
  width: number;
  height: number;
}

/** 浏览器原生可直接渲染的图片扩展名（与后端 NATIVE_IMAGE_EXTS 对齐） */
const NATIVE_IMAGE_EXTS = new Set([
  'png', 'jpg', 'jpeg', 'jfif', 'pjpeg', 'pjp', 'gif', 'webp', 'svg', 'bmp', 'ico', 'avif', 'apng',
]);

/** 已知不支持、会给出明确错误原因的格式（与后端 unsupported_reason 对齐） */
const KNOWN_UNSUPPORTED_EXTS = new Set([
  'heic', 'heif', 'jxl', 'jp2', 'j2k', 'jpf', 'jpx',
  'psd', 'psb', 'cr2', 'cr3', 'nef', 'arw', 'dng', 'orf', 'raf', 'rw2',
]);

/** 该文件是否走图片预览分支（原生支持 + 可转码的常见格式） */
export function isPreviewableImage(fileName: string): boolean {
  const ext = fileName.split('.').pop()?.toLowerCase() ?? '';
  if (!ext) return false;
  if (NATIVE_IMAGE_EXTS.has(ext)) return true;
  // 转码格式：排除明确不支持的，其余交给后端试（image crate 按魔数嗅探）
  if (KNOWN_UNSUPPORTED_EXTS.has(ext)) return true; // 仍进预览，让后端给出具体原因
  return ['tif', 'tiff', 'tga', 'dds', 'hdr', 'exr', 'pnm', 'ppm', 'pgm', 'pbm', 'qoi', 'ff', 'farbfeld'].includes(ext);
}

/**
 * 准备图片预览：返回带 asset URL 的结果。
 * 原生格式零转码（WebView 直接解码）；其他格式后端转 PNG。
 */
export async function prepareImagePreview(
  sourcePath: string,
): Promise<ImagePreviewResult & { url: string }> {
  const result = await invoke<ImagePreviewResult>('prepare_image_preview', { sourcePath });
  return { ...result, url: convertFileSrc(result.path) };
}
