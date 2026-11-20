# Cursor BYOK 功能与架构

Cursor BYOK 是一个本地模型网关。用户在桌面应用中配置模型服务，然后在 Cursor 的模型列表中选择这些模型。Rust 服务负责转换协议、调用模型和推进会话。Cursor 继续提供编辑器界面，并执行文件操作、终端命令和客户端 MCP 工具。

## 功能

| 功能 | 当前实现 |
| --- | --- |
| 模型管理 | 配置 API 地址、密钥、模型 ID、上下文窗口、推理参数和自定义请求头。支持发现模型、批量测试和排序。上游协议包括 OpenAI Responses、OpenAI Chat Completions 和 Anthropic Messages。 |
| Cursor Agent | 将本地模型与插件模型加入 Cursor 模型列表。BYOK 模型由本地运行时执行，官方模型的请求转发给 Cursor。保留多轮会话、工具调用、Rules、Skills 和 MCP 工作流。 |
| 编辑器辅助 | Tab 补全单独选路，可使用公共服务、官方直连或自定义服务。Git 提交消息可单独配置生成模型。 |
| 调用诊断 | 查看 Token 用量、缓存用量、首字延迟、生成速度和调用详情。启用详细日志后可保存原始请求与响应。 |
| 模型插件 | 插件提供 OAuth 登录、模型同步和推理适配。内置插件位于 `server/plugins/build-in/`。 |
| 外部模型 API | 启用后，其他客户端可用单独的 API Key 访问 `/byok/v1/models`、`/byok/v1/chat/completions`、`/byok/v1/responses` 和 `/byok/v1/messages`。 |

[使用说明](README-CN.md) · [桌面页面](apps/desktop/src/App.tsx) · [Cursor 模型列表](server/src/cursor/services/model_catalog.rs) · [管理 API](server/src/control/mod.rs) · [外部模型 API](server/src/api/byok/mod.rs)

## 目录结构

```text
cursor-byok/
├── apps/desktop/           # 管理应用
│   ├── src/               # React 页面，通过本地 HTTP 调用管理 API
│   └── src-tauri/         # 窗口、托盘、更新和进程内服务的生命周期
├── server/                # Rust 网关
│   ├── src/               # 服务实现
│   │   ├── app.rs         # 组装依赖、路由、网络监听和关停流程
│   │   ├── bin/           # 独立服务入口
│   │   ├── local_app/     # 本地 CA、HTTPS 代理和 Cursor 配置
│   │   ├── api/           # Cursor RPC 与外部模型 API 的 HTTP 入口
│   │   ├── control/       # 模型、插件、设置和统计的管理 API
│   │   ├── cursor/        # Cursor 请求编译、工具桥接和会话状态回传
│   │   ├── run/           # 模型调用与工具结果之间的执行循环
│   │   ├── model/         # 会话、消息、检查点和模型调用的内部结构
│   │   ├── provider/      # 上游协议适配、流式事件归一化和调用记录
│   │   ├── plugin/        # 模型插件、OAuth 和 Deno 工作进程
│   │   ├── search/        # 网页搜索、网页抓取和本地代码检索接入
│   │   └── store/         # SQLite 读写和按内容哈希保存 Blob
│   ├── migrations/        # 数据库表结构演进
│   ├── plugins/build-in/  # 随应用提供的模型插件
│   ├── prompt/            # Cursor 各模式的提示词模板
│   └── tests/             # 协议、会话、工具和外部 API 的集成测试
├── crates/semble-core/    # 本地代码索引和向量、关键词混合检索
├── protocols/cursor/      # Cursor Protobuf 协议定义
└── support/               # 协议提取、抓包调试和搜索基准
```

## 服务入口与数据流

Tauri 在桌面应用进程内启动 Axum 服务。独立的 `cursor-server` 二进制复用同一个 `App`，也可以提供管理页面。开启 Cursor 集成后，本地代理使用受信任的本地 CA，将指定的 Cursor 请求转交给 Axum。 [桌面入口](apps/desktop/src-tauri/src/desktop.rs) · [独立入口](server/src/bin/cursor-server.rs) · [服务组装](server/src/app.rs) · [Cursor 集成](server/src/local_app/mod.rs)

```text
桌面 WebView ── 管理 HTTP API ──► control/ ──► 配置、测试、统计

Cursor ── 本地 HTTPS 代理 ──► api/cursor/ ──┬─ 官方模型 ──► Cursor 官方服务
                                          └─ BYOK 模型 ──► cursor/ ──► run/
                                                                         │
外部客户端 ── /byok/v1 ────────► api/byok/ ────────────────────────────┐  │
                                                                     ▼  ▼
                                                                   provider/
                                                                     │
                                                                     ▼
                                                               上游模型 API / 插件
```

Cursor Agent 入口由本地运行时驱动工具循环。外部模型 API 复用模型配置和适配层。请求协议与上游配置匹配时可直接转发，否则先转换为内部模型请求。外部客户端自行处理工具调用。 [Cursor 路由](server/src/api/cursor/handlers.rs) · [外部接口转发](server/src/api/byok/mod.rs)

## Cursor Agent 的执行过程

```text
BidiAppend ──► 请求编译 ──► RunEngine ◄──► ProviderRouter ──► 上游模型
                 ▲              │   ▲
                 │              │   └──── 工具结果 ◄── Cursor / 服务端工具
                 │              ├──────── 工具调用 ──► Cursor / 服务端工具
                 │              │
               SQLite ◄─────────┤ 写入消息、检查点和执行结果
                                └──► Cursor 协议适配 ──► RunSSE
```

1. **选路。** `BidiAppend` 接收请求、用户输入和工具结果，`RunSSE` 返回文本、工具事件和会话状态。两者按 `request_id` 配对。首条请求的模型 ID 决定走本地 BYOK 还是 Cursor 官方服务，后续上行按序号处理。 [HTTP 入口](server/src/api/cursor/handlers.rs) · [传输注册表](server/src/cursor/transport/registry.rs)
2. **准备会话。** `cursor/compile/` 根据 Cursor 携带的会话状态恢复历史，并同步所需 Blob。它将当前输入、模式提示词、Rules 和动态 MCP 工具整理为内部执行请求。 [请求编译](server/src/cursor/compile/run.rs) · [检查点恢复](server/src/cursor/checkpoint/recovery.rs)
3. **调用模型。** `RunEngine` 读取本次执行的检查点，把历史交给 `ProviderRouter`。上游适配器负责请求序列化，并将文本、推理、工具调用和用量转换为统一的流式事件。Cursor 适配层将相应事件写回 `RunSSE`。 [执行循环](server/src/run/engine.rs) · [模型路由](server/src/provider/router.rs) · [Cursor 输出](server/src/cursor/conversation/output.rs)
4. **处理工具并继续。** 模型提出工具调用时，执行循环先记录工具轮次，再分派工具。文件、终端和客户端 MCP 的结果通过 `BidiAppend` 回传。Semble 在服务端执行，网页搜索和抓取可在 Cursor 确认后由服务端处理。结果写入检查点后，同一次执行可继续发起下一次模型调用。 [工具轮次](server/src/run/tool_round.rs) · [工具分派](server/src/cursor/tools/tool_call_dispatch/mod.rs) · [网页工具](server/src/cursor/tools/tool_call_dispatch/interaction.rs)
5. **结束本轮。** 运行时记录执行结果并向 Cursor 发布更新后的会话状态。当前传输可以短暂等待后续动作。收到新的执行请求时，运行时会处理旧执行的取消与收尾。 [运行时](server/src/cursor/conversation/runtime.rs) · [会话注册表](server/src/cursor/conversation/registry.rs)

| 生命周期 | 标识 | 持有的状态 |
| --- | --- | --- |
| 传输 | `request_id` | 上行序号、输出流和本地或官方路由。同一条传输可以承载后续执行。 |
| 会话 | `conversation_id` | 可跨轮次恢复的消息历史和当前检查点。 |
| 执行 | `run_id` | 一次 Agent 执行的模型调用、工具轮次和最终结果。工具回传后的下一次模型调用仍属于同一次执行。 |

## 持久化与排查入口

桌面版默认使用 `~/.cursor-byok-v3/cursor-byok.db`。SQLite 启用 WAL，保存模型配置和 API Key、会话消息、检查点、工具结果及调用记录。SQLite 中的 Blob 以内容哈希标识，可用于恢复 Cursor 会话。启用详细日志后，服务会记录原始调用载荷和 Cursor 协议跟踪。模型推理所需的上下文会发送给用户选定的上游。 [默认路径](server/src/config.rs) · [SQLite 初始化](server/src/store/sqlite.rs) · [模型配置表](server/migrations/0004_flatten_model_configuration.sql) · [调用记录](server/src/provider/recorder.rs)

常规轮次在已有历史后追加消息，尽量保持上游请求的历史前缀稳定。达到上下文预算时，运行时可压缩历史并更新检查点。模型协议问题从 `provider/` 查起，Cursor 协议与工具桥接从 `api/cursor/` 和 `cursor/` 查起，会话推进和工具结果提交则从 `run/` 和 `store/` 查起。 [历史映射](server/src/model/projection.rs) · [检查点存储](server/src/store/checkpoints.rs) · [上下文压缩](server/src/run/compaction.rs) · [前缀稳定性测试](server/tests/prefix_stability.rs)

## 构建与部署

`make build-desktop` 构建 Tauri 安装包，`make build-server` 构建独立服务，`make build-web` 构建管理界面。GitHub Release 工作流由 `v*` 标签触发。 [Makefile](Makefile) · [发布工作流](.github/workflows/release.yml)

在 `31e9f5c` 上，`Dockerfile` 的 Rust 阶段遗漏了 `crates/semble-core/`。按该阶段的 `COPY` 清单准备文件后，`cargo metadata` 在读取 `semble-core/Cargo.toml` 时失败，容器构建会在依赖解析阶段中断。 [Dockerfile](Dockerfile) · [Cargo 工作区](Cargo.toml)

Cursor 协议的详细上下行、工具回传和取消时序见 [Cursor Agent 链路](cursor.md)。
