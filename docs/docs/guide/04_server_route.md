# 服务端路由

服务器端路由用于指定对目标请求地址采取什么举措，调用处理函数或者指定静态文件等，一个不匹配则调用下一个。默认服务端路由如下（不写即默认如此）

```rust
server.configure(|ctx| {
    ctx.use_handlers();
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

启用potato库的jemalloc特性：

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
