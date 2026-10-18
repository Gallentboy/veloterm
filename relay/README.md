# veloterm-relay

veloterm 的 SSH 中继服务端：部署在公网/可达机器上，把 veloterm 客户端的连接
转发到目标主机（如内网 LINUX1 的 22 端口）。中继对 SSH 协议零解析——
VLR2 握手鉴权后就是纯字节管道，SSH/SFTP 端到端运行。

## 结构

- `crates/veloterm-relay-proto` —— 线缆协议与鉴权的单一实现（服务端与
  veloterm 客户端共用，杜绝双份实现）。无 runtime 依赖，轻量可复用。
- `crates/veloterm-relay` —— 服务端二进制 `veloterm_relay`。

## 快速开始

```bash
# 生成客户端凭证（32 字节随机 secret）
cargo run -p veloterm-relay -- token generate --key-id 1 --name macbook

# 写配置（端口全部可配，监听器数量任意）
cat > relay.toml <<'EOF'
[[token]]
key_id = 1
name  = "macbook"
secret = "<上面生成的 hex>"

[[listener]]
proto = "tcp"
bind = "0.0.0.0:7300"
target = "linux1.internal:22"        # 默认目标
allow_targets = ["10.0.0.5:22"]      # 客户端可显式指定的白名单（可选）
EOF

cargo run -p veloterm-relay -- -c relay.toml          # 运行
cargo run -p veloterm-relay -- -c relay.toml --check  # 仅校验配置
```

## 鉴权与传输加密（VLR3）

凭证（32 字节 secret）**永不上线、也从不直接加解密数据**。每连接协商全新密钥：

```text
① 密钥协商（明文区，仅含随机字节形态的公钥与 nonce——协议特征被混淆）
   C→R: "VLR3" | key_id | 客户端临时公钥(32B) | 客户端nonce
   R→C: "VLR3" | 服务端临时公钥(32B) | 服务端nonce
   双方各自推导: keys = HKDF-SHA256(凭证 ‖ X25519共享秘密, 双nonce)   ← 从不传输
② 认证+选路（全部进 AEAD 加密记录层，目标地址也被隐藏）
   C→R: 加密{ target | proof }   proof = HMAC(凭证, 完整协商上下文)
   R→C: 加密{ status }
③ 数据面：双向字节整体跑在 ChaCha20-Poly1305 记录层内
   （每帧独立认证，篡改/重放任一帧立即断连；SSH 端到端加密嵌套其中）
```

安全性质：无凭证者（含中间人）推导不出密钥；录制流量事后泄露凭证也解不开
（临时 DH 前向保密）；未知 key_id 走诱饵同形拒绝（防枚举）；凭证不匹配表现为
记录校验失败（客户端提示 AuthMismatch）。tcp/ws 由此获得与 QUIC 同级的传输
加密；QUIC 上叠用 V3 使流量形态对中转方也呈随机字节。
状态码：`0 OK · 1 格式错误 · 2 鉴权失败 · 3 目标不允许 · 4 目标连接失败`。

## 阶段状态

| 阶段 | 内容 | 状态 |
| --- | --- | --- |
| P1 | 协议 crate + TCP 监听（握手/鉴权/选路/搬运）+ russh 回环端到端 | ✅ |
| P2 | WebSocket / QUIC / UDP 监听 | ✅（ws 适配器字节流 / quic 自签+多流复用 / udp 会话表+时间同步） |
| P3 | veloterm 客户端接入（proto path 依赖 + connect_stream + 主机表中继配置 + 拓扑可视化） | ✅ |
| P4 | CI 车道 + Linux 产物 | ✅（test-relay.yml：test + clippy -D warnings + musl 静态产物，tag `test_relay` 触发） |

## 分发

- 手动验证车道：推 `test_relay` tag → 四目标产物（linux x86_64/aarch64 musl 静态 +
  win x64/arm64 crt-static）见该 run 的 Artifacts（7 天有效期）。
- **随 release 发布**：push release 时若 `relay/**` 有变化，publish 流水线自动构建
  四个二进制并挂到 `app-v<版本>` 的 Releases 页面资产里（命名
  `veloterm_relay-linux-x86` / `veloterm_relay-linux-aarch64` /
  `veloterm_relay-x64.exe` / `veloterm_relay-arm64.exe`）；
  relay 无变化则整段跳过。

## 测试

```bash
cd relay && cargo test && cargo clippy --workspace --all-targets
```

集成测试用 `russh` 服务端作回环 SSH 目标，验证「SSH 完整跑在中继管道上」
（即 veloterm 主程序未来的接入形态：握手后的流直接喂 `connect_stream`）。
