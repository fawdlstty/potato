# potato 设计说明

高性能、简洁的 HTTP 框架，支持 HTTP/1.1、HTTP/2、HTTP/3，同时提供客户端与服务端能力。（用户要求，见 README）

## 模块

- `potato/`：框架主体 crate。服务端（HttpServer：HTTP/1.1 keep-alive + 流水线 + writev 合批响应、HTTP/2、HTTP/3）、客户端（get! 等宏）、WebSocket、TLS、OpenAPI、WebDAV、SSH 反向代理、ACME、WebRTC/WebTransport、会话缓存、限流限速等能力，经 feature 开关裁剪（default = std/websocket/openapi/tls，full 全开）。（AI 演绎，依据 Cargo.toml features 与 examples）
- `potato-macro/`：过程宏 crate，提供 `#[http_get]` 等路由注解宏。（AI 演绎）
- `examples/`：按编号排列的服务端/客户端示例集。（AI 演绎）
- `docs/`：VitePress 文档站，发布至 potato.fawdlstty.com。（AI 演绎）
- `benchmark/qps_test/`：压测实验台（独立 workspace）。`serve2 <potato|raw> <端口>` 起 potato/raw 双变体（raw 为零解析零分配参考锚；potato 变体支持 `POTATO_PER_CORE=1` 每核拓扑、`POTATO_TEMPLATE=1` 静态模板登记），`serve_{ntex,hyper,axum,actix,may}` 五对手方（potato/raw 18080/18082，对手方 18083~18087），`bench2 [端口] [连接数] [秒数] [depth]` 压测（Content-Length 精确分帧校验），`run_bench2.sh <对手方> <depth> [对数]` 微 ABBA 配对采集（结果 /tmp/potato_vs/pairs.csv），`run_ab2.sh` / `run_ab_bin.sh` 新旧二进制 ABBA 对比，`run_strace_d1.sh` syscall 定量，`analyze_pairs.py` 脏对剔除汇总。构建 `cd benchmark/qps_test && cargo build --release`（**plain profile**；实测 LTO=fat 使 potato d8 -10%/d1 -5% 而 actix 无感，属系统性负优化处方，已弃用）；服务端 taskset -c 0-11、客户端 taskset -c 12-19。（AI 演绎）
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
- 读路径零拷贝零清零：请求读取经 `HttpStream::read_into_vec` 直读连接缓冲备用容量（省每请求 4KB 栈缓冲清零与中转 memcpy）；空缓冲跳过解析调用（httparse 对空输入必然 Partial，省一次 48 槽头数组初始化）；头区大小校验单点化（解析函数内部承担，连接循环顶同款重复检查删除）。（AI 演绎）
- d8 批合并 piece 池化：ResponseBatcher 序列化经 `HttpResponse::as_bytes_into`（与公开 `as_bytes` 同源同字节序，仅目标缓冲由调用方提供）写入 flush 后回收的复用缓冲，免每响应一次序列化堆分配；线上字节形态不变。曾试验的文本构造器 `Into<Cow<'static, str>>` 收紧（API break）实测仅 ~0.6%（噪声级）已还原为 `Into<String>`。（AI 演绎；注：分项增益 d8 +12.7%/d1 +3.9% 为 fat-profile 时代读数，2026-11-26 下午 plain 口径新旧 ABBA 复测工作树整体 vs 0.4.7 ≈持平，分项归一以 runtime.md 第零节为准）
- 每核 current-thread 拓扑（方案A）：`POTATO_PER_CORE=1` 启用 serve_http/serve_https/serve_http2 三入口的每核拓扑——N 个工作线程各持独立 current-thread runtime 与 SO_REUSEPORT 监听，连接终身单线程亲和；TLS 握手与 h2 连接处理落 worker 本地后台任务（accept 不被握手阻塞），h1/h2 共用 `handle_http2_connection` 入口；默认关闭走共享监听。停机语义：serve future 被丢弃时 watch 发送端释放，worker 随之退出（测试钉死）；全部 worker 绑定失败回 Err。同二进制 ABBA 实证 plain d1 +21.0%/d8 +6.1%，TLS d1 +32.6~36.1%/d8 +13.3~13.6%。边界：h3（QUIC/UDP）与 acme（动态 acceptor）仍走共享拓扑；accept 错误时 worker 线程退出；Windows 等无 REUSEPORT 平台自动回退。公开 API 零变化。（AI 演绎，待主人拍板是否默认化）
- 静态响应模板快路径（方案C）：`potato::register_static_template(path, method, body)` 登记后，命中门控（GET、keep-alive、无压缩协商、无请求体）的请求跳过管线分发/响应构造/序列化，直接写预构造字节（Date 按秒刷新）；未登记时原子开关短路零开销。同二进制 ABBA 实证 d1 +9.3%、d8 +33%，字节级零变化。仅适用「零参数 + &'static str 返回」路由；带 CORS/预处理/后处理管线项的服务不得登记（快路径不经过管线项）。（AI 演绎）
- 连接任务按 server 隔离与停机 drain：连接/握手任务集、活跃连接计数与 drain 通知随 `PipeContext`（server 实例）隔离（`ConnectionScope`），`drain_connections` 宽限等待后仅 abort 本 server 的任务——同进程多 server 并存时停机互不影响对方在途连接；连接配额（max_connections）随之按 server 各自计数。（用户拍板按 server 隔离，AI 演绎实现）
- d8 序列化直写化：`as_bytes_into` 的 8 处 `write!` 全部替换为常量前缀 `extend_from_slice` + `u64_ascii_str` 手写整数（新增 `push_header_line_bytes!`/`push_header_kv_bytes!` 两个 Vec<u8> 版宏），version 非 HTTP/1.1 保留 write! 通用分支；压缩分支字节形态由新增单测钉死。同码同窗 ABBA d8 +4.6%。（AI 演绎）
- d8 flush 零分配：ResponseBatcher 持久化 `slices: Vec<IoSlice>` 复用缓冲（clear+push 重建条目；条目生存期标 `'static` 仅为绕开自引用限制，SAFETY 注释逐条论证：仅 flush 调用体内读取、await 期间独占借用保证 pieces 无变动、repr(transparent) 布局不变）；`write_all_vectored_inner` 首轮 offset==0 直接 write_vectored 不重建，仅部分写续传才走重建。同码同窗 ABBA d8 +1.5%。（AI 演绎）
- 请求特判头解析期缓存：HttpRequest 增 5 个 crate 内私有缓存字段（Accept-Encoding/Connection/Content-Length/Transfer-Encoding/Expect），解析循环分派分支顺手填缓存，5 个 getter 先读缓存、None 回落原头表扫描；头表 last-wins 同步、Expect 多段合并/TE 多段/CL 溢出一律回落，公开 API 零变化。同码同窗 ABBA d1 +4.6%/d8 +1.8%。（AI 演绎）
- 先解析后 flush（内嵌冲刷点）：连接循环顶 `buf_holds_complete_request` 探测守卫删除，flush 时机内嵌进读阶段——`from_stream_impl_into` 增加 `Option<&mut ResponseBatcher>` 参数（公开入口传 None），解析 Partial 需阻塞读前、100-Continue 插写前、chunked/body 补读前各设冲刷点（`flush_pending_before_read`，批空零成本）；超时出口补冲刷；`buf_holds_complete_request` 函数保留供 fuzz 钉探测/真解析一致性。半截请求第一响应先冲出由 `test_pipeline_flush.rs` 钉死（死锁红线锚点）。同码同窗 ABBA d8 +3.5~3.7%/d1 持平。（AI 演绎）
- 量测环境（本机事实）：本机原生 Linux，127/8 环回 RTT ~32µs 可直测；实验台 `benchmark/qps_test/`（服务端 12 线程绑 0-11，客户端 bench2 8 线程绑 12-19，128 连接，depth=1/8 两档；TLS 支路 `serve2 potato-tls` + `bench2 BENCH_TLS=1` + `run_tls_ab.sh`；对手方 TLS 入口 `serve_actix tls` / `serve_ntex tls`（rustls 0.23 单栈，证书与 potato 共用 /tmp/potato_tls_bench/），TLS roster 采集 `run_tls_roster.sh`）。绝对 QPS 跨会话漂移大（桌面噪声分钟级断崖），新旧对比必须同会话双二进制 ABBA（`run_ab2.sh`），跨会话只可比率、raw/hyper 视作不动锚；跑分前先查 `pgrep cc1` 与 load1（他任务编译风暴会污染窗口），且清场脚本必须按二进制名逐个 pkill（serve2_047 孤儿污染实证）；晚间桌面负载窗口会系统性抬高 d1 比率（roster 须配 base 快照同窗判决实验区分漂移与回归，正本数字只在安静窗口采）；大改动去留必须同窗直接 ABBA 终审，阶梯级间读数不可作数（2026-11-26 先解析后 flush 项假读数反转案例）。WSL2 环境下 mirrored 模式劫持 127/8 环回（RTT ~600µs）不可用于 d1 差距测量，需绑 docker0 `172.17.0.1`（二进制读 `BENCH_BIND_IP`）。（AI 演绎）

## The Benchmarker 榜单接入（benchmark/web-frameworks/）

- 目的：让 potato 上 The Benchmarker 社区 HTTP 榜单（TechEmpower 停运后的接续选择）。（用户要求，2026-11-26）
- 内容：`benchmark/web-frameworks/` 为 the-benchmarker/web-frameworks 的克隆（浅克隆），新增 `rust/potato/` 参赛目录（config.yaml、Cargo.toml、src/main.rs），实现 v2 六路由契约（静态/动态路由、方法分发、JSON 序列化/反序列化、multipart 上传），监听 3000 端口。二进制必须命名 `server`（语言模板固定取 `target/release/server`）。（AI 演绎，规格依据仓库 SCENARIO.md 与 .spec/v2/*.feature）
- 契约要点：v2 为默认契约（config.yaml 省略 specs 即选 v2）；seed 参数与用户 ID 需按原始查询串/解码后校验（重复参数名、8 位 ASCII 数字、8 位小写字母）；媒体类型白名单外返回 415；上传要求识别重复 file 部件、filename="" 未选中输入与头残缺段。验收自测脚本 `benchmark/wfb_v2_check.py`，359 用例全过。（AI 演绎）
- 参赛实现依赖 potato 新增 API（随 0.4.7 发布，尚未发布）：`raw_query()` 原始查询串访问、multipart 元数据（`body_dup_files`/`body_unselected_files`/`body_malformed_multipart`）、`PipeContext::set_disable_compress` 响应压缩开关（v2 要求响应不得带 Content-Encoding）。Cargo.toml 暂写 `potato = "0.4.7"`，发布后可用。（AI 演绎）
- 修复 d1 重构回归：请求解析 SpecialHeader 分派重构时 Content-Length/Transfer-Encoding/Host 三头校验后未入库，导致 content-length 请求体被静默丢弃、chunked 永不识别；已补回三头入库（校验语义不变）。（AI 演绎发现并修复，2026-11-26）
- 上游流程：改动不提交，待主人确认后自行 commit/push 至 fawdlstty/web-frameworks 并向 the-benchmarker/web-frameworks develop 分支发 PR。（用户要求）

## 已知事实

- TechEmpower 官方已于 2026-03-24 宣布项目停运（Sunsetting），FrameworkBenchmarks 仓库归档只读，Round 23（2025-03）为最后一轮，官方榜单不再接受新框架与新分值；自托管 TFB 工具链（tfb --verify / --test）仍可用于自测。（AI 演绎，依据官方公告 issue #10932）
