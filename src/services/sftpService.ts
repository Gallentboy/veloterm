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
  /** 是否可编辑（后端 text_detect 判定：文本扩展名 + ≤5MB） */
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
 * 弹出原生文件选择器选择要上传的本地文件
 */
export async function pickUploadFile(): Promise<string | null> {
  return invoke<string | null>('sftp_pick_upload_file');
}

/**
 * 弹出原生文件保存对话框选择下载目标保存路径
 */
export async function pickDownloadPath(suggestedName: string): Promise<string | null> {
  return invoke<string | null>('sftp_pick_download_path', { suggestedName });
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


