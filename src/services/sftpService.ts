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
 */
export async function downloadRemoteFile(
  sessionId: string,
  remotePath: string,
  localPath: string,
  onUpdate: (progress: TransferProgress) => void,
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
    onProgress,
  });

  return { taskId, cancel, done };
}

/**
 * 异步流式上传本地文件到远程 SFTP 服务器
 * - 本地磁盘文件在 Rust 后端以 64KB 缓冲区异步推送到远程
 * - 进度通过 Tauri 2.0 Channel 以 100ms 间隔节流上报
 */
export async function uploadLocalFile(
  sessionId: string,
  localPath: string,
  remotePath: string,
  onUpdate: (progress: TransferProgress) => void,
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


