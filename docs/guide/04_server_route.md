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
    ctx.use_location_route("/", "/wwwroot");
    // ...
});
```

第一个参数为请求路径，第二个参数为本地目录地址。假如存在 `/wwwroot/a.json` 文件，那么通过请求 `/a.json` 即可访问此json文件

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
    // ...
});
```

`use_custom` 函数允许您插入自定义的请求处理逻辑，它接收一个异步闭包，该闭包接收请求并返回一个可选的响应。如果返回 Some(response)，则直接返回该响应，不再执行后续的中间件或处理器；如果返回 None，则继续执行后续的中间件或处理器。

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
