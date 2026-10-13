import type { ITheme } from '@xterm/xterm';
import type { AuthConfig, SshHostKeyVerificationMode } from '@/hosts/types';

/**
 * 终端底层驱动类型（对齐 Rust 端 `TerminalBackendKind`）：
 * - `'local'`：本地 `portable-pty` 进程；
 * - `'ssh'`：远程 `russh` SSH Channel。
 */
export type TerminalBackendKind = 'local' | 'ssh';

/**
 * 单个跳板机或 SSH 目标节点配置（对齐 Rust 端 `TargetSSHOptions`）。
 */
export interface SshJumpHopTarget {
  host: string;
  port: number;
  username: string;
  auth: AuthConfig;
  allowInsecureAlgos?: boolean;
  hostKeyVerification?: SshHostKeyVerificationMode;
}

/**
 * 终端连接目标（对齐 Rust 端 `TerminalTarget`）：
 * - `type: 'local'`：本机默认登录 Shell；
 * - `type: 'ssh'`：远程主机 SSH 会话（承载 `warpgate-protocol-ssh` 迁移的 `russh` 客户端，支持多级跳板机）。
 */
export type TerminalTarget =
  | {
      type: 'local';
      cwd?: string;
      shell?: string;
    }
  | {
      type: 'ssh';
      hostId?: number;
      host?: string;
      port?: number;
      username?: string;
      auth?: AuthConfig;
      allowInsecureAlgos?: boolean;
      hostKeyVerification?: SshHostKeyVerificationMode;
      jumpHosts?: SshJumpHopTarget[];
    };

/**
 * Tauri Rust 后端 `tauri::ipc::Channel<Vec<u8>>` 传输到前端的原始负载形态：
 * - 标准 serde `Channel<Vec<u8>>` 序列化为 `number[]`；
 * - 若使用 `tauri::ipc::Response::new(Vec<u8>)` 零拷贝通道则为 `ArrayBuffer` 或 `Uint8Array`。
 * 前端统一通过 `normalizeChannelBytes` 转为 `Uint8Array` 后直接喂给 `xterm.write(Uint8Array)`。
 */
export type TerminalChannelPayload = Uint8Array | ArrayBuffer | number[];

/**
 * 对应 Tauri `Channel<T>` 的双端契约：
 * 在 Tauri WebView 环境下直接实例化 `@tauri-apps/api/core` 的 `Channel<TerminalChannelPayload>`，
 * 在浏览器纯前端调试或 Mock 模式下提供完全一致的 `onmessage` 接口。
 */
export interface ITerminalByteChannel {
  readonly id?: number;
  onmessage: (payload: TerminalChannelPayload) => void;
  /** 供前端 Mock 传输层直接推入 `Vec<u8>` 字节流 */
  sendMockBytes?: (payload: TerminalChannelPayload) => void;
}

export interface TerminalSize {
  cols: number;
  rows: number;
}

export interface TerminalCreateParams {
  /** 前端会话 Tab ID (usize) */
  sessionId: number;
  cols: number;
  rows: number;
  cwd?: string;
  /** 可选关联的远程主机 ID；缺省表示本地 Shell 终端 */
  hostId?: number;
  /** 显式连接目标（Local PTY 或远程 SSH）；不传时后端根据 `hostId` / `cwd` 自动推导 */
  target?: TerminalTarget;
  /**
   * 对应 Rust 后端参数 `on_output: tauri::ipc::Channel<Vec<u8>>`。
   * 后端将 PTY / SSH stdout+stderr 字节块通过 `on_output.send(vec_u8)` 实时推给前端。
   */
  onOutput: ITerminalByteChannel;
  /** 会话进程退出回调（对应 Rust 后端参数 `on_exit: tauri::ipc::Channel<i32>`） */
  onExit?: (exitCode: number) => void;
}

export interface TerminalCreateResult {
  /** 后端分配的 PTY / SSH 会话唯一 ID (usize) */
  id: number;
  /** 底层驱动类型：`local` (`portable-pty`) 或 `ssh` (`russh`) */
  kind?: TerminalBackendKind;
  /** 解析出的 shell 名称或协议标签（如 `/bin/zsh`, `pwsh`, `ssh`） */
  shell: string;
  cwd?: string;
  fontFamily?: string;
  fontSize?: number;
  theme?: ITheme;
}

export interface TerminalWriteParams {
  /** 后端分配的终端 ID (usize) */
  id: number;
  /**
   * 对应 Rust 后端参数 `data: Vec<u8>`。
   * 前端将 xterm `onData` / `onBinary` 统一编码为 `Uint8Array` 发送。
   */
  data: Uint8Array;
}

export interface TerminalResizeParams {
  id: number;
  cols: number;
  rows: number;
}

export interface TerminalDisposeParams {
  id: number;
}

/**
 * 终端底层传输接口（统一承载本地 `portable-pty`、未来 `russh` 与纯浏览器 Mock Echo）：
 * Tauri 桌面环境下默认启用 `TauriChannelTerminalTransport`，纯浏览器环境自动回退 `MockEchoTerminalTransport`。
 */
export interface ITerminalTransport {
  create(params: TerminalCreateParams): Promise<TerminalCreateResult>;
  write(params: TerminalWriteParams): Promise<void>;
  resize(params: TerminalResizeParams): Promise<void>;
  dispose(params: TerminalDisposeParams): Promise<void>;
}
