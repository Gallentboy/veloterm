//! gRPC 一元调用客户端（tonic + prost-reflect，无代码生成）。
//!
//! 前端形态：`invoke('grpc_call', { request })` → `{ jsonOutput }`。
//! schema 经**服务器反射**获取（grpc.reflection.v1alpha 与 v1 双探测，消息线格式
//! 同构、仅服务全名不同），JSON ↔ protobuf 用 prost-reflect 动态消息编解码——
//! 无需本地 proto 文件，但要求服务端注册了 reflection 服务（grpcurl 同款机制）。
//! TLS：系统根证书或自定义 CA PEM（自签场景）；认证：Bearer token + 任意自定义
//! metadata。反射协议的六个消息手写 prost 结构体（tonic 无 protoc 的代码生成
//! 路径，build 期零外部工具依赖）。

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::OnceLock;
use std::time::Duration;

use futures::StreamExt;
use prost::Message as _;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

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

// ── google.rpc.Status（grpc-status-details-bin 的载荷）：服务端内部异常时
//    grpc-message 常为空，真实详情（异常消息）在此扩展里，尽力解析展示 ──

mod rpc_status {
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct RpcStatus {
        #[prost(int32, tag = "1")]
        pub code: i32,
        #[prost(string, tag = "2")]
        pub message: String,
        #[prost(message, repeated, tag = "3")]
        pub details: Vec<Any>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Any {
        #[prost(string, tag = "1")]
        pub type_url: String,
        #[prost(bytes = "vec", tag = "2")]
        pub value: Vec<u8>,
    }
}


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
    /// TLS 配置库引用（grpc_tls_profiles.id）；None = 明文 h2c。
    /// CA/客户端证书/私钥不随请求回传，命令层按 id 从库解析
    #[serde(default)]
    pub tls_profile_id: Option<usize>,
    /// 服务全名，如 com.example.UserService
    pub service: String,
    /// 方法名，如 GetUser
    pub method: String,
    /// JSON 入参（缺省/空白视为 "{}" 空消息）
    #[serde(default)]
    pub json_input: String,
    /// 入参模式："text" = 纯文本（要求入参消息只有一个 string 字段，自动包装）；
    /// 其他/缺省 = json_input 按 JSON 解析
    #[serde(default)]
    pub input_mode: Option<String>,
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

/// 内部 TLS 材料（由库中的 GrpcTlsProfileRecord 转换，或测试直接构造）：
/// 信任根、客户端身份（mTLS）、SNI 域名全部可选组合。
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
struct TlsMaterial {
    /// 信任的自定义 CA（PEM）；None = 系统根证书
    ca_pem: Option<String>,
    /// SNI / 证书校验域名；缺省取 target 的 host 部分
    server_name: Option<String>,
    /// mTLS 客户端证书（PEM）；与 client_key_pem 成对
    client_cert_pem: Option<String>,
    /// mTLS 客户端私钥（PEM，PKCS8/RSA/EC）；与 client_cert_pem 成对
    client_key_pem: Option<String>,
}

impl From<&crate::db::GrpcTlsProfileRecord> for TlsMaterial {
    fn from(record: &crate::db::GrpcTlsProfileRecord) -> Self {
        TlsMaterial {
            ca_pem: record.ca_pem.clone(),
            server_name: Some(record.server_name.clone()),
            client_cert_pem: record.client_cert_pem.clone(),
            client_key_pem: record.client_key_pem.clone(),
        }
    }
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
    // TLS 只按库存 id 引用（CA/证书/私钥不随请求回传），服务端解析成材料
    let tls = resolve_tls_material(request.tls_profile_id)?;
    call_grpc_unary(request, tls).await
}

/// 按 TLS 配置库 id 解析材料（grpc_call / grpc_list / grpc_describe 共用）。
fn resolve_tls_material(tls_profile_id: Option<usize>) -> Result<Option<TlsMaterial>, String> {
    match tls_profile_id {
        Some(id) => {
            let pool = crate::db::global_pool()
                .ok_or_else(|| "failed to get global db pool".to_string())?;
            let record = crate::db::grpc_tls_profile_by_id(&pool, id)?;
            Ok(Some(TlsMaterial::from(&record)))
        }
        None => Ok(None),
    }
}

/// 出入参的 JSON 结构模板（proto JSON 映射骨架：字段名 camelCase、int64 为字符串、
/// repeated 为 [单项模板]、map 为 {}、枚举为首个值名；自引用链在深度上限处截断为 {}）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrpcMethodSchema {
    pub request_schema: serde_json::Value,
    pub response_schema: serde_json::Value,
}

/// 给定服务与方法，经反射生成入参/出参消息的 JSON 结构模板（前端入参骨架填充 +
/// 出参只读预览）。
#[tauri::command]
pub async fn grpc_method_schema(
    request: GrpcInspectRequest,
    service: String,
    method: String,
) -> Result<GrpcMethodSchema, String> {
    let tls = resolve_tls_material(request.tls_profile_id)?;
    let timeout = Duration::from_millis(request.timeout_ms.unwrap_or(15_000));
    tokio::time::timeout(timeout, grpc_method_schema_inner(request, service, method, tls))
        .await
        .map_err(|_| format!("gRPC schema 获取超时（超过 {}ms）", timeout.as_millis()))?
}

async fn grpc_method_schema_inner(
    request: GrpcInspectRequest,
    service: String,
    method: String,
    tls: Option<TlsMaterial>,
) -> Result<GrpcMethodSchema, String> {
    let target = request.target.trim().to_string();
    if target.is_empty() {
        return Err("target 不能为空（host:port）".into());
    }
    let (service, method) = (service.trim(), method.trim());
    if service.is_empty() || method.is_empty() {
        return Err("服务名与方法名均不能为空".into());
    }
    let mut grpc = open_grpc_client(&target, tls).await?;
    let pool = reflect_descriptor_pool(&mut grpc, service).await?;
    let svc = pool
        .get_service_by_name(service)
        .ok_or_else(|| format!("反射结果中找不到服务 {service}"))?;
    let method_desc = svc.methods().find(|m| m.name() == method).ok_or_else(|| {
        format!("服务 {service} 没有方法 {method}")
    })?;
    Ok(GrpcMethodSchema {
        request_schema: message_json_template(&method_desc.input(), 0),
        response_schema: message_json_template(&method_desc.output(), 0),
    })
}

/// 消息 → JSON 骨架。深度上限拦自引用链（A.b: B, B.a: A），截断处给空对象。
fn message_json_template(desc: &prost_reflect::MessageDescriptor, depth: usize) -> serde_json::Value {
    const MAX_DEPTH: usize = 12;
    if depth > MAX_DEPTH {
        return serde_json::Value::Object(serde_json::Map::new());
    }
    let mut map = serde_json::Map::new();
    for field in desc.fields() {
        map.insert(field.json_name().to_string(), field_json_template(&field, depth));
    }
    serde_json::Value::Object(map)
}

fn field_json_template(field: &prost_reflect::FieldDescriptor, depth: usize) -> serde_json::Value {
    use prost_reflect::Kind;
    if field.is_map() {
        // map<K, V> 的 JSON 形态是对象；给空对象骨架（键形态因 K 而异，不猜示例键）
        return serde_json::Value::Object(serde_json::Map::new());
    }
    let item = |kind: &Kind, depth: usize| -> serde_json::Value {
        match kind {
            Kind::Double | Kind::Float | Kind::Int32 | Kind::Uint32 | Kind::Sint32
            | Kind::Fixed32 | Kind::Sfixed32 => serde_json::json!(0),
            // proto JSON 映射：64 位整数是字符串
            Kind::Int64 | Kind::Uint64 | Kind::Sint64 | Kind::Fixed64 | Kind::Sfixed64 => {
                serde_json::json!("0")
            }
            Kind::Bool => serde_json::json!(false),
            Kind::String => serde_json::json!(""),
            Kind::Bytes => serde_json::json!(""),
            Kind::Enum(en) => serde_json::json!(
                en.values().next().map(|v| v.name().to_string()).unwrap_or_default()
            ),
            Kind::Message(msg) => message_json_template(msg, depth + 1),
        }
    };
    let value = item(&field.kind(), depth);
    if field.is_list() {
        serde_json::Value::Array(vec![value])
    } else {
        value
    }
}

/// 纯文本入参包装：入参消息恰好只有一个 string 字段时包成 `{"字段名": 文本}`；
/// 空文本包装为空消息 `{}`；其他消息形态报错并列出实际字段（明确不支持的原因）。
fn wrap_text_input(desc: &prost_reflect::MessageDescriptor, text: &str) -> Result<String, String> {
    let fields: Vec<prost_reflect::FieldDescriptor> = desc.fields().collect();
    if fields.len() != 1 || !matches!(fields.first().map(|f| f.kind()), Some(prost_reflect::Kind::String)) {
        let summary: Vec<String> = fields
            .iter()
            .map(|f| format!("{}({})", f.name(), match f.kind() {
                prost_reflect::Kind::String => "string",
                prost_reflect::Kind::Bool => "bool",
                prost_reflect::Kind::Int32 | prost_reflect::Kind::Sint32 | prost_reflect::Kind::Sfixed32 => "int32",
                prost_reflect::Kind::Int64 | prost_reflect::Kind::Sint64 | prost_reflect::Kind::Sfixed64 => "int64",
                prost_reflect::Kind::Message(_) => "message",
                prost_reflect::Kind::Enum(_) => "enum",
                _ => "其他",
            }))
            .collect();
        return Err(format!(
            "纯文本模式要求入参消息只有一个 string 字段，{} 实际字段：{}",
            desc.full_name(),
            if summary.is_empty() { "（空消息）".to_string() } else { summary.join(", ") }
        ));
    }
    if text.is_empty() {
        return Ok("{}".to_string());
    }
    let field = fields.first().map(|f| (f.json_name().to_string(), serde_json::Value::String(text.to_string())));
    let mut obj = serde_json::Map::new();
    if let Some((name, value)) = field {
        obj.insert(name, value);
    }
    Ok(serde_json::Value::Object(obj).to_string())
}

// ── 巡检：list（服务/方法）与 describe（proto 定义文本）──

/// 巡检请求（grpc_list / grpc_describe 共用；与 GrpcCallRequest 同款的连接语义）。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrpcInspectRequest {
    /// 目标地址 host:port
    pub target: String,
    /// TLS 配置库引用；None = 明文 h2c
    #[serde(default)]
    pub tls_profile_id: Option<usize>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

/// 方法摘要（list methods 返回项）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrpcMethodSummary {
    pub name: String,
    /// 入参消息全名
    pub input_type: String,
    /// 出参消息全名
    pub output_type: String,
    pub client_streaming: bool,
    pub server_streaming: bool,
}

/// list 二态结果：不给 service → 服务全名列表；给 service → 方法摘要列表。
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum GrpcListResult {
    Services { services: Vec<String> },
    Methods { methods: Vec<GrpcMethodSummary> },
}

/// 列出服务器上的服务（或某服务的方法）。经服务器反射（ListServices / 描述符）。
#[tauri::command]
pub async fn grpc_list(
    request: GrpcInspectRequest,
    service: Option<String>,
) -> Result<GrpcListResult, String> {
    let tls = resolve_tls_material(request.tls_profile_id)?;
    let timeout = Duration::from_millis(request.timeout_ms.unwrap_or(15_000));
    tokio::time::timeout(timeout, grpc_list_inner(request, service, tls))
        .await
        .map_err(|_| format!("gRPC list 超时（超过 {}ms）", timeout.as_millis()))?
}

async fn grpc_list_inner(
    request: GrpcInspectRequest,
    service: Option<String>,
    tls: Option<TlsMaterial>,
) -> Result<GrpcListResult, String> {
    let target = request.target.trim().to_string();
    if target.is_empty() {
        return Err("target 不能为空（host:port）".into());
    }
    let mut grpc = open_grpc_client(&target, tls).await?;
    let service = service
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    match service {
        None => {
            let services = list_services_reflection(&mut grpc).await?;
            log::debug!("grpc_list 服务数 {} target={target}", services.len());
            Ok(GrpcListResult::Services { services })
        }
        Some(service) => {
            let fdps = reflect_fdps(&mut grpc, &service).await?;
            let svc = crate::grpc_describe::find_service(&fdps, &service)
                .ok_or_else(|| format!("反射结果中找不到服务 {service}"))?;
            let methods = svc
                .method
                .iter()
                .map(|m| GrpcMethodSummary {
                    name: m.name.clone().unwrap_or_default(),
                    input_type: m
                        .input_type
                        .clone()
                        .unwrap_or_default()
                        .trim_start_matches('.')
                        .to_string(),
                    output_type: m
                        .output_type
                        .clone()
                        .unwrap_or_default()
                        .trim_start_matches('.')
                        .to_string(),
                    client_streaming: m.client_streaming.unwrap_or(false),
                    server_streaming: m.server_streaming.unwrap_or(false),
                })
                .collect();
            Ok(GrpcListResult::Methods { methods })
        }
    }
}

/// describe：给出符号（服务全名 / 服务.方法 / 消息全名 / 枚举全名）的 proto
/// 定义文本（grpcurl describe 风格），经服务器反射取 FileDescriptorProto 后渲染。
#[tauri::command]
pub async fn grpc_describe(
    request: GrpcInspectRequest,
    symbol: String,
) -> Result<String, String> {
    let tls = resolve_tls_material(request.tls_profile_id)?;
    let timeout = Duration::from_millis(request.timeout_ms.unwrap_or(15_000));
    tokio::time::timeout(timeout, grpc_describe_inner(request, symbol, tls))
        .await
        .map_err(|_| format!("gRPC describe 超时（超过 {}ms）", timeout.as_millis()))?
}

async fn grpc_describe_inner(
    request: GrpcInspectRequest,
    symbol: String,
    tls: Option<TlsMaterial>,
) -> Result<String, String> {
    let target = request.target.trim().to_string();
    if target.is_empty() {
        return Err("target 不能为空（host:port）".into());
    }
    let symbol = symbol.trim().trim_start_matches('.').to_string();
    if symbol.is_empty() {
        return Err("符号名不能为空".into());
    }
    let mut grpc = open_grpc_client(&target, tls).await?;
    let fdps = reflect_fdps(&mut grpc, &symbol).await?;
    crate::grpc_describe::render_symbol_definition(&fdps, &symbol)
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

async fn call_grpc_unary(
    req: GrpcCallRequest,
    tls: Option<TlsMaterial>,
) -> Result<GrpcCallResponse, String> {
    let timeout = Duration::from_millis(req.timeout_ms.unwrap_or(15_000));
    tokio::time::timeout(timeout, call_grpc_unary_inner(req, tls))
        .await
        .map_err(|_| format!("gRPC 调用超时（超过 {}ms）", timeout.as_millis()))?
}

// ── 连接构造（调用与巡检命令共用，支持 Channel 缓存池复用）──

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct GrpcChannelKey {
    target: String,
    tls: Option<TlsMaterial>,
}

type ChannelPool = RwLock<HashMap<GrpcChannelKey, tonic::transport::Channel>>;

fn get_channel_pool() -> &'static ChannelPool {
    static POOL: OnceLock<ChannelPool> = OnceLock::new();
    POOL.get_or_init(|| RwLock::new(HashMap::new()))
}

/// 建立到 target 的 gRPC 客户端（明文或 TLS 材料），含 Channel 缓存池复用与 Buffer 的 ready() 语义。
async fn open_grpc_client(
    target: &str,
    tls: Option<TlsMaterial>,
) -> Result<GrpcClient, String> {
    let key = GrpcChannelKey {
        target: target.trim().to_string(),
        tls: tls.clone(),
    };

    // 1. 优先尝试从连接池复用既有的 HTTP/2 Channel（避免重复 TCP 握手与 TLS 协商延迟）
    let cached_channel = {
        let pool = get_channel_pool().read().await;
        pool.get(&key).cloned()
    };

    if let Some(channel) = cached_channel {
        let mut grpc = tonic::client::Grpc::new(channel);
        if grpc.ready().await.is_ok() {
            return Ok(grpc);
        }
        // 若就绪失败（服务端断开或重启），从池中移除已失效连接
        let mut pool = get_channel_pool().write().await;
        pool.remove(&key);
    }

    // 2. 缓存未命中或失效时，重新建立连接
    let scheme = if tls.is_some() { "https" } else { "http" };
    let uri: http::Uri = format!("{scheme}://{target}/")
        .parse()
        .map_err(|e| format!("target 非法（{target}）: {e}"))?;
    let mut endpoint = tonic::transport::Channel::builder(uri);
    if let Some(tls) = &tls {
        ensure_tls_provider();
        let mut cfg = tonic::transport::ClientTlsConfig::new();
        match &tls.ca_pem {
            Some(pem) => {
                cfg = cfg.ca_certificate(tonic::transport::Certificate::from_pem(pem));
            }
            None => cfg = cfg.with_native_roots(),
        }
        match (&tls.client_cert_pem, &tls.client_key_pem) {
            (Some(cert), Some(key)) => {
                cfg = cfg.identity(tonic::transport::Identity::from_pem(cert, key));
            }
            (None, None) => {}
            _ => {
                return Err(
                    "mTLS 客户端证书与私钥必须成对提供（clientCertPem + clientKeyPem）".into(),
                );
            }
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

    // 存入 Channel 连接池（容量上限 64）
    {
        let mut pool = get_channel_pool().write().await;
        if pool.len() >= 64 {
            pool.clear();
        }
        pool.insert(key, channel.clone());
    }

    let mut grpc = tonic::client::Grpc::new(channel);
    // Channel 内部是 tower Buffer：必须先 ready() 才能 call（codegen 生成的客户端
    // 同样会先 await ready；跳过会触发 Buffer 的 send_item/poll_reserve 断言 panic）
    grpc.ready()
        .await
        .map_err(|e| format!("连接就绪失败（{target}）: {e}"))?;
    Ok(grpc)
}

async fn call_grpc_unary_inner(
    req: GrpcCallRequest,
    tls: Option<TlsMaterial>,
) -> Result<GrpcCallResponse, String> {
    let timeout = Duration::from_millis(req.timeout_ms.unwrap_or(15_000));
    let target = req.target.trim().to_string();
    if target.is_empty() {
        return Err("target 不能为空（host:port）".into());
    }
    let (service, method) = (req.service.trim(), req.method.trim());
    if service.is_empty() || method.is_empty() {
        return Err("服务名与方法名均不能为空".into());
    }
    let mut grpc = open_grpc_client(&target, tls).await?;

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

    // 3. JSON 入参 → 动态消息（proto JSON 语义：字段 camelCase、int64 字符串）；
    //    纯文本模式：入参消息只有一个 string 字段时自动包装，否则报错
    let json_effective = if req.input_mode.as_deref() == Some("text") {
        wrap_text_input(&method_desc.input(), &req.json_input)?
    } else {
        req.json_input.clone()
    };
    let json_input = if json_effective.trim().is_empty() {
        "{}"
    } else {
        json_effective.trim()
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
        let detail = format_grpc_status(&s);
        log::warn!(
            "grpc_call 一元调用失败 target={target} service={service} method={method}: {s:?}"
        );
        format!("gRPC 调用失败 [{code}]: {detail}", code = s.code())
    })?;

    // 5. 动态消息 → JSON 出参
    let json_output = serde_json::to_string(&response.into_inner())
        .map_err(|e| format!("出参序列化 JSON 失败: {e}"))?;
    log::debug!(
        "grpc_call 成功 target={target} service={service} method={method}，出参 {} 字节",
        json_output.len()
    );
    Ok(GrpcCallResponse { json_output })
}

/// gRPC Status 的可读详情：优先 grpc-message；为空时解析
/// grpc-status-details-bin（google.rpc.Status，Java/Go 服务端内部异常的真实文案）。
/// 全部缺失时给简洁文案——原始 Status Debug 只写日志（warn），不上 UI。
fn format_grpc_status(s: &tonic::Status) -> String {
    if !s.message().is_empty() {
        return s.message().to_string();
    }
    // tonic 已把 grpc-status-details-bin 解码进 Status::details（google.rpc.Status 线格式）
    if !s.details().is_empty()
        && let Ok(detail) = rpc_status::RpcStatus::decode(s.details())
    {
        let mut parts = vec![format!(
            "服务端内部错误 code={}{}",
            detail.code,
            if detail.message.is_empty() { String::new() } else { format!(": {}", detail.message) }
        )];
        if !detail.details.is_empty() {
            let urls: Vec<&str> = detail.details.iter().map(|a| a.type_url.as_str()).collect();
            parts.push(format!("details=[{}]", urls.join(", ")));
        }
        return parts.join(" ");
    }
    "服务端内部错误，未返回详细原因（完整状态已记录到应用日志）".to_string()
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
    Err(reflection_unsupported_message(symbol, &unsupported))
}

/// reflect_descriptor_pool 的 fdps 层（describe 渲染用）。
async fn reflect_fdps(
    grpc: &mut GrpcClient,
    symbol: &str,
) -> Result<Vec<prost_types::FileDescriptorProto>, String> {
    let mut unsupported: Vec<String> = Vec::new();
    for svc_full in REFLECTION_SERVICES {
        match reflect_fdps_via(grpc, svc_full, symbol).await {
            Ok(fdps) => return Ok(fdps),
            Err(ReflectError::Unsupported(m)) => unsupported.push(m),
            Err(ReflectError::Other(m)) => return Err(m),
        }
    }
    Err(reflection_unsupported_message(symbol, &unsupported))
}

/// ListServices("*")：反射流式请求收服务全名列表（dual-try v1alpha/v1）。
async fn list_services_reflection(grpc: &mut GrpcClient) -> Result<Vec<String>, String> {
    let mut unsupported: Vec<String> = Vec::new();
    for svc_full in REFLECTION_SERVICES {
        match send_reflection_request(
            grpc,
            svc_full,
            MessageRequest::ListServices("*".to_string()),
        )
        .await
        {
            Ok(resp) => {
                if let Some(MessageResponse::ListServiceResponse(listed)) =
                    resp.message_response
                {
                    let mut names: Vec<String> =
                        listed.service.into_iter().map(|s| s.name).collect();
                    names.sort();
                    return Ok(names);
                }
                return Err("反射返回了非预期的消息类型（期待 ListServiceResponse）".into());
            }
            Err(ReflectError::Unsupported(m)) => unsupported.push(m),
            Err(ReflectError::Other(m)) => return Err(m),
        }
    }
    Err(reflection_unsupported_message("(list services)", &unsupported))
}

fn reflection_unsupported_message(symbol: &str, unsupported: &[String]) -> String {
    format!(
        "服务器不支持 gRPC 反射（{}），无法在无 proto 文件的情况下定位 {symbol} 的消息定义",
        unsupported.join("; ")
    )
}

/// 单版本反射：file_containing_symbol 拿主文件，再按 dependency 逐个
/// file_by_filename 补齐闭包（部分实现只返回主文件），上限 100 轮防环。
async fn reflect_via(
    grpc: &mut GrpcClient,
    svc_full: &str,
    symbol: &str,
) -> Result<prost_reflect::DescriptorPool, ReflectError> {
    let fdps = reflect_fdps_via(grpc, svc_full, symbol).await?;
    let set = prost_types::FileDescriptorSet { file: fdps };
    let set_bytes = set.encode_to_vec();
    prost_reflect::DescriptorPool::decode(set_bytes.as_slice())
        .map_err(|e| ReflectError::Other(format!("反射文件集无法构建 DescriptorPool: {e}")))
}

/// reflect_via 的 fdps 暴露层：describe 渲染直接吃 FileDescriptorProto（grpcurl
/// 的 protoprint 同样在 fdp 层工作），避免 DescriptorPool 反查原始定义的丢失。
async fn reflect_fdps_via(
    grpc: &mut GrpcClient,
    svc_full: &str,
    symbol: &str,
) -> Result<Vec<prost_types::FileDescriptorProto>, ReflectError> {
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
        log::debug!("反射 {svc_full} 收到响应（累计 {} 个文件）", fdps.len());
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
    Ok(fdps)
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
            let detail = format_grpc_status(&s);
            log::warn!("反射调用失败 {svc_full}: {s:?}");
            if s.code() == tonic::Code::Unimplemented {
                ReflectError::Unsupported(format!("{svc_full}: {detail}"))
            } else {
                ReflectError::Other(format!(
                    "反射服务调用失败 [{}]: {}",
                    s.code(),
                    detail
                ))
            }
        })?;
    let mut inbound = response.into_inner();
    while let Some(item) = inbound.next().await {
        let resp = item.map_err(|s| {
            log::warn!("反射流中断 {svc_full}: {s:?}");
            ReflectError::Other(format!(
                "反射流中断 [{}]: {}",
                s.code(),
                format_grpc_status(&s)
            ))
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
    use base64::Engine as _;
    use std::convert::Infallible;
    use std::pin::Pin;
    use std::sync::{Arc, Mutex};
    use std::task::{Context, Poll};

    use http_body_util::BodyExt as _;
    use tonic::codegen::Service;
    use tonic::server::NamedService;
    use tonic::transport::Identity;

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

    #[derive(Clone, PartialEq, prost::Message)]
    #[allow(dead_code)]
    struct BoomRequest {}

    #[derive(Clone, PartialEq, prost::Message)]
    #[allow(dead_code)]
    struct BoomResponse {}

    #[derive(Clone, PartialEq, prost::Message)]
    struct EchoRequest {
        #[prost(string, tag = "1")]
        message: String,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    struct EchoResponse {
        #[prost(string, tag = "1")]
        reply: String,
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
                p::DescriptorProto {
                    name: Some("BoomRequest".to_string()),
                    ..Default::default()
                },
                p::DescriptorProto {
                    name: Some("BoomResponse".to_string()),
                    ..Default::default()
                },
                p::DescriptorProto {
                    name: Some("EchoRequest".to_string()),
                    field: vec![field("message", 1, p::field_descriptor_proto::Type::String)],
                    ..Default::default()
                },
                p::DescriptorProto {
                    name: Some("EchoResponse".to_string()),
                    field: vec![field("reply", 1, p::field_descriptor_proto::Type::String)],
                    ..Default::default()
                },
            ],
            service: vec![p::ServiceDescriptorProto {
                name: Some("UserService".to_string()),
                method: vec![
                    p::MethodDescriptorProto {
                        name: Some("GetUser".to_string()),
                        input_type: Some(".veloterm.test.GetUserRequest".to_string()),
                        output_type: Some(".veloterm.test.GetUserResponse".to_string()),
                        ..Default::default()
                    },
                    p::MethodDescriptorProto {
                        name: Some("Boom".to_string()),
                        input_type: Some(".veloterm.test.BoomRequest".to_string()),
                        output_type: Some(".veloterm.test.BoomResponse".to_string()),
                        ..Default::default()
                    },
                    p::MethodDescriptorProto {
                        name: Some("Echo".to_string()),
                        input_type: Some(".veloterm.test.EchoRequest".to_string()),
                        output_type: Some(".veloterm.test.EchoResponse".to_string()),
                        ..Default::default()
                    },
                ],
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
                if path == "/veloterm.test.UserService/Boom" {
                    // 模拟 Java/Go 服务端内部异常：grpc-status 13 + 空 grpc-message +
                    // grpc-status-details-bin（google.rpc.Status 才有真实文案）
                    let _ = read_grpc_frame(body).await;
                    return Ok(grpc_internal_with_details("boom:模拟服务端空指针"));
                }
                if path == "/veloterm.test.UserService/Echo" {
                    let payload = match read_grpc_frame(body).await {
                        Ok(p) => p,
                        Err(e) => return Ok(grpc_trailers_response(13, &format!("bad frame: {e}"))),
                    };
                    let request = match EchoRequest::decode(payload.as_slice()) {
                        Ok(r) => r,
                        Err(e) => return Ok(grpc_trailers_response(13, &format!("bad payload: {e}"))),
                    };
                    let response = EchoResponse { reply: format!("echo:{}", request.message) };
                    return Ok(grpc_ok_response(&response.encode_to_vec()));
                }
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
                    // 测试文件里的任何符号（服务/方法/消息）都能定位到 veloterm_test.proto
                    Some(MessageRequest::FileContainingSymbol(sym))
                        if sym == TEST_SERVICE || sym.starts_with("veloterm.test.") =>
                    {
                        MessageResponse::FileDescriptorResponse(reflection::FileDescriptorResponse {
                            file_descriptor_proto: vec![test_fdp_bytes()],
                        })
                    }
                    Some(MessageRequest::FileByFilename(name)) if name == "veloterm_test.proto" => {
                        MessageResponse::FileDescriptorResponse(reflection::FileDescriptorResponse {
                            file_descriptor_proto: vec![test_fdp_bytes()],
                        })
                    }
                    Some(MessageRequest::ListServices(_)) => {
                        MessageResponse::ListServiceResponse(reflection::ListServiceResponse {
                            service: vec![reflection::ServiceResponse {
                                name: TEST_SERVICE.to_string(),
                            }],
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
        build_grpc_response(Some(payload), 0, "", None)
    }

    fn grpc_trailers_response(status: u16, message: &str) -> http::Response<tonic::body::Body> {
        build_grpc_response(None, status, message, None)
    }

    /// INTERNAL + 空 grpc-message + grpc-status-details-bin（google.rpc.Status）。
    fn grpc_internal_with_details(message: &str) -> http::Response<tonic::body::Body> {
        let detail = rpc_status::RpcStatus {
            code: 13,
            message: message.to_string(),
            details: vec![],
        };
        let encoded = base64::engine::general_purpose::STANDARD.encode(detail.encode_to_vec());
        build_grpc_response(
            None,
            13,
            "",
            Some((http::HeaderName::from_static("grpc-status-details-bin"), encoded)),
        )
    }

    #[allow(clippy::type_complexity)]
    fn build_grpc_response(
        payload: Option<&[u8]>,
        status: u16,
        message: &str,
        bin_trailer: Option<(http::HeaderName, String)>,
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
        if let Some((name, value)) = bin_trailer
            && let Ok(v) = http::HeaderValue::from_str(&value)
        {
            trailers.insert(name, v);
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

    /// 服务端 TLS 形态：明文 / 普通 TLS / 强制客户端证书（mTLS）
    #[derive(Clone)]
    enum TlsMode {
        None,
        Plain,
        Mutual(String), // 客户端 CA PEM
    }

    struct TestServer {
        target: String,
        ca_pem: Option<String>,
        seen_authorization: Arc<Mutex<Option<String>>>,
    }

    async fn spawn_test_server(
        reflection: ReflectionVersion,
        tls: TlsMode,
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
        let client_ca = match &tls {
            TlsMode::Mutual(ca) => Some(ca.clone()),
            _ => None,
        };
        if !matches!(tls, TlsMode::None) {
            ensure_tls_provider();
            let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
            let cert_pem = cert.cert.pem();
            let key_pem = cert.key_pair.serialize_pem();
            ca_pem = Some(cert_pem.clone());
            let mut tls_config =
                tonic::transport::ServerTlsConfig::new().identity(Identity::from_pem(cert_pem, key_pem));
            if let Some(client_ca) = client_ca {
                tls_config = tls_config
                    .client_ca_root(tonic::transport::Certificate::from_pem(client_ca));
            }
            server = server.tls_config(tls_config).unwrap();
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
            tls_profile_id: None,
            service: "veloterm.test.UserService".to_string(),
            method: "GetUser".to_string(),
            json_input: r#"{"id": 1}"#.to_string(),
            input_mode: None,
            bearer_token: None,
            headers: vec![],
            timeout_ms: Some(5_000),
        }
    }

    // ── 用例 ──

    #[test]
    fn grpc_status_format_fallback_is_clean() {
        // 空 message + 无 details：兜底文案必须简洁，不得把 Status Debug 拼给 UI
        let s = tonic::Status::internal("");
        let text = format_grpc_status(&s);
        assert!(text.contains("未返回详细原因"), "text: {text}");
        assert!(!text.contains("Status debug"), "text: {text}");
    }

    #[tokio::test]
    async fn grpc_call_text_input_wraps_single_string_field() {
        let server = spawn_test_server(ReflectionVersion::V1Alpha, TlsMode::None, None).await;
        let mut req = base_request(&server.target);
        req.method = "Echo".to_string();
        req.json_input = "你好 gRPC".to_string();
        req.input_mode = Some("text".to_string());
        let resp = call_grpc_unary(req, None).await.expect("纯文本模式 Echo 应成功");
        assert!(resp.json_output.contains("echo:你好 gRPC"), "resp: {}", resp.json_output);
    }

    #[tokio::test]
    async fn grpc_call_text_input_rejects_multi_field_message() {
        let server = spawn_test_server(ReflectionVersion::V1Alpha, TlsMode::None, None).await;
        let mut req = base_request(&server.target); // GetUser 入参是 int64 字段（非单 string）
        req.json_input = "raw text".to_string();
        req.input_mode = Some("text".to_string());
        let err = call_grpc_unary(req, None).await.expect_err("非单 string 消息纯文本应报错");
        assert!(err.contains("只有一个 string 字段"), "err: {err}");
    }

    #[tokio::test]
    async fn grpc_call_plaintext_roundtrip_with_bearer() {
        let server = spawn_test_server(
            ReflectionVersion::V1Alpha,
            TlsMode::None,
            Some("Bearer tok-123".to_string()),
        )
        .await;
        let mut req = base_request(&server.target);
        req.bearer_token = Some("tok-123".to_string());
        let resp = call_grpc_unary(req, None).await.expect("明文回环调用应成功");
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
            TlsMode::None,
            Some("Bearer tok-123".to_string()),
        )
        .await;
        let mut req = base_request(&server.target);
        req.bearer_token = Some("wrong".to_string());
        let err = call_grpc_unary(req, None).await.expect_err("错误 token 应被拒绝");
        assert!(err.contains("PermissionDenied"), "错误信息应为权限拒绝: {err}");
    }

    #[tokio::test]
    async fn grpc_call_tls_with_self_signed_ca() {
        let server = spawn_test_server(ReflectionVersion::V1Alpha, TlsMode::Plain, None).await;
        let req = base_request(&server.target);
        let tls = TlsMaterial {
            ca_pem: server.ca_pem.clone(),
            server_name: Some("localhost".to_string()),
            client_cert_pem: None,
            client_key_pem: None,
        };
        let resp = call_grpc_unary(req, Some(tls)).await.expect("自定义 CA 的 TLS 调用应成功");
        assert!(resp.json_output.contains("Alice"));
    }

    #[tokio::test]
    async fn grpc_call_tls_with_wrong_ca_fails() {
        let server = spawn_test_server(ReflectionVersion::V1Alpha, TlsMode::Plain, None).await;
        let wrong_ca = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
        let req = base_request(&server.target);
        let tls = TlsMaterial {
            ca_pem: Some(wrong_ca.cert.pem()),
            server_name: Some("localhost".to_string()),
            client_cert_pem: None,
            client_key_pem: None,
        };
        let err = call_grpc_unary(req, Some(tls)).await.expect_err("错误 CA 应握手失败");
        assert!(err.contains("连接"), "错误应为连接失败: {err}");
    }

    #[tokio::test]
    async fn grpc_call_reflection_v1_fallback() {
        // 只挂 v1 反射：dual-try 应在 v1alpha UNIMPLEMENTED 后落到 v1 成功
        let server = spawn_test_server(ReflectionVersion::V1, TlsMode::None, None).await;
        let resp = call_grpc_unary(base_request(&server.target), None)
            .await
            .expect("仅 v1 反射时也应成功");
        assert!(resp.json_output.contains("Alice"));
    }

    #[tokio::test]
    async fn grpc_call_mtls_with_client_cert() {
        // 服务端强制客户端证书：客户端证书的 CA（此处直接用自签证书自身）交给服务端校验
        let client_cert =
            rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
        let server = spawn_test_server(
            ReflectionVersion::V1Alpha,
            TlsMode::Mutual(client_cert.cert.pem()),
            None,
        )
        .await;
        let req = base_request(&server.target);
        let tls = TlsMaterial {
            ca_pem: server.ca_pem.clone(),
            server_name: Some("localhost".to_string()),
            client_cert_pem: Some(client_cert.cert.pem()),
            client_key_pem: Some(client_cert.key_pair.serialize_pem()),
        };
        let resp = call_grpc_unary(req, Some(tls)).await.expect("携带客户端证书的 mTLS 调用应成功");
        assert!(resp.json_output.contains("Alice"));
    }

    #[tokio::test]
    async fn grpc_call_mtls_missing_client_cert_fails() {
        let client_cert =
            rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
        let server = spawn_test_server(
            ReflectionVersion::V1Alpha,
            TlsMode::Mutual(client_cert.cert.pem()),
            None,
        )
        .await;
        let req = base_request(&server.target);
        let tls = TlsMaterial {
            ca_pem: server.ca_pem.clone(),
            server_name: Some("localhost".to_string()),
            client_cert_pem: None,
            client_key_pem: None,
        };
        let err = call_grpc_unary(req, Some(tls)).await.expect_err("缺客户端证书应握手失败");
        // 服务端在 TLS 握手阶段拒连：tonic 客户端表现为首个 RPC 的 transport error
        // （TCP/TLS 握手不总能前移到 connect 阶段）
        assert!(
            err.contains("连接") || err.contains("transport error"),
            "错误应为传输层失败: {err}"
        );
    }

    #[tokio::test]
    async fn grpc_call_internal_error_with_details_bin() {
        // grpc-message 为空的服务端内部错误：真实文案应从 grpc-status-details-bin 解出
        let server = spawn_test_server(ReflectionVersion::V1Alpha, TlsMode::None, None).await;
        let mut req = base_request(&server.target);
        req.method = "Boom".to_string();
        req.json_input = "{}".to_string();
        let err = call_grpc_unary(req, None).await.expect_err("Boom 应返回内部错误");
        assert!(
            err.contains("boom:模拟服务端空指针"),
            "错误应包含 details-bin 里的真实文案: {err}"
        );
    }

    #[tokio::test]
    async fn grpc_call_without_reflection_reports_unsupported() {
        let server = spawn_test_server(ReflectionVersion::None, TlsMode::None, None).await;
        let err = call_grpc_unary(base_request(&server.target), None)
            .await
            .expect_err("无反射服务应报错");
        assert!(
            err.contains("不支持 gRPC 反射"),
            "错误应明确指出不支持反射: {err}"
        );
    }

    #[tokio::test]
    async fn grpc_call_method_not_found_lists_available() {
        let server = spawn_test_server(ReflectionVersion::V1Alpha, TlsMode::None, None).await;
        let mut req = base_request(&server.target);
        req.method = "GetUserX".to_string();
        let err = call_grpc_unary(req, None).await.expect_err("方法名错误应报错");
        assert!(
            err.contains("可用方法") && err.contains("GetUser"),
            "错误应列出可用方法: {err}"
        );
    }

    #[tokio::test]
    async fn grpc_call_bad_json_input_type_mismatch() {
        let server = spawn_test_server(ReflectionVersion::V1Alpha, TlsMode::None, None).await;
        let mut req = base_request(&server.target);
        req.json_input = r#"{"id": "not-a-number"}"#.to_string();
        let err = call_grpc_unary(req, None).await.expect_err("入参类型错误应报错");
        assert!(
            err.contains("入参 JSON"),
            "错误应指出入参 JSON 问题: {err}"
        );
    }

    // ── list / describe 巡检 ──

    fn inspect_request(target: &str) -> GrpcInspectRequest {
        GrpcInspectRequest {
            target: target.to_string(),
            tls_profile_id: None,
            timeout_ms: Some(5_000),
        }
    }

    #[tokio::test]
    async fn grpc_list_services_via_reflection() {
        let server = spawn_test_server(ReflectionVersion::V1Alpha, TlsMode::None, None).await;
        let result = grpc_list(inspect_request(&server.target), None)
            .await
            .expect("list services 应成功");
        // 形态不符时落到空列表，断言失败自然报错（避免 panic 宏碰 clippy::panic deny）
        let services = match result {
            GrpcListResult::Services { services } => services,
            GrpcListResult::Methods { .. } => Vec::new(),
        };
        assert!(services.contains(&TEST_SERVICE.to_string()), "services: {services:?}");
    }

    #[tokio::test]
    async fn grpc_list_methods_summary() {
        let server = spawn_test_server(ReflectionVersion::V1Alpha, TlsMode::None, None).await;
        let result = grpc_list(inspect_request(&server.target), Some(TEST_SERVICE.to_string()))
            .await
            .expect("list methods 应成功");
        let methods = match result {
            GrpcListResult::Methods { methods } => methods,
            GrpcListResult::Services { .. } => Vec::new(),
        };
        let get_user = methods
            .iter()
            .find(|m| m.name == "GetUser")
            .expect("应包含 GetUser");
        assert_eq!(get_user.input_type, "veloterm.test.GetUserRequest");
        assert_eq!(get_user.output_type, "veloterm.test.GetUserResponse");
        assert!(!get_user.client_streaming && !get_user.server_streaming);
        assert!(methods.iter().any(|m| m.name == "Boom"), "methods: {methods:?}");
    }

    #[tokio::test]
    async fn grpc_method_schema_json_templates() {
        let server = spawn_test_server(ReflectionVersion::V1Alpha, TlsMode::None, None).await;
        let schema = grpc_method_schema(
            inspect_request(&server.target),
            TEST_SERVICE.to_string(),
            "GetUser".to_string(),
        )
        .await
        .expect("schema 获取应成功");
        // int64 入参：proto JSON 映射为字符串；int32 出参为数字
        assert_eq!(schema.request_schema, serde_json::json!({"id": "0"}));
        assert_eq!(
            schema.response_schema,
            serde_json::json!({"name": "", "age": 0})
        );
    }

    #[tokio::test]
    async fn grpc_describe_service_renders_proto() {
        let server = spawn_test_server(ReflectionVersion::V1Alpha, TlsMode::None, None).await;
        let text = grpc_describe(inspect_request(&server.target), TEST_SERVICE.to_string())
            .await
            .expect("describe service 应成功");
        assert!(text.contains("is a service"), "text: {text}");
        assert!(text.contains("service UserService {"));
        assert!(text.contains("rpc GetUser(veloterm.test.GetUserRequest) returns (veloterm.test.GetUserResponse);"));
        // 连带渲染方法入出参消息定义
        assert!(text.contains("message GetUserRequest {"));
        assert!(text.contains("int64 id = 1;"));
        assert!(text.contains("string name = 1;"));
    }

    #[tokio::test]
    async fn grpc_describe_method_with_messages() {
        let server = spawn_test_server(ReflectionVersion::V1Alpha, TlsMode::None, None).await;
        let symbol = format!("{TEST_SERVICE}.GetUser");
        let text = grpc_describe(inspect_request(&server.target), symbol)
            .await
            .expect("describe method 应成功");
        assert!(text.contains("is a method"), "text: {text}");
        assert!(text.contains("rpc GetUser(veloterm.test.GetUserRequest)"));
        assert!(text.contains("message GetUserRequest {"));
    }

    #[tokio::test]
    async fn grpc_describe_message_definition() {
        let server = spawn_test_server(ReflectionVersion::V1Alpha, TlsMode::None, None).await;
        let text = grpc_describe(
            inspect_request(&server.target),
            "veloterm.test.GetUserResponse".to_string(),
        )
        .await
        .expect("describe message 应成功");
        assert!(text.contains("is a message"), "text: {text}");
        assert!(text.contains("string name = 1;"));
        assert!(text.contains("int32 age = 2;"));
    }

    #[tokio::test]
    async fn grpc_describe_unknown_symbol_reports_available() {
        let server = spawn_test_server(ReflectionVersion::V1Alpha, TlsMode::None, None).await;
        let err = grpc_describe(inspect_request(&server.target), "no.such.Symbol".to_string())
            .await
            .expect_err("未知符号应报错");
        // 反射服务端以 NOT_FOUND 拒绝未知 symbol（真实服务器同款行为）
        assert!(err.contains("not found"), "err: {err}");
    }
}
