/**
 * 主机/分组数据模型 v2（持久化于 Rust 后端 SQLCipher 加密数据库 `veloterm.db`）。
 *
 * - id 均为数字自增主键（分组、主机各自独立序列），对应后端 i64；
 * - 分组（Group）独立成树（parentId），在分组管理页维护，可无限嵌套；
 * - 主机（Host）与分组多对多（groupIds 数组），一台主机可属多个分组；
 * - 时间戳为毫秒整数，对应后端 i64。
 * 两个集合都用「扁平记录 + parentId/groupIds」存储，树形关系靠过滤推导，
 * 与 SQL 行式存储 (`groups`, `hosts`, `host_groups`, `sessions`) 一一对应。
 */

/** 认证方式：用户名密码 / 公钥私钥 / 本地 SSH Agent 三选一 */
export type AuthConfig =
  | { method: 'password'; password: string }
  | { method: 'key'; privateKey: string; passphrase?: string }
  | { method: 'agent' };

export type AuthMethod = AuthConfig['method'];

/** 主机公钥校验策略（对齐后端 `SshHostKeyVerificationMode`） */
export type SshHostKeyVerificationMode = 'auto_accept' | 'auto_reject' | 'ignore';

/** 主机/分组所属的环境级别：开发 / 测试 / 预发 / 生产 */
export type HostEnvironmentLevel = 'dev' | 'test' | 'staging' | 'prod';

/** 中继入站传输类型（对齐后端 `RelayTransport`，veloterm_relay 的监听协议） */
export type RelayTransport = 'tcp' | 'ws' | 'quic';

/** SSH 中继配置（veloterm_relay）：有值时该主机经中继连接（首跳传输替换）。
 *  目标无需配置：客户端自动携带所连主机地址作为中继目标 */
export interface SshRelayEndpoint {
  /** 中继服务器地址 */
  host: string;
  port: number;
  transport: RelayTransport;
  /** 凭证 key_id（veloterm_relay token generate --key-id） */
  keyId: number;
  /** 32 字节十六进制 secret（veloterm_relay token generate 生成） */
  secret: string;
  /** QUIC 证书指纹 pin（SHA-256，服务端 cert generate 输出，冒号分隔或裸十六进制）；
   *  有值时强校验不匹配拒连（防中间人）；tcp/ws 忽略 */
  certFingerprint?: string | null;
}

/** shell 内 passwd 改密提示词配置（对齐后端 `PasswdPrompts`，存 hosts.passwd_prompts JSON 列）。
 *  三条都非空时启用严格匹配（改密成功后自动回写主机密码），任一为空用内置默认模式 */
export interface PasswdPrompts {
  /** 当前密码提示词，如 "(current) UNIX password:" */
  current: string;
  /** 新密码提示词，如 "New password:" */
  new: string;
  /** 确认新密码提示词，如 "Retype new password:" */
  retype: string;
}

export interface GroupNode {
  id: number;
  /** 父分组 id；null 表示根级 */
  parentId: number | null;
  name: string;
  createdAt: number;
  updatedAt: number;
}

export interface HostNode {
  id: number;
  /** 显示名（主机名） */
  name: string;
  host: string;
  port: number;
  username: string;
  auth: AuthConfig;
  /** 前置跳板机主机 id；null 或缺省表示直连 */
  jumpHostId?: number | null;
  /** 中继库引用（侧栏「中继」页配置 id）；嵌入式 relay 为空且此值有效时生效 */
  relayId?: number | null;
  /** SSH 中继配置（嵌入式）；null 或缺省 = 不经中继（与跳板链可组合：中继送达链首端点） */
  relay?: SshRelayEndpoint | null;
  /** 主机公钥校验策略；缺省为 'auto_accept'（首次连接自动信任 TOFU） */
  hostKeyVerification?: SshHostKeyVerificationMode;
  /** 是否允许兼容旧版不安全算法（ssh-rsa / dh-group1-sha1 / 3des-cbc 等）；缺省为 false */
  allowInsecureAlgos?: boolean;
  /** 终端就绪钩子：会话建立后自动执行的命令库命令 id；null 或缺省 = 未启用（仅存主键 id，命令被删后钩子静默失效） */
  readyCommandId?: number | null;
  /** passwd 改密提示词配置（严格匹配用）；null 或缺省 = 未配置（用内置默认模式检测） */
  passwdPrompts?: PasswdPrompts | null;
  /** 环境级别：dev 开发 / test 测试 / staging 预发 / prod 生产；缺省为 'dev' */
  envLevel?: HostEnvironmentLevel;
  /** 是否开启危险命令拦截防护罩；缺省为 true（仅在 staging/prod 触发） */
  safetyGuardEnabled?: boolean;
  /** 所属分组（多选）；空数组 = 未分组 */
  groupIds: number[];
  createdAt: number;
  updatedAt: number;
}

export interface HostStoreData {
  version: 2;
  groups: GroupNode[];
  hosts: HostNode[];
}

/** v1 旧结构（文件夹与主机同树、字符串 UUID 主键），仅用于迁移读取 */
export interface LegacyHostStoreData {
  version: 1;
  nodes: Array<
    | { kind: 'folder'; id: string; parentId: string | null; name: string; createdAt: number; updatedAt: number }
    | (Omit<HostNode, 'groupIds'> & { kind: 'host'; id: string; parentId: string | null })
  >;
}

export type HostInput = Omit<HostNode, 'id' | 'createdAt' | 'updatedAt'>;
export type GroupInput = Pick<GroupNode, 'name' | 'parentId'>;
