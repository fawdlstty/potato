# potato 设计说明

高性能、简洁的 HTTP 框架，支持 HTTP/1.1、HTTP/2、HTTP/3，同时提供客户端与服务端能力。（用户要求，见 README）

## 模块

- `potato/`：框架主体 crate。服务端（HttpServer：HTTP/1.1 keep-alive + 流水线 + writev 合批响应、HTTP/2、HTTP/3）、客户端（get! 等宏）、WebSocket、TLS、OpenAPI、WebDAV、SSH 反向代理、ACME、WebRTC/WebTransport、会话缓存、限流限速等能力，经 feature 开关裁剪（default = std/websocket/openapi/tls，full 全开）。（AI 演绎，依据 Cargo.toml features 与 examples）
- `potato-macro/`：过程宏 crate，提供 `#[http_get]` 等路由注解宏。（AI 演绎）
- `examples/`：按编号排列的服务端/客户端示例集。（AI 演绎）
- `docs/`：VitePress 文档站，发布至 potato.fawdlstty.com。（AI 演绎）
- `benchmark/`：独立压测 crate（worker），对比 potato 与 ntex 等框架的 /hello 吞吐；`serve <框架> [端口]` 起服务，`bench [端口] [连接数] [每连接请求数] [路径] [期望响应体]` 压测。（AI 演绎，2026-11-25 扩展了 bench 的路径/期望体参数以支持 TFB 端点）
- `fuzz/`：cargo-fuzz 模糊测试 crate，覆盖 HTTP/1.1 自研分帧状态机（请求头解析、完整分帧含 chunked body、响应解析）。断言内存安全（ASan）、消耗量不越界、帧边界可复现，并对齐 hyper 的走私防御语义：TE+CL 并存、重复/冲突/非纯数字 CL、重复 TE、非纯 chunked TE、1.1 多 Host 一律拒绝（请求响应两侧同口径）；flush 探测 `buf_holds_complete_request` 与真解析一致性也在断言内。零运行时代价，产物只存在于 fuzz 构建。（用户要求 + AI 演绎，2026-11-25）

## 路径参数路由（/user/{id}）

- 路由模板里 `{name}` 段按 handler 同名参数的类型自动解析，支持 String/bool/全部数值类型（`#[potato::http_get("/user/{id}")] async fn user(id: i64)`）；参数必须占满整段，编译期校验模板合法性（参数名与函数参数一一对应、无重复、无残缺括号），类型不匹配请求返回 400。（用户要求 + AI 演绎）
- 匹配算法（对标 actix/axum 调研结论，效率优先）：静态路由走精确匹配表不变；动态路由 init 时一次性把模板拆成段存表（按段数分桶），请求期只做固定段 `==` 精确比较 + 参数段非空捕获，严禁正则（actix 动态路由编译正则是其最慢路径，130 路由 ~454µs vs matchit 2.45µs）。参数以原始路径字节区间存入 `req.url_path_params`（无 %xx 转义零拷贝），取值经 `req.path_param(name)`（String 参数返回 `Cow<str>`，仅含转义才分配解码；%2F 不切分段）。宏为每个路径参数生成专用提取代码（宏生成参数解析）。固定段精确命中优先于参数段（/user/admin 不被 /user/{id} 抢走）；HEAD 回退动态 GET；OPTIONS Allow 聚合动态方法；HTTP/1.1、HTTP/2、HTTP/3 共用同一分发。（AI 演绎，调研对标 actix-router/axum+matchit）
- 无宏提取的路由（controller 等）也可通过 `req.path_param(name)` 手动取值；OpenAPI 文档把路径参数渲染为 `in: path` required 参数。（AI 演绎）

## TechEmpower 测评接入（benchmark/techempower/）

- 目的：以 TechEmpower FrameworkBenchmarks（TFB）标准规格评测 potato。（用户要求，2026-11-25）
- 内容：该目录与 TFB 官方仓库 `frameworks/Rust/potato/` 一一对应，含 `benchmark_config.json`（json + plaintext 两个测试，database=None）、`potato.dockerfile`、TFB 应用（`/json`、`/plaintext` 端点）与 README。官方验证要求：/plaintext 响应体精确为 "Hello, World!"（text/plain），/json 响应体精确为 `{"message":"Hello, World!"}`（application/json），且支持 keep-alive、流水线与高并发；potato 默认响应头自带 Date/Server/Connection 满足要求。（AI 演绎，规格依据 TFB 官方源码 metadata.py 与 actix 模板）
- 只实现 json 与 plaintext 两类测试；数据库类测试（db/queries/fortunes/updates/cached-queries）未实现。（AI 演绎：potato 无数据库依赖）
- 本地验证方式：`benchmark/techempower` 下 `cargo run --release -- [端口]` 起服务，curl 检查响应头与响应体，`cd benchmark && cargo run --release -- bench` 压测；Git Bash 下需 `MSYS_NO_PATHCONV=1` 前缀避免路径参数被改写。（AI 演绎）
- 2026-11-25 本机参考分值（单机自压，128 连接 × 5000 请求）：plaintext ≈ 7.58 万 qps，json ≈ 7.72 万 qps。仅作参考，不代表 TFB 官方分值。（AI 演绎）

## d1 热路径性能（追平 actix/ntex）

- 目标与授权：ratio.md 显示 depth=1 无流水线下 potato 距 actix/ntex 有明显差距，主人下令追平并授权必要的 API break。（用户要求，2026-11-26）
- 头存储平铺化：请求/响应头表从 FxHashMap 迁移为平铺插入序向量（`potato/src/utils/header_store.rs` 的 RequestHeaders/ResponseHeaders，典型 2~10 项线性扫描优于哈希）；`HttpRequest::headers` / `HttpResponse::headers` 公开字段类型随之变更（API break），公开方法面同名同语义。响应头序列化顺序由哈希随机序变为确定的插入序（HTTP 头序无语义约束，fuzz 断言已同步）。（用户要求授权 break + AI 演绎实现）
- 响应标准头位掩码：ResponseHeaders 内置 10 个标准头槽位（std_mask/std_idx），Date/Server/Connection/Content-Type/Pragma/Cache-Control/Content-Encoding/Trailer/Content-Length/Transfer-Encoding 的 get/insert 掩码命中直达下标；调用点零改动，d1/d8 双受益。（AI 演绎）
- keep-alive 读超时懒重臂：每连接一根 Sleep，距上次重臂 ≥ SLACK（min(timeout/4, 1s)）才 reset；空闲关闭语义由「timeout 整」放宽为「timeout-SLACK ~ timeout」。Sleep 已触发时重臂条件必然成立，不会误杀活跃连接。（AI 演绎）
- 请求特判头判别值分派：解析期每头单次 HeaderItem 判别值分派，替代 Content-Length/Transfer-Encoding/Host/Expect 逐头最多 4 次大小写不敏感字符串比较；拒绝与合并语义不变。（AI 演绎）
- 量测环境（本机事实）：WSL2 mirrored 模式劫持 127/8 环回（RTT ~600µs），不可用于 d1 差距测量；压测统一绑 docker0 `172.17.0.1` 内核原生环回（二进制读 `BENCH_BIND_IP`）。配套脚本 `benchmark/{run_full_docker0,usreq,analyze_pairs}.sh`。（AI 演绎）

## The Benchmarker 榜单接入（benchmark/web-frameworks/）

- 目的：让 potato 上 The Benchmarker 社区 HTTP 榜单（TechEmpower 停运后的接续选择）。（用户要求，2026-11-26）
- 内容：`benchmark/web-frameworks/` 为 the-benchmarker/web-frameworks 的克隆（浅克隆），新增 `rust/potato/` 参赛目录（config.yaml、Cargo.toml、src/main.rs），实现 v2 六路由契约（静态/动态路由、方法分发、JSON 序列化/反序列化、multipart 上传），监听 3000 端口。二进制必须命名 `server`（语言模板固定取 `target/release/server`）。（AI 演绎，规格依据仓库 SCENARIO.md 与 .spec/v2/*.feature）
- 契约要点：v2 为默认契约（config.yaml 省略 specs 即选 v2）；seed 参数与用户 ID 需按原始查询串/解码后校验（重复参数名、8 位 ASCII 数字、8 位小写字母）；媒体类型白名单外返回 415；上传要求识别重复 file 部件、filename="" 未选中输入与头残缺段。验收自测脚本 `benchmark/wfb_v2_check.py`，359 用例全过。（AI 演绎）
- 参赛实现依赖 potato 新增 API（随 0.4.7 发布，尚未发布）：`raw_query()` 原始查询串访问、multipart 元数据（`body_dup_files`/`body_unselected_files`/`body_malformed_multipart`）、`PipeContext::set_disable_compress` 响应压缩开关（v2 要求响应不得带 Content-Encoding）。Cargo.toml 暂写 `potato = "0.4.7"`，发布后可用。（AI 演绎）
- 修复 d1 重构回归：请求解析 SpecialHeader 分派重构时 Content-Length/Transfer-Encoding/Host 三头校验后未入库，导致 content-length 请求体被静默丢弃、chunked 永不识别；已补回三头入库（校验语义不变）。（AI 演绎发现并修复，2026-11-26）
- 上游流程：改动不提交，待主人确认后自行 commit/push 至 fawdlstty/web-frameworks 并向 the-benchmarker/web-frameworks develop 分支发 PR。（用户要求）

## 已知事实

- TechEmpower 官方已于 2026-03-24 宣布项目停运（Sunsetting），FrameworkBenchmarks 仓库归档只读，Round 23（2025-03）为最后一轮，官方榜单不再接受新框架与新分值；自托管 TFB 工具链（tfb --verify / --test）仍可用于自测。（AI 演绎，依据官方公告 issue #10932）
