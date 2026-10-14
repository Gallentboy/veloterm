export const isMacOS =
  typeof navigator !== 'undefined' && /Macintosh|Mac OS X/i.test(navigator.userAgent);

/**
 * macOS 红绿灯安全区宽度（与 ZCode desktopWindowButtonPosition 对齐）：
 * 红绿灯组 x:22 起、3 枚 12px 按钮间距 20px，约 70px，加右侧余量取 96px。
 * 侧边栏顶部交互按钮必须排在这条安全线之后。
 */
export const MACOS_TRAFFIC_LIGHT_LEFT_PADDING_PX = 96;

/** 运行平台归类（供 CSS 按平台切换字体栈等分支）。 */
export type PlatformKind = 'macos' | 'ios' | 'windows' | 'linux' | 'android' | 'other';

function detectPlatformKind(): PlatformKind {
  if (typeof navigator === 'undefined') {
    return 'other';
  }
  const ua = navigator.userAgent;
  // iPadOS 13+ 的 UA 与 macOS 一致，靠触点数区分
  if (
    /iPad|iPhone|iPod/.test(ua) ||
    (navigator.platform === 'MacIntel' && navigator.maxTouchPoints > 1)
  ) {
    return 'ios';
  }
  if (/Macintosh|Mac OS X/i.test(ua)) {
    return 'macos';
  }
  if (/Android/i.test(ua)) {
    return 'android';
  }
  if (/Windows/i.test(ua)) {
    return 'windows';
  }
  if (/Linux/i.test(ua)) {
    return 'linux';
  }
  return 'other';
}

/** 当前运行平台。 */
export const platformKind: PlatformKind = detectPlatformKind();

/** 非 macOS / 非 iOS —— 即使用 HarmonyOS Sans SC 作为首选界面字体的平台。 */
export const usesHarmonyOsSansFont = platformKind !== 'macos' && platformKind !== 'ios';

/**
 * 把平台标记写入 `<html data-platform>`，供 CSS 按平台选择字体栈
 * （非 macOS / 非 iOS 走 HarmonyOS Sans SC 首选）。与 `applyTheme` 同为首屏渲染前调用。
 */
export function applyPlatformAttribute(): void {
  if (typeof document === 'undefined') {
    return;
  }
  document.documentElement.dataset.platform = platformKind;
}
