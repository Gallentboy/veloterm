export const isMacOS =
  typeof navigator !== 'undefined' && /Macintosh|Mac OS X/i.test(navigator.userAgent);

/**
 * macOS 红绿灯安全区宽度（与 ZCode desktopWindowButtonPosition 对齐）：
 * 红绿灯组 x:22 起、3 枚 12px 按钮间距 20px，约 70px，加右侧余量取 96px。
 * 侧边栏顶部交互按钮必须排在这条安全线之后。
 */
export const MACOS_TRAFFIC_LIGHT_LEFT_PADDING_PX = 96;
