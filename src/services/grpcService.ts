/**
 * gRPC 调试服务：一元调用（服务器反射取 schema，JSON 出入参）。
 * TLS 支持系统根/自定义 CA、mTLS 客户端证书、SNI 域名覆盖；
 * 认证支持 Bearer token 与任意自定义 metadata。
 */

import { invoke } from '@tauri-apps/api/core';

export interface GrpcTlsConfig {
  /** 信任的自定义 CA（PEM 文本）；null = 系统根证书 */
  caPem?: string | null;
  /** SNI / 证书校验域名；null = 取 target 的 host 部分 */
  serverName?: string | null;
  /** mTLS 客户端证书（PEM）；与 clientKeyPem 成对 */
  clientCertPem?: string | null;
  /** mTLS 客户端私钥（PEM，PKCS8/RSA/EC）；与 clientCertPem 成对 */
  clientKeyPem?: string | null;
}

export interface GrpcHeaderPair {
  name: string;
  value: string;
}

export interface GrpcCallRequest {
  /** 目标地址 host:port */
  target: string;
  /** null = 明文 h2c */
  tls: GrpcTlsConfig | null;
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
