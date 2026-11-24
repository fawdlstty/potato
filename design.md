# potato 设计说明

高性能、简洁的 HTTP 框架，支持 HTTP/1.1、HTTP/2、HTTP/3，同时提供客户端与服务端能力。（用户要求，见 README）

## 模块

- `potato/`：框架主体 crate。服务端（HttpServer：HTTP/1.1 keep-alive + 流水线 + writev 合批响应、HTTP/2、HTTP/3）、客户端（get! 等宏）、WebSocket、TLS、OpenAPI、WebDAV、SSH 反向代理、ACME、WebRTC/WebTransport、会话缓存、限流限速等能力，经 feature 开关裁剪（default = std/websocket/openapi/tls，full 全开）。（AI 演绎，依据 Cargo.toml features 与 examples）
- `potato-macro/`：过程宏 crate，提供 `#[http_get]` 等路由注解宏。（AI 演绎）
- `examples/`：按编号排列的服务端/客户端示例集。（AI 演绎）
- `docs/`：VitePress 文档站，发布至 potato.fawdlstty.com。（AI 演绎）
- `benchmark/`：独立压测 crate（worker），对比 potato 与 ntex 等框架的 /hello 吞吐；`serve <框架> [端口]` 起服务，`bench [端口] [连接数] [每连接请求数] [路径] [期望响应体]` 压测。（AI 演绎，2026-11-25 扩展了 bench 的路径/期望体参数以支持 TFB 端点）

## TechEmpower 测评接入（benchmark/techempower/）

- 目的：以 TechEmpower FrameworkBenchmarks（TFB）标准规格评测 potato。（用户要求，2026-11-25）
- 内容：该目录与 TFB 官方仓库 `frameworks/Rust/potato/` 一一对应，含 `benchmark_config.json`（json + plaintext 两个测试，database=None）、`potato.dockerfile`、TFB 应用（`/json`、`/plaintext` 端点）与 README。官方验证要求：/plaintext 响应体精确为 "Hello, World!"（text/plain），/json 响应体精确为 `{"message":"Hello, World!"}`（application/json），且支持 keep-alive、流水线与高并发；potato 默认响应头自带 Date/Server/Connection 满足要求。（AI 演绎，规格依据 TFB 官方源码 metadata.py 与 actix 模板）
- 只实现 json 与 plaintext 两类测试；数据库类测试（db/queries/fortunes/updates/cached-queries）未实现。（AI 演绎：potato 无数据库依赖）
- 本地验证方式：`benchmark/techempower` 下 `cargo run --release -- [端口]` 起服务，curl 检查响应头与响应体，`cd benchmark && cargo run --release -- bench` 压测；Git Bash 下需 `MSYS_NO_PATHCONV=1` 前缀避免路径参数被改写。（AI 演绎）
- 2026-11-25 本机参考分值（单机自压，128 连接 × 5000 请求）：plaintext ≈ 7.58 万 qps，json ≈ 7.72 万 qps。仅作参考，不代表 TFB 官方分值。（AI 演绎）

## 已知事实

- TechEmpower 官方已于 2026-03-24 宣布项目停运（Sunsetting），FrameworkBenchmarks 仓库归档只读，Round 23（2025-03）为最后一轮，官方榜单不再接受新框架与新分值；自托管 TFB 工具链（tfb --verify / --test）仍可用于自测。（AI 演绎，依据官方公告 issue #10932）
