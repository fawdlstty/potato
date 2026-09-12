#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

#[cfg(feature = "std")]
mod full;

#[cfg(feature = "std")]
pub use full::*;

// lite 在 std 下也参与编译：其 websocket 子模块（泛型 WebsocketIo 客户端）
// 与 block_on 执行器对单线程泵模型有用，经独立命名空间导出——lite 顶层
// 的 HttpMethod/HttpRequest 等与 full 同名，仍只在 no_std 下 glob 导出。
#[allow(dead_code, unused_imports)]
mod lite;

/// 轻量泛型 WebSocket 客户端（no_std 同源实现）：任何实现
/// [`wslite::WebsocketIo`] 的 socket（含同步 socket 包一层即时完成的
/// future）即可驱动，read/write/掩码/Ping 应答/分片重组全部内置。
#[cfg(feature = "websocket")]
pub mod wslite {
    pub use crate::lite::websocket::*;
}

#[cfg(feature = "std")]
pub use lite::block_on;

#[cfg(not(feature = "std"))]
pub use lite::*;

#[cfg(not(feature = "std"))]
pub use potato_macro::{
    http_delete, http_get, http_head, http_options, http_patch, http_post, http_put,
};
