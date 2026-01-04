# Server-Side Routing

Server-side routing is used to specify what actions to take for target request addresses, calling handler functions or specifying static files, etc. If one doesn't match, the next is tried. The default server-side routing is as follows (if not written, it defaults to this):

```rust
server.configure(|ctx| {
    ctx.use_handlers();
});
```

Here the `use_handlers` function represents searching for matching handler functions for the request path. If found, it redirects to the corresponding handler function.

In addition to the above handler functions, there are several other routes:

## OpenAPI Documentation

Add the following code in the configure function:

```rust
server.configure(|ctx| {
    // ...
    ctx.use_openapi("/doc/");
    // ...
});
```

The path refers to the address for requesting documentation. For production environments,尽量 avoid using documentation interfaces or change to complex paths to avoid exposing interfaces.

## Local Directory Routing

Add the following code in the configure function:

```rust
server.configure(|ctx| {
    // ...
    ctx.use_location_route("/", "/wwwroot");
    // ...
});
```

The first parameter is the request path, and the second parameter is the local directory address. If a file `/wwwroot/a.json` exists, it can be accessed via the request `/a.json`.

## Embedded Resource Routing

Add the following code in the configure function:

```rust
server.configure(|ctx| {
    // ...
    ctx.use_embedded_route("/", embed_dir!("assets/wwwroot"));
    // ...
});
```

Embedded resources mean that the directory specified by the `embed_dir` macro is built into the executable program at compile time. Subsequently, during runtime, it doesn't require the local path to exist and can still provide corresponding file request responses.

## Memory Leak Debugging Routing

The implementation mechanism of this feature is to take over the program's memory allocation actions, recording the memory allocation location each time it allocates. Then at the dump location, it traverses all unreleased memory and prints memory allocation information. Enable the jemalloc feature of the potato library:

```shell
cargo add potato --features jemalloc
```

Then add the following code in the configure function:

```rust
server.configure(|ctx| {
    // ...
    ctx.use_jemalloc("/profile.pdf");
    // ...
});
```

This method requires the function's runtime environment to be Linux, and the following libraries to be installed:

```shell
# ubuntu/debian
sudo apt install libjemalloc-dev graphviz ghostscript
```

After running the service, request `/profile.pdf` to see the detailed memory allocation records of the program stack. If memory leak issues exist, focus on examining the functions with larger reports for troubleshooting.

## Custom Routing

Add the following code in the configure function:

```rust
server.configure(|ctx| {
    // ...
    ctx.use_custom(|req| async { Some(HttpResponse::text("hello")) });
    // ...
});
```

The `use_custom` function allows you to insert custom request processing logic. It takes an asynchronous closure that receives a request and returns an optional response. If it returns Some(response), the response is returned directly without executing subsequent middleware or handlers; if it returns None, subsequent middleware or handlers continue to execute.

## WebDAV Routing

Enable the webdav feature of the potato library:

```shell
cargo add potato --features webdav
```

Then add the following code in the configure function:

```rust
server.configure(|ctx| {
    // ...
    ctx.use_webdav_localfs("/webdav", "/tmp");
    // ctx.use_webdav_memfs("/webdav");
    // ...
});
```