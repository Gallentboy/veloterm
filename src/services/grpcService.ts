/**
 * gRPC 调试服务：一元调用（服务器反射取 schema，JSON 出入参）。
 * TLS 走配置库（校验域名为唯一 key）：调用只按 id 引用，CA/客户端证书/私钥
 * 由后端解析、不随请求回传。认证支持 Bearer token 与任意自定义 metadata。
 */

import { invoke } from '@tauri-apps/api/core';

export interface GrpcHeaderPair {
  name: string;
  value: string;
}

export interface GrpcCallRequest {
  /** 目标地址 host:port */
  target: string;
  /** TLS 配置库引用（grpc_tls_profiles.id）；null = 明文 h2c */
  tlsProfileId: number | null;
  /** 服务全名，如 com.example.UserService */
  service: string;
  /** 方法名，如 GetUser */
  method: string;
  /** JSON 入参；空串 = 空消息 */
  jsonInput: string;
  /** Bearer token → authorization: Bearer <token> */
  bearerToken?: string | null;
  /** 附加自定义 metadata */
  headers?: GrpcHeaderPair[];
  /** 整体超时毫秒；缺省 15s */
  timeoutMs?: number | null;
}

export interface GrpcCallResponse {
  /** JSON 出参（proto JSON 映射：字段名 camelCase、int64 为字符串） */
  jsonOutput: string;
}

export function grpcCall(request: GrpcCallRequest): Promise<GrpcCallResponse> {
  return invoke('grpc_call', { request });
}

// ── TLS 配置库（校验域名为唯一 key）──

export interface GrpcTlsProfile {
  id: number;
  /** 校验域名（SNI / 证书校验），唯一标识 */
  serverName: string;
  /** 信任 CA（PEM）；null = 系统根证书 */
  caPem?: string | null;
  /** mTLS 客户端证书（PEM）；与 clientKeyPem 成对 */
  clientCertPem?: string | null;
  /** mTLS 客户端私钥（PEM）；与 clientCertPem 成对 */
  clientKeyPem?: string | null;
  createdAt: number;
  updatedAt: number;
}

export function grpcTlsList(): Promise<GrpcTlsProfile[]> {
  return invoke('grpc_tls_list');
}

export function grpcTlsSave(record: GrpcTlsProfile): Promise<GrpcTlsProfile> {
  return invoke('grpc_tls_save', { record });
}

export function grpcTlsDelete(id: number): Promise<void> {
  return invoke('grpc_tls_delete', { id });
}

/** gRPC 调用种子：ZK 节点数据识别出服务地址后，一键带去 gRPC 页新 tab 自动填充。 */
export interface GrpcCallSeed {
  /** host:port */
  target: string;
  /** 服务全名（URL path 段） */
  service: string;
  /** 原文是 https:// 则为 true（提示 TLS 可能需要开启） */
  tls: boolean;
}

/** 识别 gRPC 服务地址：`https?://host:port/服务全名?...` 或裸 `host:port/服务全名`。
 *  服务段必须含点（至少一层包名）才算命中，避免误报普通 URL/路径。 */
export function parseGrpcServiceUrl(text: string): GrpcCallSeed | null {
  const trimmed = text.trim();
  const match = trimmed.match(
    /^(https?:\/\/)?([A-Za-z0-9._-]+):(\d{1,5})\/([A-Za-z0-9_.$-]+?)(?:[/?#].*)?$/,
  );
  if (!match) return null;
  const service = match[4] ?? '';
  if (!service.includes('.')) return null;
  return {
    target: `${match[2]}:${match[3]}`,
    service,
    tls: (match[1] ?? '').toLowerCase().startsWith('https'),
  };
}
