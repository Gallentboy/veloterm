import { Channel, invoke } from '@tauri-apps/api/core';

export interface RemoteFileItem {
  name: string;
  path: string;
  is_dir: boolean;
  is_symlink?: boolean;
  size: string;
  modified_at: number;
  permissions: number;
  owner?: string;
  group?: string;
  /** 是否可编辑（后端 text_detect 判定：文本扩展名 / 常见点文件名 / .env 家族 + ≤5MB） */
  editable?: boolean;
}

export interface TransferProgress {
  task_id: number; // 严格对应 Rust 的 usize
  transferred: string;
  total: string;
  speed_bps: number;
  percent: number;
  is_finished: boolean;
}

export interface TransferHandle {
  taskId: number;
  cancel: () => void;
  done: Promise<void>;
}

/**
 * 读取远程 SFTP 目录列表
 */
export async function listRemoteDirectory(
  sessionId: string,
  path: string,
): Promise<RemoteFileItem[]> {
  return invoke<RemoteFileItem[]>('sftp_list_dir', { sessionId, path });
}

/**
 * 建立与远程主机的 SFTP 连接句柄
 */
export async function connectSftp(sessionId: string, hostId: number): Promise<void> {
  return invoke('sftp_connect', { sessionId, hostId });
}

/**
 * 关闭指定会话的 SFTP 连接
 */
export async function closeSftp(sessionId: string): Promise<void> {
  return invoke('sftp_close', { sessionId });
}

/**
 * 异步流式下载远程文件到本地磁盘
 * - 二进制数据流在 Rust 后端直接落盘，不经由 IPC 回传前端
 * - 进度通过 Tauri 2.0 Channel 以 100ms 间隔节流上报
 * - `resumeOffset > 0` 时从断点续传（后端会校验本地现存长度后幂等续写）
 */
export async function downloadRemoteFile(
  sessionId: string,
  remotePath: string,
  localPath: string,
  onUpdate: (progress: TransferProgress) => void,
  speedLimitBps = 0,
  resumeOffset = 0,
): Promise<TransferHandle> {
  const taskId = await invoke<number>('sftp_next_task_id');
  const onProgress = new Channel<TransferProgress>();

  onProgress.onmessage = (progress) => {
    onUpdate(progress);
  };

  const cancel = () => {
    void invoke('sftp_cancel_transfer', { taskId });
  };

  const done = invoke<void>('sftp_download_file', {
    sessionId,
    taskId,
    remotePath,
    localPath,
    resumeOffset: resumeOffset > 0 ? resumeOffset : null,
    speedLimitBps: speedLimitBps > 0 ? speedLimitBps : null,
    onProgress,
  });

  return { taskId, cancel, done };
}

/**
 * 异步流式上传本地文件到远程 SFTP 服务器
 * - 后端写远端同目录临时文件，成功后 rename 原子覆盖
 * - `resumeOffset > 0` 时从远端临时断点续传（失败/取消均保留断点）
 */
export async function uploadLocalFile(
  sessionId: string,
  localPath: string,
  remotePath: string,
  onUpdate: (progress: TransferProgress) => void,
  speedLimitBps = 0,
  resumeOffset = 0,
): Promise<TransferHandle> {
  const taskId = await invoke<number>('sftp_next_task_id');
  const onProgress = new Channel<TransferProgress>();

  onProgress.onmessage = (progress) => {
    onUpdate(progress);
  };

  const cancel = () => {
    void invoke('sftp_cancel_transfer', { taskId });
  };

  const done = invoke<void>('sftp_upload_file', {
    sessionId,
    taskId,
    localPath,
    remotePath,
    resumeOffset: resumeOffset > 0 ? resumeOffset : null,
    speedLimitBps: speedLimitBps > 0 ? speedLimitBps : null,
    onProgress,
  });

  return { taskId, cancel, done };
}

/**
 * 远端到远端流式复制（同会话 = 同主机复制；不同会话 = 跨主机经本机内存管道中转，不落盘）
 * - Rust 后端 reader→writer 直连两条 SFTP 会话，512KB 块流水线转发（在途 ~6MB 有界）
 * - `resumeOffset > 0` 时从目标端临时断点续传（失败/取消均保留断点）
 */
export async function copyRemoteFile(
  sourceSessionId: string,
  sourceRemotePath: string,
  targetSessionId: string,
  targetRemotePath: string,
  onUpdate: (progress: TransferProgress) => void,
  speedLimitBps = 0,
  resumeOffset = 0,
): Promise<TransferHandle> {
  const taskId = await invoke<number>('sftp_next_task_id');
  const onProgress = new Channel<TransferProgress>();

  onProgress.onmessage = (progress) => {
    onUpdate(progress);
  };

  const cancel = () => {
    void invoke('sftp_cancel_transfer', { taskId });
  };

  const done = invoke<void>('sftp_copy_file', {
    taskId,
    sourceSessionId,
    sourcePath: sourceRemotePath,
    targetSessionId,
    targetPath: targetRemotePath,
    resumeOffset: resumeOffset > 0 ? resumeOffset : null,
    speedLimitBps: speedLimitBps > 0 ? speedLimitBps : null,
    onProgress,
  });

  return { taskId, cancel, done };
}

/**
 * 弹出原生文件选择器选择要上传的本地文件（多选；取消返回 null）
 */
export async function pickUploadFiles(): Promise<string[] | null> {
  return invoke<string[] | null>('sftp_pick_upload_files');
}

/**
 * 弹出原生文件保存对话框选择下载目标保存路径
 */
export async function pickDownloadPath(suggestedName: string): Promise<string | null> {
  return invoke<string | null>('sftp_pick_download_path', { suggestedName });
}

/**
 * 弹出原生目录选择器选择批量下载的目标目录（取消返回 null）
 */
export async function pickDownloadDir(): Promise<string | null> {
  return invoke<string | null>('sftp_pick_download_dir');
}

/**
 * 删除远程文件或目录
 */
export async function removeRemoteItem(
  sessionId: string,
  path: string,
  isDir: boolean,
): Promise<void> {
  return invoke<void>('sftp_remove', { sessionId, path, isDir });
}

/** 目录递归删除的扫描结果（truncated=true 表示触计数上限，显示「N+」） */
export interface TreeScanResult {
  nodeCount: number;
  truncated: boolean;
}

/** Shell 极速删除预览：command 为后端与执行同一函数构造的完整命令串 */
export interface FastRemovePreview {
  command: string;
  timeoutSecs: number;
}

/** 纯 SFTP 递归删除进度事件 */
export interface RemoveProgress {
  taskId: number;
  deleted: number;
  failed: number;
  total: number | null;
  isFinished: boolean;
}

/** 纯 SFTP 递归删除结果 */
export interface RemoveTreeOutcome {
  deleted: number;
  failed: number;
}

/**
 * 扫描目录节点数（含根；上限 2000、超时 8s，超限 truncated=true）——
 * 由前端决定是否走「极速删除」询问流程
 */
export async function scanRemoteTree(sessionId: string, path: string): Promise<TreeScanResult> {
  return invoke<TreeScanResult>('sftp_scan_tree', { sessionId, path });
}

/**
 * 极速删除预览：取将与执行完全一致的命令串（红字审批框展示用）。
 * 服务器无 POSIX shell（exec_disabled）时抛错——调用方静默转纯 SFTP 路径
 */
export async function previewFastRemove(
  sessionId: string,
  path: string,
  nodeCount: number | null,
): Promise<FastRemovePreview> {
  return invoke<FastRemovePreview>('sftp_preview_fast_remove', {
    sessionId,
    path,
    nodeCount,
  });
}

/** Shell 极速删除（sh -c 'rm -rf -- "$1"' 形态；失败自动回退由调用方编排） */
export async function removeRemoteFast(
  sessionId: string,
  path: string,
  nodeCount: number | null,
): Promise<void> {
  return invoke<void>('sftp_remove_fast', { sessionId, path, nodeCount });
}

/**
 * 纯 SFTP 逐文件递归删除（进度 + 可取消）：
 * - taskId 与传输任务同一命名空间，cancelTransfer(taskId) 即取消
 * - 取消后后端返回 Err("CANCELLED:<已删数>")——已删不恢复，如实提示
 */
export async function removeRemoteTree(
  sessionId: string,
  path: string,
  totalHint: number | null,
  onUpdate: (progress: RemoveProgress) => void,
): Promise<TransferHandle> {
  const taskId = await invoke<number>('sftp_next_task_id');
  const onProgress = new Channel<RemoveProgress>();
  onProgress.onmessage = (progress) => {
    onUpdate(progress);
  };
  const cancel = () => {
    void invoke('sftp_cancel_transfer', { taskId });
  };
  const done = invoke<RemoveTreeOutcome>('sftp_remove_tree', {
    sessionId,
    path,
    taskId,
    totalHint,
    onProgress,
  }).then(() => undefined);
  return { taskId, cancel, done };
}

/**
 * 目录大小（远端 du：SFTP 递归 stat 大目录既慢又费流量；返回字节数）
 */
export async function dirSizeRemote(sessionId: string, path: string): Promise<number> {
  return invoke<number>('sftp_dir_size', { sessionId, path });
}



