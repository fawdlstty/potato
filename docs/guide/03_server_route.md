# 服务端路由

服务器端路由用于指定对目标请求地址采取什么举措，调用处理函数或者指定静态文件等，一个不匹配则调用下一个。默认服务端路由如下（不写即默认如此）

```rust
server.configure(|ctx| {
    ctx.use_handlers(false);
});
```

此处 `use_handlers` 函数即代表对于请求路径搜索匹配的处理函数，如果有，则转到对应处理函数。

路由除了上述处理函数外，还有下面几个：

## OpenAPI 文档

在configure函数里加入如下代码：

```rust
server.configure(|ctx| {
    // ...
    ctx.use_openapi("/doc/");
    // ...
});
```

其中路径指的是请求文档的地址。对于生产而言尽量不使用文档接口或改为复杂的路径，避免接口暴漏

## 本地目录路由

在configure函数里加入如下代码：

```rust
server.configure(|ctx| {
    // ...
    ctx.use_location_route("/", "/wwwroot", false);
    // ...
});
```

第一个参数为请求路径，第二个参数为本地目录地址，第三个参数用于控制软连接是否允许越过 `wwwroot`：

- `true`：允许 `wwwroot` 内软连接指向目录外部文件/目录
- `false`：不允许软连接越界，越界访问会被拦截

假如存在 `/wwwroot/a.json` 文件，那么通过请求 `/a.json` 即可访问此 json 文件。

该路由同时支持常见静态资源能力：

- 条件请求：`If-None-Match`、`If-Modified-Since`、`If-Match`、`If-Unmodified-Since`
- 分段下载：`Range` 与 `If-Range`

## 内建资源路由

在configure函数里加入如下代码：

```rust
server.configure(|ctx| {
    // ...
    ctx.use_embedded_route("/", embed_dir!("assets/wwwroot"));
    // ...
});
```

内建资源含义为编译期将 `embed_dir` 宏指定的目录内置进可执行程序，后续运行时可以不要求本地路径存在，也能提供相应的文件请求响应

## 内存泄露调试路由

此功能的实现机制是接管程序的内存分配动作，每次分配时记录内存分配位置，然后在dump的地方遍历所有未释放的内存，打印内存分配信息。启用potato库的jemalloc特性：

```shell
cargo add potato --features jemalloc
```

然后在configure函数里加入如下代码：

```rust
server.configure(|ctx| {
    // ...
    ctx.use_jemalloc("/profile.pdf");
    // ...
});
```

此方法要求函数的运行环境为linux，且安装完如下库：

```shell
# ubuntu/debian
sudo apt install libjemalloc-dev graphviz ghostscript
```

此后运行服务，请求 `/profile.pdf`，即可看到程序栈详细内存分配记录，如果存在内存泄露问题，找到报告里占比较大的函数重点排查

## 自定义路由

在configure函数里加入如下代码：

```rust
server.configure(|ctx| {
    // ...
    ctx.use_custom(|req| async { Some(HttpResponse::text("hello")) });
    ctx.use_custom_sync(|req| Some(HttpResponse::text("hello")));
    // ...
});
```

`use_custom` 函数允许您插入异步自定义请求处理逻辑；`use_custom_sync` 则用于同步自定义处理逻辑。

- 异步闭包：`ctx.use_custom(|req| async { ... })`
- 同步闭包：`ctx.use_custom_sync(|req| { ... })`

如果返回 `Some(response)`，则直接返回该响应，不再执行后续的中间件或处理器；如果返回 `None`，则继续执行后续的中间件或处理器。对于 `use_custom_sync`，框架会按同步方式直接调用，不会通过异步包装来模拟同步。

## 全局预处理和后处理

首先使用宏标注预处理和后处理函数：

```rust
#[potato::preprocess]
async fn my_preprocess(req: &mut HttpRequest) -> Option<HttpResponse> {
    // 预处理逻辑：请求到达时执行
    // 返回 Some(response) 可短路请求
    None
}

#[potato::postprocess]
async fn my_postprocess(req: &mut HttpRequest, res: &mut HttpResponse) {
    // 后处理逻辑：handler完成后执行
    res.add_header("X-Custom".into(), "value".into());
}
```

然后在configure函数里注册：

```rust
server.configure(|ctx| {
    // ...
    ctx.use_preprocess(my_preprocess);
    ctx.use_postprocess(my_postprocess);
    // ...
});
```

- `use_preprocess`：注册全局预处理函数，在所有路由处理之前执行。如果返回 `Some(response)`，则直接返回该响应，跳过后续所有处理。
- `use_postprocess`：注册全局后处理函数，在 handler 生成响应后执行，可以修改响应内容（如添加响应头）。

注意：预处理和后处理函数必须通过 `#[potato::preprocess]` 和 `#[potato::postprocess]` 宏标注。

## WebDAV 路由

启用potato库的webdav特性：

```shell
cargo add potato --features webdav
```

然后在configure函数里加入如下代码：

```rust
server.configure(|ctx| {
    // ...
    ctx.use_webdav_localfs("/webdav", "/tmp");
    // ctx.use_webdav_memfs("/webdav");
    // ...
});
```

## 反向代理路由

在configure函数里加入如下代码：

```rust
server.configure(|ctx| {
    // ...
    ctx.use_reverse_proxy("/", "http://www.fawdlstty.com", true);
    // ...
});
```

`use_reverse_proxy` 函数用于设置反向代理路由，它接受三个参数：
- 第一个参数 `url_path`：本地路径前缀，指定哪些请求路径需要被代理
- 第二个参数 `proxy_url`：目标代理服务器地址，请求将被转发到此地址
- 第三个参数 `modify_content`：布尔值，指定是否修改响应内容中的URL（将代理服务器地址替换为本地路径）

当设置 `modify_content` 为 `true` 时，响应内容中的代理服务器地址会被替换为本地路径，这对于处理静态资源中的硬编码URL非常有用。该功能支持WebSocket连接的代理，能够自动处理协议升级。
