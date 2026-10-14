/**
 * WebGL 硬件加速能力探测（供终端渲染器选择使用）。
 *
 * 背景：`@xterm/addon-webgl` 自身**没有**任何「硬件是否可用」的 API，它唯一的检测是
 * `WebglRenderer` 构造函数里的 `getContext('webgl2')` 判空 —— 只要拿得到上下文就挂载。
 * 而浏览器在纯软件光栅化（SwiftShader / WARP / llvmpipe）下同样会返回一个**合法**上下文，
 * 此时 addon 会「成功」挂载却由 CPU 绘制，往往比 xterm 自带的 DOM renderer 更慢。
 *
 * 因此这里额外做一次探测：**只有确认存在硬件加速时才启用 WebGL**，否则直接走 DOM renderer。
 */

export interface HardwareAccelerationResult {
  /** 是否具备任何有效的硬件加速（WebGL 1 或 WebGL 2 且非纯软件光栅模拟） */
  hasHardwareAcceleration: boolean;
  /** WebGL 1.0 是否支持硬件加速 */
  hasWebGL1Hardware: boolean;
  /** WebGL 2.0 是否支持硬件加速 */
  hasWebGL2Hardware: boolean;
  /** 底层 GPU / 渲染器名称（如 SwiftShader、Apple M4 等） */
  renderer: string;
  /** 显卡 / 驱动厂商 */
  vendor: string;
}

/**
 * 纯软件光栅化渲染器黑名单关键词。
 *
 * 实测（headless Chromium + CDP 读 `UNMASKED_RENDERER_WEBGL`）各平台软件后端的字符串形态：
 * - Chromium SwiftShader：`ANGLE (Google, Vulkan 1.3.0 (SwiftShader Device (LLVM 10.0.0) (0x0000C0DE)), SwiftShader driver)`
 * - Windows WARP：`ANGLE (Microsoft, Microsoft Basic Render Driver Direct3D11 vs_5_0 ps_5_0, D3D11)`
 * - Linux Mesa 软件路径：`llvmpipe (LLVM 15.0.7, 256 bits)` / `softpipe` / `Mesa OffScreen`
 * - macOS 软件回退：`Apple Software Renderer`
 *
 * 注意 Chrome 137 起已取消 WebGL 自动回退 SwiftShader、**Windows 改用 WARP**，
 * 所以 `microsoft basic render` 这条在 Windows 上才是真正兜底的那一条。
 */
const SOFTWARE_RENDERER_REGEX =
  /(swiftshader|llvmpipe|softpipe|swrast|software rasterizer|software renderer|mesa offscreen|mesa.*indirect|microsoft basic render|warp)/i;

/** 渲染器名称占位符：区别于真实字符串，用于判断某一级探测是否真的拿到了信息 */
const NOT_AVAILABLE = 'N/A';
const UNKNOWN = 'Unknown';

interface ContextProbe {
  /** 上下文存在且未命中软件光栅化黑名单 */
  isHardware: boolean;
  renderer: string;
  vendor: string;
}

function inspectContext(kind: 'webgl' | 'webgl2'): ContextProbe {
  const canvas = document.createElement('canvas');
  canvas.width = 1;
  canvas.height = 1;

  let gl: WebGLRenderingContext | WebGL2RenderingContext | null = null;
  let isHardware = false;
  let renderer = NOT_AVAILABLE;
  let vendor = NOT_AVAILABLE;

  try {
    // `failIfMajorPerformanceCaveat` 请求浏览器在「只能给到软件光栅化」时直接返回 null。
    // 但实测 Chromium 在 SwiftShader / WARP 下**仍会返回上下文**（该提示并非可靠判据），
    // 所以它只当第一道门，真正的判定必须靠下面的渲染器字符串黑名单。
    gl =
      kind === 'webgl2'
        ? canvas.getContext('webgl2', { failIfMajorPerformanceCaveat: true })
        : canvas.getContext('webgl', { failIfMajorPerformanceCaveat: true });

    if (gl) {
      const debugInfo = gl.getExtension('WEBGL_debug_renderer_info');
      if (debugInfo) {
        renderer = String(gl.getParameter(debugInfo.UNMASKED_RENDERER_WEBGL) || UNKNOWN);
        vendor = String(gl.getParameter(debugInfo.UNMASKED_VENDOR_WEBGL) || UNKNOWN);
      } else {
        // 部分 WebView 不暴露 UNMASKED_*，退回被掩码的通用值（此时不据此判死）
        renderer = String(gl.getParameter(gl.RENDERER) || 'Masked');
        vendor = String(gl.getParameter(gl.VENDOR) || 'Masked');
      }

      // 双重校验：即使绕过了 PerformanceCaveat，命中软件黑名单仍视为无硬件加速
      isHardware = !SOFTWARE_RENDERER_REGEX.test(renderer);
    }
  } catch {
    isHardware = false;
  } finally {
    if (gl) {
      // 探测用的上下文必须立刻归还：浏览器单页面活跃 WebGL 上下文有 8~16 个的硬上限，
      // 而终端 LRU 池本身还要占额度（见 terminalSessionRegistry）。
      gl.getExtension('WEBGL_lose_context')?.loseContext();
    }
    canvas.width = 0;
    canvas.height = 0;
  }

  return { isHardware, renderer, vendor };
}

function probeHardwareAcceleration(): HardwareAccelerationResult {
  // 非浏览器 / SSR 环境兜底
  if (typeof window === 'undefined' || typeof document === 'undefined') {
    return {
      hasHardwareAcceleration: false,
      hasWebGL1Hardware: false,
      hasWebGL2Hardware: false,
      renderer: 'SSR Environment',
      vendor: UNKNOWN,
    };
  }

  const webgl2 = inspectContext('webgl2');
  const webgl1 = inspectContext('webgl');

  // 优先报告 WebGL2 的渲染器信息（终端 WebGL 渲染器走的正是 WebGL2）
  const useWebgl2Info = webgl2.renderer !== NOT_AVAILABLE;

  return {
    hasHardwareAcceleration: webgl2.isHardware || webgl1.isHardware,
    hasWebGL1Hardware: webgl1.isHardware,
    hasWebGL2Hardware: webgl2.isHardware,
    renderer: useWebgl2Info ? webgl2.renderer : webgl1.renderer,
    vendor: useWebgl2Info ? webgl2.vendor : webgl1.vendor,
  };
}

/** 探测结果缓存：GPU 能力在同一进程内不会变化，且探测本身要创建/销毁上下文，只应做一次。 */
let cachedResult: HardwareAccelerationResult | null = null;

/**
 * 检测当前环境是否开启并支持 WebGL 硬件加速（结果缓存，首次调用时探测）。
 */
export function checkHardwareAcceleration(): HardwareAccelerationResult {
  if (!cachedResult) {
    cachedResult = probeHardwareAcceleration();
    console.info(
      '[webgl] hardware acceleration probe:',
      `WebGL2=${cachedResult.hasWebGL2Hardware}`,
      `WebGL1=${cachedResult.hasWebGL1Hardware}`,
      `renderer="${cachedResult.renderer}"`,
      `vendor="${cachedResult.vendor}"`,
    );
  }
  return cachedResult;
}

/**
 * `@xterm/addon-webgl` 只支持 WebGL2，所以启用前必须用这一条判定，
 * 而不是 `hasHardwareAcceleration` —— 后者在「仅 WebGL1 硬件加速」时也为 `true`，
 * 此时挂载 addon 仍会抛 `WebGL2 not supported`。
 */
export function hasWebgl2HardwareAcceleration(): boolean {
  return checkHardwareAcceleration().hasWebGL2Hardware;
}
