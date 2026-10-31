/**
 * Maven 仓库助手服务层：对接 src-tauri/src/maven/mod.rs 的 7 条命令。
 * 事件流（MavenEvent）与 Channel 流式三行式（fleetExecService 同型）；
 * runId 前端生成（时间戳+序号），取消命令 maven_cancel_run 按 runId 寻址。
 */
import { invoke } from '@tauri-apps/api/core';
import { Channel } from '@tauri-apps/api/core';

// ---------------- 事件 ----------------

export type MavenEvent =
  | { type: 'log'; level: number; message: string }
  | { type: 'downloadStart'; rel: string }
  | { type: 'downloadDone'; rel: string; size: string; speed: string; took: string }
  | { type: 'uploadStart'; rel: string }
  | { type: 'uploadDone'; rel: string; size: string; took: string }
  | { type: 'stats'; done: number; total: number; active: number; queued: number; failed: number; bytes: number };

// ---------------- DTO ----------------

export interface MavenGetRequest {
  gav?: string | null;
  pomPath?: string | null;
  gavList?: string | null;
  output?: string | null;
  layout?: string | null;
  settingsPath?: string | null;
  repos?: string[] | null;
  define?: string[] | null;
  updateSnapshots?: boolean | null;
  allClassifiers?: boolean | null;
  sources?: boolean | null;
  checksum?: string | null;
  threads?: number | null;
  noTransitive?: boolean | null;
  proxy?: string | null;
}

export interface MavenTreeNode {
  id: number;
  parent: number | null;
  depth: number;
  group: string;
  artifact: string;
  version: string;
  packaging: string;
  classifier: string | null;
  scope: string;
  loser: boolean;
  conflictWith: string | null;
  managedFrom: string | null;
  children: number[];
}

export interface MavenFailedItem {
  item: string;
  error: string;
}

export interface MavenResolveResult {
  nodes: MavenTreeNode[];
  winners: string[];
  treeText: string;
  failures: MavenFailedItem[];
}

export interface MavenGetSummary {
  downloaded: number;
  skipped: number;
  pomDownloaded: number;
  pomCacheHits: number;
  failed: MavenFailedItem[];
  bytes: number;
  elapsedMs: number;
  cancelled: boolean;
  resolvedTotal: number;
  outputDir: string;
  reportPath: string | null;
}

export interface SearchHit {
  group: string;
  artifact: string;
  latest: string;
  extra: string;
}

export interface ProbeCell {
  name: string;
  /** 1=存在 ✓ / 0=不存在 ✗ / 2=无法判定 ? */
  state: number;
}

export interface VersionProbe {
  version: string;
  pomOk: boolean;
  extras: ProbeCell[];
}

export interface VersionSuggestion {
  version: string;
  tag: string;
  extras: ProbeCell[];
}

export type SearchOutcome =
  | { type: 'hits'; hits: SearchHit[] }
  | { type: 'versions'; group: string; artifact: string; versions: VersionProbe[]; unknownCount: number }
  | {
      type: 'existence';
      group: string;
      artifact: string;
      version: string;
      exists: boolean;
      pomOk: boolean;
      extras: ProbeCell[];
      suggestions: VersionSuggestion[];
    };

export interface PutPlanItem {
  local: string;
  rel: string;
}

export interface MavenPutPlan {
  items: PutPlanItem[];
  skipped: number;
}

export interface PutSummary {
  uploaded: number;
  skipped: number;
  uploadedList: string[];
  skippedList: string[];
  failed: MavenFailedItem[];
  bytes: number;
  elapsedMs: number;
  cancelled: boolean;
  reportPath: string | null;
}

export interface MavenEnvInfo {
  settingsPath: string | null;
  settingsExists: boolean;
  localRepository: string;
  defaultRepoUrl: string;
}

// ---------------- 命令封装 ----------------

let runSeq = 0;
export function nextMavenRunId(): string {
  runSeq += 1;
  return `maven-${Date.now()}-${runSeq}`;
}

export function mavenResolve(request: MavenGetRequest): Promise<MavenResolveResult> {
  return invoke('maven_resolve', { request });
}

export function mavenGetRun(
  runId: string,
  request: MavenGetRequest,
  onEvent: (e: MavenEvent) => void,
): Promise<MavenGetSummary> {
  const channel = new Channel<MavenEvent>();
  channel.onmessage = onEvent;
  return invoke('maven_get_run', { runId, request, onEvent: channel });
}

export function mavenCancelRun(runId: string): Promise<boolean> {
  return invoke('maven_cancel_run', { runId });
}

export function mavenSearch(params: {
  query: string;
  repo?: string | null;
  username?: string | null;
  password?: string | null;
  versions?: boolean;
  checkJar?: boolean;
  classifiers?: string[] | null;
  rows?: number;
}): Promise<SearchOutcome> {
  return invoke('maven_search', {
    query: params.query,
    repo: params.repo ?? null,
    username: params.username ?? null,
    password: params.password ?? null,
    versions: params.versions ?? false,
    checkJar: params.checkJar ?? false,
    classifiers: params.classifiers ?? null,
    rows: params.rows ?? 20,
  });
}

export function mavenPutPlan(dir: string): Promise<MavenPutPlan> {
  return invoke('maven_put_plan', { dir });
}

export interface PutRunParams {
  dir: string;
  repo: string;
  username?: string | null;
  password?: string | null;
  threads?: number | null;
  checkRepo?: string | null;
  checkUsername?: string | null;
  checkPassword?: string | null;
}

export function mavenPutRun(
  runId: string,
  params: PutRunParams,
  onEvent: (e: MavenEvent) => void,
): Promise<PutSummary> {
  const channel = new Channel<MavenEvent>();
  channel.onmessage = onEvent;
  return invoke('maven_put_run', {
    runId,
    dir: params.dir,
    repo: params.repo,
    username: params.username ?? null,
    password: params.password ?? null,
    threads: params.threads ?? null,
    checkRepo: params.checkRepo ?? null,
    checkUsername: params.checkUsername ?? null,
    checkPassword: params.checkPassword ?? null,
    onEvent: channel,
  });
}

export function mavenEnvInfo(): Promise<MavenEnvInfo> {
  return invoke('maven_env_info');
}
