extern crate proc_macro;

use proc_macro::TokenStream;
use quote::{format_ident, quote, ToTokens};
use syn::ItemFn;

/// 简化类型字符串（仅处理 lite 模式需要的情况）
fn type_simplify(s: &str) -> String {
    s.replace(" :: ", "::")
        .replace(" <", "<")
        .replace("< ", "<")
        .replace(" >", ">")
        .replace("> ", ">")
}

/// potato-lite 模式下的简化 handler 宏
/// 仅生成基本的路由包装函数，不包含 inventory 注册、缓存、中间件等复杂功能
fn lite_http_handler_macro(attr: TokenStream, input: TokenStream, req_name: &str) -> TokenStream {
    let _req_name = req_name;

    // 解析函数
    let root_fn = syn::parse_macro_input!(input as ItemFn);
    let fn_name = root_fn.sig.ident.clone();
    let is_async = root_fn.sig.asyncness.is_some();
    let has_request_arg = root_fn.sig.inputs.iter().any(|arg| {
        if let syn::FnArg::Typed(arg) = arg {
            let ty_str = type_simplify(&arg.ty.to_token_stream().to_string());
            ty_str == "& mut HttpRequest"
        } else {
            false
        }
    });

    // 解析路由路径（从属性参数中获取）
    let route_path = {
        let attr_stream: proc_macro2::TokenStream = attr.into();
        let tokens: Vec<_> = attr_stream.into_iter().collect();
        let mut path = String::new();
        for token in &tokens {
            if let proc_macro2::TokenTree::Literal(lit) = token {
                let lit_str = lit.to_string();
                if lit_str.starts_with('"') && lit_str.ends_with('"') {
                    path = lit_str[1..lit_str.len() - 1].to_string();
                    break;
                }
            }
        }
        path
    };

    let wrap_func_name = format_ident!("__potato_lite_wrap_{}", fn_name);
    let route_info_name =
        format_ident!("__POTATO_LITE_ROUTE_{}", fn_name.to_string().to_uppercase());
    let route_path_static: &str = Box::leak(route_path.into_boxed_str());
    let http_method = _req_name.to_string();

    // 生成参数传递代码
    let call_expr = if has_request_arg {
        quote! { #fn_name(req) }
    } else {
        quote! { #fn_name() }
    };

    // 根据是否异步生成不同的 wrapper
    let wrapper_body = if is_async {
        // 对于 async 函数，使用单次 poll 策略（适用于立即完成的简单函数）
        quote! {
            use core::future::Future;
            use core::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
            const VT: RawWakerVTable = RawWakerVTable::new(
                |_| RawWaker::new(core::ptr::null(), &VT),
                |_| {},
                |_| {},
                |_| {},
            );
            let waker = unsafe { Waker::from_raw(RawWaker::new(core::ptr::null(), &VT)) };
            let mut cx = Context::from_waker(&waker);
            let mut fut = #call_expr;
            // SAFETY: fut 在此作用域内不会被移动
            let mut fut = unsafe { core::pin::Pin::new_unchecked(&mut fut) };
            match Future::poll(fut, &mut cx) {
                Poll::Ready(result) => Some(result),
                Poll::Pending => None,
            }
        }
    } else {
        quote! {
            Some(#call_expr)
        }
    };

    let output = quote! {
        #root_fn

        #[doc(hidden)]
        #[allow(unused_variables)]
        fn #wrap_func_name(req: &mut potato_lite::HttpRequest) -> Option<potato_lite::HttpResponse> {
            #wrapper_body
        }

        #[doc(hidden)]
        #[::linkme::distributed_slice(potato_lite::ROUTE_HANDLERS)]
        static #route_info_name: potato_lite::RouteHandler = potato_lite::RouteHandler {
            method: #http_method,
            path: #route_path_static,
            handler: #wrap_func_name,
        };
    };

    output.into()
}

#[proc_macro_attribute]
pub fn http_get(attr: TokenStream, input: TokenStream) -> TokenStream {
    lite_http_handler_macro(attr, input, "GET")
}

#[proc_macro_attribute]
pub fn http_post(attr: TokenStream, input: TokenStream) -> TokenStream {
    lite_http_handler_macro(attr, input, "POST")
}

#[proc_macro_attribute]
pub fn http_put(attr: TokenStream, input: TokenStream) -> TokenStream {
    lite_http_handler_macro(attr, input, "PUT")
}

#[proc_macro_attribute]
pub fn http_delete(attr: TokenStream, input: TokenStream) -> TokenStream {
    lite_http_handler_macro(attr, input, "DELETE")
}

#[proc_macro_attribute]
pub fn http_options(attr: TokenStream, input: TokenStream) -> TokenStream {
    lite_http_handler_macro(attr, input, "OPTIONS")
}

#[proc_macro_attribute]
pub fn http_head(attr: TokenStream, input: TokenStream) -> TokenStream {
    lite_http_handler_macro(attr, input, "HEAD")
}
