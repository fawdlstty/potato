#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc;

#[cfg(feature = "std")]
mod full;

#[cfg(feature = "std")]
pub use full::*;

#[cfg(not(feature = "std"))]
mod lite;

#[cfg(not(feature = "std"))]
pub use lite::*;

#[cfg(not(feature = "std"))]
pub use potato_macro::{
    http_delete, http_get, http_head, http_options, http_patch, http_post, http_put,
};
