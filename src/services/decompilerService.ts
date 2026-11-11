/**
//! Java JAR/WAR/Class 反编译与结构透视服务层
 */
import { invoke } from '@tauri-apps/api/core';

export interface JarEntryNode {
  path: string;
  name: string;
  is_dir: boolean;
  is_class: boolean;
  is_inner_class: boolean;
  size: number;
  compressed_size: number;
  children?: JarEntryNode[];
}

export interface JarMetadata {
  group_id?: string;
  artifact_id?: string;
  version?: string;
  main_class?: string;
  built_by?: string;
  build_jdk?: string;
  created_by?: string;
  total_classes: number;
  total_files: number;
  manifest_entries: Record<string, string>;
}

export interface JarStructureOverview {
  file_path: string;
  file_size: number;
  metadata: JarMetadata;
  tree: JarEntryNode[];
}

export interface DecompilerStatus {
  jre_found: boolean;
  jre_path?: string;
  jre_version?: string;
  worker_jar_found: boolean;
  vineflower_jar_found: boolean;
  is_running: boolean;
}

/** 打开并解析本地 Jar / War 结构与元数据 */
export async function decompilerOpenJar(filePath: string): Promise<JarStructureOverview> {
  return invoke<JarStructureOverview>('decompiler_open_jar', { filePath });
}

/** 读取指定条目（Class 自动反编译为 Java 源码，文本直接返回） */
export async function decompilerReadEntry(filePath: string, entryPath: string): Promise<string> {
  return invoke<string>('decompiler_read_entry', { filePath, entryPath });
}

/** 获取常驻 Worker 与 JRE 运行环境就绪状态 */
export async function decompilerGetStatus(): Promise<DecompilerStatus> {
  return invoke<DecompilerStatus>('decompiler_get_status');
}

/** 弹出原生文件选择对话框选择本地 JAR / WAR 文件路径 */
export async function decompilerPickLocalJar(): Promise<string | null> {
  return invoke<string | null>('decompiler_pick_local_jar');
}

/** 显式停止常驻反编译守护进程释放内存 */
export async function decompilerStopDaemon(): Promise<void> {
  return invoke<void>('decompiler_stop_daemon');
}
