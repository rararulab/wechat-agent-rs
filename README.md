# wechat-agent-rs

> **AI-Generated Project** — 本项目由 [Claude Code](https://claude.com/claude-code)（Claude Opus 4.6）全程生成，包括代码实现、测试、文档、CI/CD 配置，无人工编写代码。

微信 Agent SDK 的 Rust 实现，从 [frostming/weixin-agent-sdk](https://github.com/frostming/weixin-agent-sdk)（Python）移植而来。原项目由 [Frost Ming](https://github.com/frostming) 开发，本项目是其 Rust 等价实现，保持相同的顶层 API 设计（`Agent` trait、`login()`、`start()`）。

通过长轮询方式接收微信消息，并分发给 AI Agent 处理。无需搭建 HTTP 服务，开箱即用。

## 功能特性

- 扫码登录微信
- 长轮询接收消息（文本、图片、语音、视频、文件）
- AES-128-ECB 媒体加解密
- 自动发送 typing 状态
- Markdown 转纯文本（适配微信消息格式）
- 本地凭证持久化
- 异步架构（tokio）

## 快速开始

### 安装

在 `Cargo.toml` 中添加依赖：

```toml
[dependencies]
wechat-agent-rs = { git = "https://github.com/rararulab/wechat-agent-rs" }
tokio = { version = "1", features = ["full"] }
snafu = "0.9"
anyhow = "1"
tracing-subscriber = "0.3"
```

### 实现 Agent

只需实现 `Agent` trait 的 `chat` 方法：

```rust
use std::{future::Future, pin::Pin, sync::Arc};

use wechat_agent_rs::{Agent, ChatRequest, ChatResponse, LoginOptions, StartOptions, login, start};

// 定义你的 Agent 结构体
struct EchoAgent;

impl Agent for EchoAgent {
    fn chat(
        &self,
        request: ChatRequest,
    ) -> Pin<Box<dyn Future<Output = wechat_agent_rs::Result<ChatResponse>> + Send + '_>> {
        Box::pin(async move {
            // 将用户发送的消息原样返回
            Ok(ChatResponse {
                text:  Some(format!("你说了: {}", request.text)),
                media: None,
            })
        })
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 初始化日志
    tracing_subscriber::fmt::init();

    // 如果已有保存的账号则复用，否则扫码登录
    let account_id = match wechat_agent_rs::storage::get_account_ids() {
        Ok(ids) if !ids.is_empty() => ids[0].clone(),
        _ => login(LoginOptions::default()).await?,
    };

    println!("使用账号: {account_id}");

    // 启动消息循环
    let agent = Arc::new(EchoAgent);
    start(
        agent,
        StartOptions {
            account_id: Some(account_id),
        },
    )
    .await?;

    Ok(())
}
```

### 运行

```bash
cargo run --example echo_bot
```

首次运行会在终端显示二维码，用微信扫码登录。登录后凭证保存在 `~/.openclaw/openclaw-weixin/`，后续运行自动复用。

## 核心概念

### Agent trait

`Agent` 是 SDK 的核心抽象。你只需实现 `chat` 方法，SDK 会自动完成消息收发、媒体处理、状态管理等工作。

```rust
pub trait Agent: Send + Sync {
    fn chat(
        &self,
        request: ChatRequest,
    ) -> Pin<Box<dyn Future<Output = Result<ChatResponse>> + Send + '_>>;
}
```

**`ChatRequest`** — 收到的消息：

| 字段 | 类型 | 说明 |
|------|------|------|
| `conversation_id` | `String` | 会话/用户唯一标识 |
| `text` | `String` | 文本内容（纯媒体消息时为空） |
| `media` | `Option<IncomingMedia>` | 附带的媒体文件 |

**`ChatResponse`** — 回复的消息：

| 字段 | 类型 | 说明 |
|------|------|------|
| `text` | `Option<String>` | 文本回复（支持 Markdown，SDK 自动转纯文本） |
| `media` | `Option<OutgoingMedia>` | 附带的媒体文件 |

### 媒体处理

SDK 自动处理媒体的下载、AES 解密、上传和加密，开发者无需关心底层细节。

**接收媒体** (`IncomingMedia`)：

| 字段 | 类型 | 说明 |
|------|------|------|
| `media_type` | `MediaType` | `Image` / `Audio` / `Video` / `File` |
| `file_path` | `String` | 解密后的本地文件路径 |
| `mime_type` | `String` | MIME 类型 |
| `file_name` | `Option<String>` | 原始文件名 |

**发送媒体** (`OutgoingMedia`)：

| 字段 | 类型 | 说明 |
|------|------|------|
| `media_type` | `OutgoingMediaType` | `Image` / `Video` / `File` |
| `url` | `String` | 文件下载 URL（SDK 会下载后加密上传） |
| `file_name` | `Option<String>` | 文件名 |

### 登录流程

```
login() → 获取二维码 → 终端显示 → 用户扫码 → 确认登录 → 凭证保存到本地
```

1. 调用 `login(LoginOptions)` 发起登录
2. SDK 在终端打印 QR 码
3. 用微信扫码并确认
4. 凭证自动保存到 `~/.openclaw/openclaw-weixin/`
5. 返回 `account_id`，后续传给 `start()` 使用

### 消息循环

```
start() → 加载凭证 → 长轮询 get_updates → 收到消息 → 发送 typing → agent.chat() → 发送回复
```

- 每条消息在独立的 tokio task 中处理，互不阻塞
- 超时和网络错误自动重试，连续 3 次失败后退避 30 秒
- 会话过期（`SessionExpired`）时循环终止，需要重新登录

## API 参考

### 公开类型

| 类型 | 说明 |
|------|------|
| `Agent` | 核心 trait，实现 `chat` 方法处理消息 |
| `ChatRequest` | 收到的聊天消息 |
| `ChatResponse` | 回复的聊天消息 |
| `IncomingMedia` | 接收到的媒体文件 |
| `OutgoingMedia` | 要发送的媒体文件 |
| `MediaType` | 接收媒体类型：`Image` / `Audio` / `Video` / `File` |
| `OutgoingMediaType` | 发送媒体类型：`Image` / `Video` / `File` |
| `LoginOptions` | 登录选项（可自定义 `base_url`） |
| `StartOptions` | 启动选项（可指定 `account_id`） |
| `Error` | 错误枚举 |
| `Result<T>` | `std::result::Result<T, Error>` 别名 |

### 公开函数

| 函数 | 签名 | 说明 |
|------|------|------|
| `login` | `async fn login(LoginOptions) -> Result<String>` | 扫码登录，返回 account_id |
| `start` | `async fn start(Arc<dyn Agent>, StartOptions) -> Result<()>` | 启动消息循环 |

### 错误处理

SDK 使用 [`snafu`](https://docs.rs/snafu) 进行错误管理，所有错误统一为 `Error` 枚举：

| 变体 | 说明 |
|------|------|
| `Http` | HTTP 请求失败 |
| `Json` | JSON 序列化/反序列化失败 |
| `Io` | 文件系统 I/O 失败 |
| `Api` | 微信 API 返回非零错误码 |
| `SessionExpired` | 会话过期，需重新登录 |
| `QrCodeExpired` | 二维码超时未扫描 |
| `LoginFailed` | 登录流程失败 |
| `NoAccount` | 未找到已保存的账号 |
| `Encryption` | AES 加解密失败 |

## 高级用法

### OpenAI 集成

以下示例展示如何接入 OpenAI 兼容的 API，实现一个带上下文记忆的聊天机器人：

```rust
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use base64::Engine;
use reqwest::Client;
use serde_json::{Value, json};
use snafu::ResultExt;
use wechat_agent_rs::{
    Agent, ChatRequest, ChatResponse, LoginOptions, StartOptions,
    errors::{HttpSnafu, IoSnafu},
    login, start,
};

struct OpenAIAgent {
    client:        Client,
    base_url:      String,       // OpenAI API 地址
    api_key:       String,       // API 密钥
    model:         String,       // 模型名称
    system_prompt: String,       // 系统提示词
    histories:     Mutex<HashMap<String, Vec<Value>>>, // 每个会话的聊天记录
}

impl OpenAIAgent {
    fn new() -> Self {
        Self {
            client:        Client::new(),
            // 支持自定义 Base URL，方便接入国内 API 代理
            base_url:      std::env::var("OPENAI_BASE_URL")
                .unwrap_or_else(|_| "https://api.openai.com/v1".into()),
            api_key:       std::env::var("OPENAI_API_KEY").expect("需要设置 OPENAI_API_KEY"),
            model:         std::env::var("OPENAI_MODEL").unwrap_or_else(|_| "gpt-4o".into()),
            system_prompt: std::env::var("SYSTEM_PROMPT")
                .unwrap_or_else(|_| "You are a helpful assistant.".into()),
            histories:     Mutex::new(HashMap::new()),
        }
    }
}

impl Agent for OpenAIAgent {
    fn chat(
        &self,
        request: ChatRequest,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = wechat_agent_rs::Result<ChatResponse>> + Send + '_>,
    > {
        Box::pin(async move {
            // 处理图片消息：转为 base64 发给 Vision 模型
            let user_content = if let Some(ref media) = request.media {
                match media.media_type {
                    wechat_agent_rs::MediaType::Image => {
                        let data = std::fs::read(&media.file_path).context(IoSnafu)?;
                        let b64 = base64::engine::general_purpose::STANDARD.encode(&data);
                        json!([
                            {"type": "text", "text": request.text},
                            {"type": "image_url", "image_url": {
                                "url": format!("data:{};base64,{b64}", media.mime_type)
                            }}
                        ])
                    }
                    _ => {
                        // 其他媒体类型附加文件信息
                        json!(format!(
                            "{}\n[附件: {} ({})]",
                            request.text,
                            media.file_name.as_deref().unwrap_or("file"),
                            media.mime_type
                        ))
                    }
                }
            } else {
                json!(request.text)
            };

            // 维护会话上下文（最多保留 50 条）
            let messages = {
                let mut histories = self.histories.lock().unwrap();
                let history = histories
                    .entry(request.conversation_id.clone())
                    .or_default();
                history.push(json!({"role": "user", "content": user_content}));
                if history.len() > 50 {
                    history.drain(0..history.len() - 50);
                }
                let mut messages = vec![
                    json!({"role": "system", "content": self.system_prompt})
                ];
                messages.extend(history.iter().cloned());
                messages
            };

            // 调用 OpenAI Chat Completions API
            let resp = self
                .client
                .post(format!("{}/chat/completions", self.base_url))
                .header("Authorization", format!("Bearer {}", self.api_key))
                .json(&json!({
                    "model": self.model,
                    "messages": messages,
                }))
                .send()
                .await
                .context(HttpSnafu)?
                .json::<Value>()
                .await
                .context(HttpSnafu)?;

            let reply = resp["choices"][0]["message"]["content"]
                .as_str()
                .unwrap_or("(无回复)")
                .to_string();

            // 保存助手回复到上下文
            self.histories
                .lock()
                .unwrap()
                .entry(request.conversation_id)
                .or_default()
                .push(json!({"role": "assistant", "content": &reply}));

            Ok(ChatResponse {
                text:  Some(reply),
                media: None,
            })
        })
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let account_id = match wechat_agent_rs::storage::get_account_ids() {
        Ok(ids) if !ids.is_empty() => ids[0].clone(),
        _ => login(LoginOptions::default()).await?,
    };

    let agent = Arc::new(OpenAIAgent::new());
    start(
        agent,
        StartOptions {
            account_id: Some(account_id),
        },
    )
    .await?;

    Ok(())
}
```

**环境变量：**

| 变量 | 必填 | 默认值 | 说明 |
|------|------|--------|------|
| `OPENAI_API_KEY` | 是 | - | API 密钥 |
| `OPENAI_BASE_URL` | 否 | `https://api.openai.com/v1` | API 地址（支持国内代理） |
| `OPENAI_MODEL` | 否 | `gpt-4o` | 模型名称 |
| `SYSTEM_PROMPT` | 否 | `You are a helpful assistant.` | 系统提示词 |

### 自定义 Base URL

```rust
let account_id = login(LoginOptions {
    base_url: Some("https://custom-url.example.com".into()),
}).await?;
```

## 项目结构

```
src/
  lib.rs       # 入口，re-export 公开 API
  api.rs       # HTTP 客户端，封装微信 iLink Bot API
  bot.rs       # 登录（login）和启动（start）编排
  errors.rs    # 错误类型定义（snafu）
  media.rs     # 媒体上传/下载，AES-128-ECB 加解密
  models.rs    # 数据模型：Agent trait、ChatRequest、ChatResponse 等
  runtime.rs   # 长轮询消息循环、消息处理、Markdown 转纯文本
  storage.rs   # 本地文件持久化（凭证、配置、轮询游标）
examples/
  echo_bot.rs    # 回声机器人示例
  openai_bot.rs  # OpenAI 聊天机器人示例
```

## 开发

```bash
just fmt        # 格式化代码
just lint       # 运行 clippy + doc warnings + cargo deny
just test       # 运行测试
just pre-commit # 提交前全量检查（格式 + lint + 测试）
```

## 许可证

[MIT](LICENSE)
# test ci trigger
