//! gRPC 一元调用客户端（tonic + prost-reflect，无代码生成）。
//!
//! 前端形态：`invoke('grpc_call', { request })` → `{ jsonOutput }`。
//! schema 经**服务器反射**获取（grpc.reflection.v1alpha 与 v1 双探测，消息线格式
//! 同构、仅服务全名不同），JSON ↔ protobuf 用 prost-reflect 动态消息编解码——
//! 无需本地 proto 文件，但要求服务端注册了 reflection 服务（grpcurl 同款机制）。
//! TLS：系统根证书或自定义 CA PEM（自签场景）；认证：Bearer token + 任意自定义
//! metadata。反射协议的六个消息手写 prost 结构体（tonic 无 protoc 的代码生成
//! 路径，build 期零外部工具依赖）。

use std::collections::{HashSet, VecDeque};
use std::time::Duration;

use futures::StreamExt;
use prost::Message as _;
use serde::{Deserialize, Serialize};

// ── 反射协议消息（grpc/reflection/v1alpha/server_reflection.proto 手写；
//    v1 与 v1alpha 消息线格式同构，共用一套结构体，仅服务全名不同）──

mod reflection {
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct ServerReflectionRequest {
        #[prost(string, tag = "1")]
        pub host: String,
        #[prost(oneof = "server_reflection_request::MessageRequest", tags = "4, 3, 7")]
        pub message_request: Option<server_reflection_request::MessageRequest>,
    }

    pub mod server_reflection_request {
        #[derive(Clone, PartialEq, prost::Oneof)]
        pub enum MessageRequest {
            #[prost(string, tag = "4")]
            FileContainingSymbol(String),
            #[prost(string, tag = "3")]
            FileByFilename(String),
            #[prost(string, tag = "7")]
            ListServices(String),
        }
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct ServerReflectionResponse {
        #[prost(string, tag = "1")]
        pub valid_host: String,
        #[prost(oneof = "server_reflection_response::MessageResponse", tags = "4, 7, 6")]
        pub message_response: Option<server_reflection_response::MessageResponse>,
    }

    pub mod server_reflection_response {
        /// 变体名与 proto 官方定义一致（同 Response 后缀是上游命名，非本仓风格）
        #[allow(clippy::enum_variant_names)]
        #[derive(Clone, PartialEq, prost::Oneof)]
        pub enum MessageResponse {
            #[prost(message, tag = "4")]
            FileDescriptorResponse(super::FileDescriptorResponse),
            #[prost(message, tag = "7")]
            ErrorResponse(super::ErrorResponse),
            #[prost(message, tag = "6")]
            ListServiceResponse(super::ListServiceResponse),
        }
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct FileDescriptorResponse {
        #[prost(bytes = "vec", repeated, tag = "1")]
        pub file_descriptor_proto: Vec<Vec<u8>>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct ErrorResponse {
        #[prost(int32, tag = "1")]
        pub error_code: i32,
        #[prost(string, tag = "2")]
        pub error_message: String,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct ListServiceResponse {
        #[prost(message, repeated, tag = "1")]
        pub service: Vec<ServiceResponse>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct ServiceResponse {
        #[prost(string, tag = "1")]
        pub name: String,
    }
}

use reflection::server_reflection_request::MessageRequest;
use reflection::server_reflection_response::MessageResponse;
use reflection::{ServerReflectionRequest, ServerReflectionResponse};

/// 反射服务的两个版本全名：v1alpha 部署面最广，v1 是现行标准；逐个探测。
const REFLECTION_SERVICES: [&str; 2] = [
    "grpc.reflection.v1alpha.ServerReflection",
    "grpc.reflection.v1.ServerReflection",
];

// ── 对前端的请求/响应结构 ──

/// 单次 gRPC 一元调用请求（前端 invoke('grpc_call', { request })）。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrpcCallRequest {
    /// 目标地址 host:port
    pub target: String,
    /// TLS 配置；None = 明文 h2c
    #[serde(default)]
    pub tls: Option<GrpcTlsConfig>,
    /// 服务全名，如 com.example.UserService
    pub service: String,
    /// 方法名，如 GetUser
    pub method: String,
    /// JSON 入参（缺省/空白视为 "{}" 空消息）
    #[serde(default)]
    pub json_input: String,
    /// Bearer token → authorization: Bearer <token>
    #[serde(default)]
    pub bearer_token: Option<String>,
    /// 附加自定义 metadata
    #[serde(default)]
    pub headers: Vec<GrpcHeader>,
    /// 整体（连接+反射+调用）超时毫秒；缺省 15s
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrpcTlsConfig {
    /// 信任的自定义 CA（PEM 文本）；None = 系统根证书
    #[serde(default)]
    pub ca_pem: Option<String>,
    /// SNI / 证书校验域名；缺省取 target 的 host 部分
    #[serde(default)]
    pub server_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrpcHeader {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrpcCallResponse {
    /// JSON 出参（proto JSON 映射：字段名 camelCase、int64 为字符串）
    pub json_output: String,
}

#[tauri::command]
pub async fn grpc_call(request: GrpcCallRequest) -> Result<GrpcCallResponse, String> {
    let timeout = Duration::from_millis(request.timeout_ms.unwrap_or(15_000));
    tokio::time::timeout(timeout, call_grpc_unary(request))
        .await
        .map_err(|_| format!("gRPC 调用超时（超过 {}ms）", timeout.as_millis()))?
}

// ── 动态消息 Codec（tonic 的 ProstCodec 走 U::default()+merge 路径，
//    DynamicMessage 的空默认实例没有 descriptor、无法 merge，故自写）──

#[derive(Clone)]
struct DynamicCodec {
    output: prost_reflect::MessageDescriptor,
}

struct DynamicEncoder;

impl tonic::codec::Encoder for DynamicEncoder {
    type Item = prost_reflect::DynamicMessage;
    type Error = tonic::Status;

    fn encode(
        &mut self,
        item: Self::Item,
        dst: &mut tonic::codec::EncodeBuf<'_>,
    ) -> Result<(), Self::Error> {
        item.encode(dst)
            .map_err(|e| tonic::Status::internal(format!("protobuf 编码失败: {e}")))
    }
}

struct DynamicDecoder {
    output: prost_reflect::MessageDescriptor,
}

impl tonic::codec::Decoder for DynamicDecoder {
    type Item = prost_reflect::DynamicMessage;
    type Error = tonic::Status;

    fn decode(
        &mut self,
        buf: &mut tonic::codec::DecodeBuf<'_>,
    ) -> Result<Option<Self::Item>, Self::Error> {
        prost_reflect::DynamicMessage::decode(self.output.clone(), buf)
            .map(Some)
            .map_err(|e| tonic::Status::internal(format!("protobuf 解码失败: {e}")))
    }
}

impl tonic::codec::Codec for DynamicCodec {
    type Encode = prost_reflect::DynamicMessage;
    type Decode = prost_reflect::DynamicMessage;
    type Encoder = DynamicEncoder;
    type Decoder = DynamicDecoder;

    fn encoder(&mut self) -> Self::Encoder {
        DynamicEncoder
    }

    fn decoder(&mut self) -> Self::Decoder {
        DynamicDecoder {
            output: self.output.clone(),
        }
    }
}

// ── 主流程 ──

type GrpcClient = tonic::client::Grpc<tonic::transport::Channel>;

/// 进程级 rustls CryptoProvider 只装一次（aws-lc，与 quinn/russh 同源）。
/// reqwest 生态同时 union 了 ring+aws-lc 双 feature，rustls 无法自动选默认，
/// 而 tonic 的 TLS 走 builder()（依赖进程默认）——首次 TLS 使用前显式安装。
fn ensure_tls_provider() {
    use std::sync::Once;
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    });
}

async fn call_grpc_unary(req: GrpcCallRequest) -> Result<GrpcCallResponse, String> {
    let target = req.target.trim().to_string();
    if target.is_empty() {
        return Err("target 不能为空（host:port）".into());
    }
    let (service, method) = (req.service.trim(), req.method.trim());
    if service.is_empty() || method.is_empty() {
        return Err("服务名与方法名均不能为空".into());
    }
    let timeout = Duration::from_millis(req.timeout_ms.unwrap_or(15_000));

    // 1. 连接（明文或 TLS）
    let scheme = if req.tls.is_some() { "https" } else { "http" };
    let uri: http::Uri = format!("{scheme}://{target}/")
        .parse()
        .map_err(|e| format!("target 非法（{target}）: {e}"))?;
    let mut endpoint = tonic::transport::Channel::builder(uri);
    if let Some(tls) = &req.tls {
        ensure_tls_provider();
        let mut cfg = tonic::transport::ClientTlsConfig::new();
        match &tls.ca_pem {
            Some(pem) => {
                cfg = cfg.ca_certificate(tonic::transport::Certificate::from_pem(pem));
            }
            None => cfg = cfg.with_native_roots(),
        }
        if let Some(name) = tls.server_name.as_deref().filter(|n| !n.trim().is_empty()) {
            cfg = cfg.domain_name(name.trim());
        }
        endpoint = endpoint
            .tls_config(cfg)
            .map_err(|e| format!("TLS 配置失败: {e}"))?;
    }
    let channel = endpoint
        .connect()
        .await
        .map_err(|e| format!("连接 {target} 失败: {e}"))?;
    let mut grpc = tonic::client::Grpc::new(channel);
    // Channel 内部是 tower Buffer：必须先 ready() 才能 call（codegen 生成的客户端
    // 同样会先 await ready；跳过会触发 Buffer 的 send_item/poll_reserve 断言 panic）
    grpc.ready()
        .await
        .map_err(|e| format!("连接就绪失败（{target}）: {e}"))?;

    // 2. 反射取 schema（以服务全名为 symbol），构建 DescriptorPool
    let pool = reflect_descriptor_pool(&mut grpc, service).await?;
    let svc = pool
        .get_service_by_name(service)
        .ok_or_else(|| format!("反射结果中找不到服务 {service}"))?;
    let method_desc = svc.methods().find(|m| m.name() == method).ok_or_else(|| {
        let names: Vec<String> = svc.methods().map(|m| m.name().to_string()).collect();
        format!(
            "服务 {service} 没有方法 {method}；可用方法：{}",
            names.join(", ")
        )
    })?;

    // 3. JSON 入参 → 动态消息（proto JSON 语义：字段 camelCase、int64 字符串）
    let json_input = if req.json_input.trim().is_empty() {
        "{}"
    } else {
        req.json_input.trim()
    };
    let mut de = serde_json::Deserializer::from_str(json_input);
    let input: prost_reflect::DynamicMessage =
        prost_reflect::DynamicMessage::deserialize(method_desc.input(), &mut de)
            .map_err(|e| format!("入参 JSON 与 {service}.{method} 的入参类型不符: {e}"))?;

    // 4. 一元调用 /{service}/{method}
    let path: http::uri::PathAndQuery = format!("/{service}/{method}")
        .parse()
        .map_err(|e| format!("路径构造失败: {e}"))?;
    let codec = DynamicCodec {
        output: method_desc.output(),
    };
    let mut request = tonic::Request::new(input);
    request.set_timeout(timeout);
    if let Some(token) = req.bearer_token.as_deref().filter(|t| !t.trim().is_empty()) {
        let value: tonic::metadata::MetadataValue<_> = format!("Bearer {}", token.trim())
            .parse()
            .map_err(|_| "Bearer token 含非法字符".to_string())?;
        request.metadata_mut().insert("authorization", value);
    }
    for h in &req.headers {
        let name: tonic::metadata::MetadataKey<_> = h
            .name
            .parse()
            .map_err(|_| format!("metadata 名非法: {}", h.name))?;
        let value: tonic::metadata::MetadataValue<_> = h
            .value
            .parse()
            .map_err(|_| format!("metadata 值含非法字符（不支持二进制 header）: {}", h.value))?;
        request.metadata_mut().insert(name, value);
    }
    // ready() 的 slot 每次调用消费一个（前一个已被反射 streaming 用掉），unary 前再 reserve
    grpc.ready()
        .await
        .map_err(|e| format!("连接就绪失败（{target}）: {e}"))?;
    let response = grpc.unary(request, path, codec).await.map_err(|s| {
        format!(
            "gRPC 调用失败 [{}]: {}",
            s.code(),
            if s.message().is_empty() { "(无详情)" } else { s.message() }
        )
    })?;

    // 5. 动态消息 → JSON 出参
    let json_output = serde_json::to_string(&response.into_inner())
        .map_err(|e| format!("出参序列化 JSON 失败: {e}"))?;
    Ok(GrpcCallResponse { json_output })
}

/// 反射错误：Unsupported = 该版本反射服务不存在（可试另一版本）。
enum ReflectError {
    Unsupported(String),
    Other(String),
}

/// 经反射拉取 symbol（服务全名）所在文件及其依赖闭包，构建 DescriptorPool。
/// 先 v1alpha 后 v1 逐个探测；某版本反射调通但返回业务错误时直接上报，
/// 不再尝试另一版本（避免把「服务不存在」误报成「不支持反射」）。
async fn reflect_descriptor_pool(
    grpc: &mut GrpcClient,
    symbol: &str,
) -> Result<prost_reflect::DescriptorPool, String> {
    let mut unsupported: Vec<String> = Vec::new();
    for svc_full in REFLECTION_SERVICES {
        match reflect_via(grpc, svc_full, symbol).await {
            Ok(pool) => return Ok(pool),
            Err(ReflectError::Unsupported(m)) => unsupported.push(m),
            Err(ReflectError::Other(m)) => return Err(m),
        }
    }
    Err(format!(
        "服务器不支持 gRPC 反射（{}），无法在无 proto 文件的情况下定位 {symbol} 的消息定义",
        unsupported.join("; ")
    ))
}

/// 单版本反射：file_containing_symbol 拿主文件，再按 dependency 逐个
/// file_by_filename 补齐闭包（部分实现只返回主文件），上限 100 轮防环。
async fn reflect_via(
    grpc: &mut GrpcClient,
    svc_full: &str,
    symbol: &str,
) -> Result<prost_reflect::DescriptorPool, ReflectError> {
    let mut fdps: Vec<prost_types::FileDescriptorProto> = Vec::new();
    let mut have: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<MessageRequest> = VecDeque::new();
    queue.push_back(MessageRequest::FileContainingSymbol(symbol.to_string()));
    let mut rounds = 0usize;
    while let Some(message_request) = queue.pop_front() {
        rounds += 1;
        if rounds > 100 {
            return Err(ReflectError::Other("反射文件拉取超过 100 轮，疑似依赖环".into()));
        }
        let resp = send_reflection_request(grpc, svc_full, message_request).await?;
        let fds = match resp.message_response {
            Some(MessageResponse::FileDescriptorResponse(fds)) => fds,
            Some(MessageResponse::ErrorResponse(err)) => {
                return Err(ReflectError::Other(format!(
                    "反射错误 {}: {}",
                    err.error_code, err.error_message
                )));
            }
            _ => return Err(ReflectError::Other("反射返回了非预期的消息类型".into())),
        };
        for raw in fds.file_descriptor_proto {
            let fdp = prost_types::FileDescriptorProto::decode(raw.as_slice())
                .map_err(|e| {
                    ReflectError::Other(format!("反射返回的 FileDescriptorProto 损坏: {e}"))
                })?;
            let fdp_name = fdp.name.clone().unwrap_or_default();
            for dep in &fdp.dependency {
                if !have.contains(dep) {
                    queue.push_back(MessageRequest::FileByFilename(dep.clone()));
                }
            }
            if have.insert(fdp_name) {
                fdps.push(fdp);
            }
        }
    }
    let set = prost_types::FileDescriptorSet { file: fdps };
    let set_bytes = set.encode_to_vec();
    prost_reflect::DescriptorPool::decode(set_bytes.as_slice())
        .map_err(|e| ReflectError::Other(format!("反射文件集无法构建 DescriptorPool: {e}")))
}

/// 发一条反射请求（ServerReflectionInfo 是双向流，客户端只发一条、收第一条有效响应）。
async fn send_reflection_request(
    grpc: &mut GrpcClient,
    svc_full: &str,
    message_request: MessageRequest,
) -> Result<ServerReflectionResponse, ReflectError> {
    let path: http::uri::PathAndQuery = format!("/{svc_full}/ServerReflectionInfo")
        .parse()
        .map_err(|e| ReflectError::Other(format!("反射路径构造失败: {e}")))?;
    let msg = ServerReflectionRequest {
        host: String::new(),
        message_request: Some(message_request),
    };
    let stream = futures::stream::once(async move { msg });
    let codec =
        tonic::codec::ProstCodec::<ServerReflectionRequest, ServerReflectionResponse>::default();
    grpc.ready()
        .await
        .map_err(|e| ReflectError::Other(format!("连接就绪失败: {e}")))?;
    let response = grpc
        .streaming(tonic::Request::new(stream), path, codec)
        .await
        .map_err(|s| {
            if s.code() == tonic::Code::Unimplemented {
                ReflectError::Unsupported(format!("{svc_full}: {}", s.message()))
            } else {
                ReflectError::Other(format!(
                    "反射服务调用失败 [{}]: {}",
                    s.code(),
                    s.message()
                ))
            }
        })?;
    let mut inbound = response.into_inner();
    while let Some(item) = inbound.next().await {
        let resp = item.map_err(|s| {
            ReflectError::Other(format!("反射流中断 [{}]: {}", s.code(), s.message()))
        })?;
        if let Some(MessageResponse::ErrorResponse(err)) = &resp.message_response {
            return Err(ReflectError::Other(format!(
                "反射错误 {}: {}",
                err.error_code, err.error_message
            )));
        }
        if resp.message_response.is_some() {
            return Ok(resp);
        }
    }
    Err(ReflectError::Other("反射流结束但未收到响应".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::convert::Infallible;
    use std::pin::Pin;
    use std::sync::{Arc, Mutex};
    use std::task::{Context, Poll};

    use http_body_util::BodyExt as _;
    use tonic::codegen::Service;
    use tonic::server::NamedService;
    use tonic::transport::{Identity, ServerTlsConfig};

    // ── 测试业务协议：veloterm.test.UserService/GetUser ──

    #[derive(Clone, PartialEq, prost::Message)]
    struct GetUserRequest {
        #[prost(int64, tag = "1")]
        id: i64,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    struct GetUserResponse {
        #[prost(string, tag = "1")]
        name: String,
        #[prost(int32, tag = "2")]
        age: i32,
    }

    const TEST_SERVICE: &str = "veloterm.test.UserService";

    fn test_fdp() -> prost_types::FileDescriptorProto {
        use prost_types as p;
        let field = |name: &str, number: i32,
                     ty: p::field_descriptor_proto::Type|
         p::FieldDescriptorProto {
            name: Some(name.to_string()),
            number: Some(number),
            label: Some(p::field_descriptor_proto::Label::Optional as i32),
            r#type: Some(ty as i32),
            json_name: Some(name.to_string()),
            ..Default::default()
        };
        p::FileDescriptorProto {
            name: Some("veloterm_test.proto".to_string()),
            package: Some("veloterm.test".to_string()),
            syntax: Some("proto3".to_string()),
            message_type: vec![
                p::DescriptorProto {
                    name: Some("GetUserRequest".to_string()),
                    field: vec![field(
                        "id",
                        1,
                        p::field_descriptor_proto::Type::Int64,
                    )],
                    ..Default::default()
                },
                p::DescriptorProto {
                    name: Some("GetUserResponse".to_string()),
                    field: vec![
                        field("name", 1, p::field_descriptor_proto::Type::String),
                        field("age", 2, p::field_descriptor_proto::Type::Int32),
                    ],
                    ..Default::default()
                },
            ],
            service: vec![p::ServiceDescriptorProto {
                name: Some("UserService".to_string()),
                method: vec![p::MethodDescriptorProto {
                    name: Some("GetUser".to_string()),
                    input_type: Some(".veloterm.test.GetUserRequest".to_string()),
                    output_type: Some(".veloterm.test.GetUserResponse".to_string()),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    fn test_fdp_bytes() -> Vec<u8> {
        test_fdp().encode_to_vec()
    }

    // ── 手写 gRPC 服务端基元（帧 + 响应构造）──

    /// 从请求体解出单条 grpc 消息（5 字节帧头 + payload）。
    async fn read_grpc_frame(body: tonic::body::Body) -> Result<Vec<u8>, String> {
        let collected = http_body_util::BodyExt::collect(body)
            .await
            .map_err(|e| format!("读请求体失败: {e}"))?
            .to_bytes();
        let header = collected.get(..5).ok_or("请求体不足 5 字节帧头")?;
        let len_bytes: [u8; 4] = header
            .get(1..5)
            .and_then(|s| s.try_into().ok())
            .ok_or("帧头损坏")?;
        let len = u32::from_be_bytes(len_bytes) as usize;
        collected
            .get(5..5 + len)
            .map(|s| s.to_vec())
            .ok_or_else(|| format!("帧长 {len} 超出实际数据"))
    }

    fn grpc_frame(payload: &[u8]) -> bytes::Bytes {
        let mut frame = Vec::with_capacity(5 + payload.len());
        frame.push(0u8);
        frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        frame.extend_from_slice(payload);
        bytes::Bytes::from(frame)
    }

    // ── 业务服务（校验 Bearer，按 id 回固定用户）──

    #[derive(Clone)]
    struct TestUserService {
        expect_authorization: Option<String>,
        seen_authorization: Arc<Mutex<Option<String>>>,
    }

    impl NamedService for TestUserService {
        const NAME: &'static str = "veloterm.test.UserService";
    }

    impl Service<http::Request<tonic::body::Body>> for TestUserService {
        type Response = http::Response<tonic::body::Body>;
        type Error = Infallible;
        type Future =
            Pin<Box<dyn std::future::Future<Output = Result<Self::Response, Infallible>> + Send>>;

        fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Infallible>> {
            Poll::Ready(Ok(()))
        }

        fn call(&mut self, req: http::Request<tonic::body::Body>) -> Self::Future {
            let expect = self.expect_authorization.clone();
            let seen = self.seen_authorization.clone();
            let path = req.uri().path().to_string();
            let authorization = req
                .headers()
                .get("authorization")
                .and_then(|v| v.to_str().ok())
                .map(str::to_string);
            let body = req.into_body();
            Box::pin(async move {
                if path != "/veloterm.test.UserService/GetUser" {
                    return Ok(grpc_trailers_response(12, "Method not found in test server"));
                }
                *seen.lock().unwrap() = authorization.clone();
                if let Some(expect) = &expect
                    && authorization.as_deref() != Some(expect.as_str())
                {
                    return Ok(grpc_trailers_response(7, "PermissionDenied: bad token"));
                }
                let payload = match read_grpc_frame(body).await {
                    Ok(p) => p,
                    Err(e) => return Ok(grpc_trailers_response(13, &format!("bad frame: {e}"))),
                };
                let request = match GetUserRequest::decode(payload.as_slice()) {
                    Ok(r) => r,
                    Err(e) => return Ok(grpc_trailers_response(13, &format!("bad payload: {e}"))),
                };
                let response = if request.id == 1 {
                    GetUserResponse { name: "Alice".to_string(), age: 30 }
                } else {
                    GetUserResponse { name: format!("user-{}", request.id), age: 20 }
                };
                Ok(grpc_ok_response(&response.encode_to_vec()))
            })
        }
    }

    // ── 反射服务（const 泛型切换 v1alpha / v1）──

    #[derive(Clone)]
    struct TestReflectionSvc<const V1: bool>;

    impl NamedService for TestReflectionSvc<false> {
        const NAME: &'static str = "grpc.reflection.v1alpha.ServerReflection";
    }

    impl NamedService for TestReflectionSvc<true> {
        const NAME: &'static str = "grpc.reflection.v1.ServerReflection";
    }

    impl<const V1: bool> Service<http::Request<tonic::body::Body>> for TestReflectionSvc<V1> {
        type Response = http::Response<tonic::body::Body>;
        type Error = Infallible;
        type Future =
            Pin<Box<dyn std::future::Future<Output = Result<Self::Response, Infallible>> + Send>>;

        fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Infallible>> {
            Poll::Ready(Ok(()))
        }

        fn call(&mut self, req: http::Request<tonic::body::Body>) -> Self::Future {
            let body = req.into_body();
            Box::pin(async move {
                let payload = match read_grpc_frame(body).await {
                    Ok(p) => p,
                    Err(e) => return Ok(grpc_trailers_response(13, &format!("bad frame: {e}"))),
                };
                let request = match ServerReflectionRequest::decode(payload.as_slice()) {
                    Ok(r) => r,
                    Err(e) => return Ok(grpc_trailers_response(13, &format!("bad payload: {e}"))),
                };
                let message_response = match request.message_request {
                    Some(MessageRequest::FileContainingSymbol(sym)) if sym == TEST_SERVICE => {
                        MessageResponse::FileDescriptorResponse(reflection::FileDescriptorResponse {
                            file_descriptor_proto: vec![test_fdp_bytes()],
                        })
                    }
                    Some(MessageRequest::FileByFilename(name)) if name == "veloterm_test.proto" => {
                        MessageResponse::FileDescriptorResponse(reflection::FileDescriptorResponse {
                            file_descriptor_proto: vec![test_fdp_bytes()],
                        })
                    }
                    Some(MessageRequest::FileContainingSymbol(sym)) => {
                        return Ok(grpc_trailers_response(5, &format!("symbol not found: {sym}")));
                    }
                    _ => {
                        return Ok(grpc_trailers_response(12, "unsupported reflection request"));
                    }
                };
                let resp = ServerReflectionResponse {
                    valid_host: String::new(),
                    message_response: Some(message_response),
                };
                Ok(grpc_ok_response(&resp.encode_to_vec()))
            })
        }
    }

    // ── 响应构造 ──

    /// 200 + grpc 响应帧 + trailers（grpc-status；非 0 带 grpc-message）。
    fn grpc_ok_response(payload: &[u8]) -> http::Response<tonic::body::Body> {
        build_grpc_response(Some(payload), 0, "")
    }

    fn grpc_trailers_response(status: u16, message: &str) -> http::Response<tonic::body::Body> {
        build_grpc_response(None, status, message)
    }

    fn build_grpc_response(
        payload: Option<&[u8]>,
        status: u16,
        message: &str,
    ) -> http::Response<tonic::body::Body> {
        let mut trailers = http::HeaderMap::new();
        if let Ok(v) = http::HeaderValue::from_str(&status.to_string()) {
            trailers.insert("grpc-status", v);
        }
        if !message.is_empty()
            && let Ok(v) = http::HeaderValue::from_str(&percent_encode_grpc_message(message))
        {
            trailers.insert("grpc-message", v);
        }
        let data = match payload {
            Some(p) => grpc_frame(p),
            None => bytes::Bytes::new(),
        };
        let body = http_body_util::Full::new(data)
            .with_trailers(futures::future::ready(Some(Ok::<_, Infallible>(trailers))));
        http::Response::builder()
            .status(200)
            .header("content-type", "application/grpc")
            .body(tonic::body::Body::new(body))
            .unwrap_or_else(|_| unreachable!("静态构造的响应总是合法"))
    }

    /// grpc-message 要求 percent-encoding（非 ASCII/保留字符）。
    fn percent_encode_grpc_message(msg: &str) -> String {
        let mut out = String::with_capacity(msg.len());
        for b in msg.bytes() {
            if b.is_ascii_graphic() && b != b'%' {
                out.push(b as char);
            } else {
                out.push_str(&format!("%{b:02X}"));
            }
        }
        out
    }

    // ── 服务器启动 helper ──

    enum ReflectionVersion {
        None,
        V1Alpha,
        V1,
    }

    struct TestServer {
        target: String,
        ca_pem: Option<String>,
        seen_authorization: Arc<Mutex<Option<String>>>,
    }

    async fn spawn_test_server(
        reflection: ReflectionVersion,
        tls: bool,
        expect_authorization: Option<String>,
    ) -> TestServer {
        let seen = Arc::new(Mutex::new(None));
        let biz = TestUserService {
            expect_authorization,
            seen_authorization: seen.clone(),
        };
        let mut routes = tonic::service::Routes::default().add_service(biz);
        match reflection {
            ReflectionVersion::V1Alpha => {
                routes = routes.add_service(TestReflectionSvc::<false>);
            }
            ReflectionVersion::V1 => {
                routes = routes.add_service(TestReflectionSvc::<true>);
            }
            ReflectionVersion::None => {}
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut ca_pem = None;
        let incoming = futures::stream::unfold(listener, |l| async move {
            let accepted = l.accept().await.map(|(s, _)| s);
            Some((accepted, l))
        });
        let mut server = tonic::transport::Server::builder();
        if tls {
            ensure_tls_provider();
            let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
            let cert_pem = cert.cert.pem();
            let key_pem = cert.key_pair.serialize_pem();
            ca_pem = Some(cert_pem.clone());
            let identity = Identity::from_pem(cert_pem, key_pem);
            server = server
                .tls_config(ServerTlsConfig::new().identity(identity))
                .unwrap();
        }
        tokio::spawn(async move {
            let _ = server.serve_with_incoming(routes, incoming).await;
        });
        TestServer {
            target: format!("127.0.0.1:{}", addr.port()),
            ca_pem,
            seen_authorization: seen,
        }
    }

    fn base_request(target: &str) -> GrpcCallRequest {
        GrpcCallRequest {
            target: target.to_string(),
            tls: None,
            service: "veloterm.test.UserService".to_string(),
            method: "GetUser".to_string(),
            json_input: r#"{"id": 1}"#.to_string(),
            bearer_token: None,
            headers: vec![],
            timeout_ms: Some(5_000),
        }
    }

    // ── 用例 ──

    #[tokio::test]
    async fn grpc_call_plaintext_roundtrip_with_bearer() {
        let server = spawn_test_server(
            ReflectionVersion::V1Alpha,
            false,
            Some("Bearer tok-123".to_string()),
        )
        .await;
        let mut req = base_request(&server.target);
        req.bearer_token = Some("tok-123".to_string());
        let resp = grpc_call(req).await.expect("明文回环调用应成功");
        let value: serde_json::Value = serde_json::from_str(&resp.json_output).unwrap();
        assert_eq!(
            value,
            serde_json::json!({"name": "Alice", "age": 30}),
            "出参 JSON 应与固定响应一致"
        );
        assert_eq!(
            server.seen_authorization.lock().unwrap().as_deref(),
            Some("Bearer tok-123"),
            "Bearer token 应作为 authorization metadata 到达服务端"
        );
    }

    #[tokio::test]
    async fn grpc_call_rejects_wrong_token() {
        let server = spawn_test_server(
            ReflectionVersion::V1Alpha,
            false,
            Some("Bearer tok-123".to_string()),
        )
        .await;
        let mut req = base_request(&server.target);
        req.bearer_token = Some("wrong".to_string());
        let err = grpc_call(req).await.expect_err("错误 token 应被拒绝");
        assert!(err.contains("PermissionDenied"), "错误信息应为权限拒绝: {err}");
    }

    #[tokio::test]
    async fn grpc_call_tls_with_self_signed_ca() {
        let server = spawn_test_server(ReflectionVersion::V1Alpha, true, None).await;
        let mut req = base_request(&server.target);
        req.tls = Some(GrpcTlsConfig {
            ca_pem: server.ca_pem.clone(),
            server_name: Some("localhost".to_string()),
        });
        let resp = grpc_call(req).await.expect("自定义 CA 的 TLS 调用应成功");
        assert!(resp.json_output.contains("Alice"));
    }

    #[tokio::test]
    async fn grpc_call_tls_with_wrong_ca_fails() {
        let server = spawn_test_server(ReflectionVersion::V1Alpha, true, None).await;
        let wrong_ca = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
        let mut req = base_request(&server.target);
        req.tls = Some(GrpcTlsConfig {
            ca_pem: Some(wrong_ca.cert.pem()),
            server_name: Some("localhost".to_string()),
        });
        let err = grpc_call(req).await.expect_err("错误 CA 应握手失败");
        assert!(err.contains("连接"), "错误应为连接失败: {err}");
    }

    #[tokio::test]
    async fn grpc_call_reflection_v1_fallback() {
        // 只挂 v1 反射：dual-try 应在 v1alpha UNIMPLEMENTED 后落到 v1 成功
        let server = spawn_test_server(ReflectionVersion::V1, false, None).await;
        let resp = grpc_call(base_request(&server.target))
            .await
            .expect("仅 v1 反射时也应成功");
        assert!(resp.json_output.contains("Alice"));
    }

    #[tokio::test]
    async fn grpc_call_without_reflection_reports_unsupported() {
        let server = spawn_test_server(ReflectionVersion::None, false, None).await;
        let err = grpc_call(base_request(&server.target))
            .await
            .expect_err("无反射服务应报错");
        assert!(
            err.contains("不支持 gRPC 反射"),
            "错误应明确指出不支持反射: {err}"
        );
    }

    #[tokio::test]
    async fn grpc_call_method_not_found_lists_available() {
        let server = spawn_test_server(ReflectionVersion::V1Alpha, false, None).await;
        let mut req = base_request(&server.target);
        req.method = "GetUserX".to_string();
        let err = grpc_call(req).await.expect_err("方法名错误应报错");
        assert!(
            err.contains("可用方法") && err.contains("GetUser"),
            "错误应列出可用方法: {err}"
        );
    }

    #[tokio::test]
    async fn grpc_call_bad_json_input_type_mismatch() {
        let server = spawn_test_server(ReflectionVersion::V1Alpha, false, None).await;
        let mut req = base_request(&server.target);
        req.json_input = r#"{"id": "not-a-number"}"#.to_string();
        let err = grpc_call(req).await.expect_err("入参类型错误应报错");
        assert!(
            err.contains("入参 JSON"),
            "错误应指出入参 JSON 问题: {err}"
        );
    }
}
