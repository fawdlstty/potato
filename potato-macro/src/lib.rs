mod utils;

use proc_macro::TokenStream;
use proc_macro2::{Ident, Span};
use quote::{format_ident, quote, ToTokens};
use rand::Rng;
use serde_json::json;
use std::{collections::HashSet, sync::LazyLock};
use syn::Token;
use utils::StringExt as _;

/// CORS配置结构体(宏内部使用)
struct CorsAttrConfig {
    origin: Option<String>,
    methods: Option<String>,
    headers: Option<String>,
    max_age: Option<String>,
    credentials: bool,
    expose_headers: Option<String>,
}

/// 解析CORS属性
fn parse_cors_attr(tokens: &proc_macro2::TokenStream) -> CorsAttrConfig {
    let config = CorsAttrConfig {
        origin: None,
        methods: None,
        headers: None,
        max_age: None,
        credentials: false,
        expose_headers: None,
    };

    if tokens.is_empty() {
        return config; // 返回最小限制配置(origin="*", headers="*", methods自动计算)
    }

    // 解析 key = value 格式
    use syn::parse::Parser;

    fn parse_inner(input: syn::parse::ParseStream) -> syn::Result<CorsAttrConfig> {
        let mut config = CorsAttrConfig {
            origin: None,
            methods: None,
            headers: None,
            max_age: None,
            credentials: false,
            expose_headers: None,
        };

        let vars =
            syn::punctuated::Punctuated::<syn::MetaNameValue, Token![,]>::parse_terminated(input)?;
        for meta in vars {
            let key = meta
                .path
                .get_ident()
                .map(|i| i.to_string())
                .unwrap_or_default();
            if let syn::Expr::Lit(expr_lit) = &meta.value {
                match &expr_lit.lit {
                    syn::Lit::Str(s) => {
                        let val = s.value();
                        match key.as_str() {
                            "origin" => config.origin = Some(val),
                            "methods" => config.methods = Some(val),
                            "headers" => config.headers = Some(val),
                            "max_age" => config.max_age = Some(val),
                            "expose_headers" => config.expose_headers = Some(val),
                            _ => {}
                        }
                    }
                    syn::Lit::Bool(b) => {
                        if key == "credentials" {
                            config.credentials = b.value();
                        }
                    }
                    _ => {}
                }
            }
        }
        Ok(config)
    }

    match parse_inner.parse2(tokens.clone()) {
        Ok(cfg) => cfg,
        Err(e) => panic!("Failed to parse cors attributes: {e}"),
    }
}

static ARG_TYPES: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    [
        "String", "bool", "u8", "u16", "u32", "u64", "usize", "i8", "i16", "i32", "i64", "isize",
        "f32", "f64",
    ]
    .into_iter()
    .collect()
});

/// Controller 字段类型
/// 验证 Controller 结构体字段
fn validate_controller_struct(item_struct: &syn::ItemStruct) -> (bool, bool) {
    let mut has_once_cache = false;
    let mut has_session_cache = false;

    if let syn::Fields::Named(fields_named) = &item_struct.fields {
        for field in &fields_named.named {
            let field_type_str = field.ty.to_token_stream().to_string().type_simplify();

            // 验证类型必须是 &OnceCache 或 &SessionCache（支持生命周期参数）
            if field_type_str.contains("OnceCache") {
                has_once_cache = true;
            } else if field_type_str.contains("SessionCache") {
                has_session_cache = true;
            } else {
                panic!(
                    "Controller field must be &OnceCache or &SessionCache, got: {}",
                    field_type_str
                );
            }
        }
    }

    (has_once_cache, has_session_cache)
}

/// 解析 header 标注的 tokens，返回 (key, value)
fn parse_header_attr(tokens: &proc_macro2::TokenStream) -> Result<(String, String), syn::Error> {
    use syn::parse::Parser;

    let parser = |input: syn::parse::ParseStream| {
        // 支持两种格式：
        // 1. Key = "value" (标准 header)
        // 2. Custom("key") = "value" (自定义 header)
        let key_ident: Ident = input.parse()?;
        let key_name = key_ident.to_string();

        if key_name == "Custom" {
            // Custom("key") = "value" 格式
            let content;
            syn::parenthesized!(content in input);
            let key_lit: syn::LitStr = content.parse()?;
            let key = key_lit.value();
            let _: Token![=] = input.parse()?;
            let value: syn::LitStr = input.parse()?;
            Ok((key, value.value()))
        } else {
            // Key = "value" 格式
            let _: Token![=] = input.parse()?;
            let value: syn::LitStr = input.parse()?;
            Ok((key_name, value.value()))
        }
    };

    parser.parse2(tokens.clone())
}

fn random_ident() -> Ident {
    let mut rng = rand::thread_rng();
    let value = format!("__potato_id_{}", rng.r#gen::<u64>());
    Ident::new(&value, Span::call_site())
}

fn attr_last_ident(attr: &syn::Attribute) -> Option<String> {
    attr.meta
        .path()
        .segments
        .iter()
        .last()
        .map(|segment| segment.ident.to_string())
}

fn parse_hook_attr_items(attr: &syn::Attribute, attr_name: &str) -> Vec<Ident> {
    let parser = syn::punctuated::Punctuated::<Ident, syn::Token![,]>::parse_terminated;
    let idents = attr.parse_args_with(parser).unwrap_or_else(|err| {
        panic!("invalid `{attr_name}` annotation: {err}");
    });
    if idents.is_empty() {
        panic!("`{attr_name}` annotation requires at least one function name");
    }
    idents.into_iter().collect()
}

fn collect_handler_hooks(root_fn: &mut syn::ItemFn) -> (Vec<Ident>, Vec<Ident>) {
    enum HookKind {
        Pre,
        Post,
    }
    let mut hooks = vec![];
    let mut new_attrs = Vec::with_capacity(root_fn.attrs.len());
    for attr in root_fn.attrs.iter() {
        match attr_last_ident(attr).as_deref() {
            Some("preprocess") => {
                hooks.extend(
                    parse_hook_attr_items(attr, "preprocess")
                        .into_iter()
                        .map(|item| (HookKind::Pre, item)),
                );
            }
            Some("postprocess") => {
                hooks.extend(
                    parse_hook_attr_items(attr, "postprocess")
                        .into_iter()
                        .map(|item| (HookKind::Post, item)),
                );
            }
            _ => new_attrs.push(attr.clone()),
        }
    }
    root_fn.attrs = new_attrs;
    let mut preprocess_fns = vec![];
    let mut postprocess_fns = vec![];
    for (kind, hook) in hooks.into_iter() {
        match kind {
            HookKind::Pre => preprocess_fns.push(hook),
            HookKind::Post => postprocess_fns.push(hook),
        }
    }
    (preprocess_fns, postprocess_fns)
}

fn validate_preprocess_signature(root_fn: &syn::ItemFn) -> (String, bool, bool) {
    if root_fn.sig.inputs.is_empty() || root_fn.sig.inputs.len() > 3 {
        panic!("`preprocess` function must accept one to three arguments");
    }
    let mut arg_types = vec![];
    for arg in root_fn.sig.inputs.iter() {
        match arg {
            syn::FnArg::Typed(arg) => {
                arg_types.push(arg.ty.to_token_stream().to_string().type_simplify())
            }
            _ => panic!("`preprocess` function does not support receiver argument"),
        }
    }
    if arg_types[0] != "& mut HttpRequest" {
        panic!(
            "`preprocess` first argument type must be `&mut potato::HttpRequest`, got `{}`",
            arg_types[0]
        );
    }

    let has_once_cache = arg_types.iter().any(|t| t == "& mut OnceCache");
    let has_session_cache = arg_types.iter().any(|t| t == "& mut SessionCache");

    if arg_types.len() == 2 && !has_once_cache && !has_session_cache {
        panic!(
            "`preprocess` second argument type must be `&mut potato::OnceCache` or `&mut potato::SessionCache`, got `{}`",
            arg_types[1]
        );
    }
    if arg_types.len() == 3 {
        if !has_once_cache {
            panic!("`preprocess` must have `&mut potato::OnceCache` as one of the arguments");
        }
        if !has_session_cache {
            panic!("`preprocess` must have `&mut potato::SessionCache` as one of the arguments");
        }
    }

    let ret_type = root_fn
        .sig
        .output
        .to_token_stream()
        .to_string()
        .type_simplify();
    match &ret_type[..] {
        "Result<Option<HttpResponse>>" | "Option<HttpResponse>" | "Result<()>" | "()" => {}
        _ => panic!(
            "unsupported `preprocess` return type: `{ret_type}`, expected `anyhow::Result<Option<potato::HttpResponse>>`, `Option<potato::HttpResponse>`, `anyhow::Result<()>`, or `()`"
        ),
    }
    (ret_type, has_once_cache, has_session_cache)
}

fn validate_postprocess_signature(root_fn: &syn::ItemFn) -> (String, bool, bool) {
    if root_fn.sig.inputs.len() < 2 && root_fn.sig.inputs.len() > 4 {
        panic!("`postprocess` function must accept two to four arguments");
    }
    let mut arg_types = vec![];
    for arg in root_fn.sig.inputs.iter() {
        match arg {
            syn::FnArg::Typed(arg) => {
                arg_types.push(arg.ty.to_token_stream().to_string().type_simplify())
            }
            _ => panic!("`postprocess` function does not support receiver argument"),
        }
    }
    if arg_types[0] != "& mut HttpRequest" {
        panic!(
            "`postprocess` first argument must be `&mut potato::HttpRequest`, got `{}`",
            arg_types[0]
        );
    }
    if arg_types[1] != "& mut HttpResponse" {
        panic!(
            "`postprocess` second argument must be `&mut potato::HttpResponse`, got `{}`",
            arg_types[1]
        );
    }

    let remaining_args = &arg_types[2..];
    let has_once_cache = remaining_args.iter().any(|t| t == "& mut OnceCache");
    let has_session_cache = remaining_args.iter().any(|t| t == "& mut SessionCache");

    if arg_types.len() == 3 && !has_once_cache && !has_session_cache {
        panic!(
            "`postprocess` third argument must be `&mut potato::OnceCache` or `&mut potato::SessionCache`, got `{}`",
            arg_types[2]
        );
    }
    if arg_types.len() == 4 && (!has_once_cache || !has_session_cache) {
        panic!(
            "`postprocess` with 4 arguments must have both `&mut potato::OnceCache` and `&mut potato::SessionCache`"
        );
    }

    let ret_type = root_fn
        .sig
        .output
        .to_token_stream()
        .to_string()
        .type_simplify();
    match &ret_type[..] {
        "Result<()>" | "()" => {}
        _ => panic!(
            "unsupported `postprocess` return type: `{ret_type}`, expected `anyhow::Result<()>` or `()`"
        ),
    }
    (ret_type, has_once_cache, has_session_cache)
}

fn preprocess_macro(attr: TokenStream, input: TokenStream) -> TokenStream {
    if !attr.is_empty() {
        return input;
    }
    let root_fn = syn::parse_macro_input!(input as syn::ItemFn);
    let fn_name = root_fn.sig.ident.clone();
    let wrap_name = format_ident!("__potato_preprocess_adapter_{}", fn_name);
    let wrap_name_inner = format_ident!("__potato_preprocess_adapter_inner_{}", fn_name);
    let is_async = root_fn.sig.asyncness.is_some();
    let (ret_type, has_once_cache, has_session_cache) = validate_preprocess_signature(&root_fn);

    // 根据是否需要缓存生成不同的函数签名
    let wrap_signature = match (has_once_cache, has_session_cache) {
        (true, true) => quote! {
            async fn #wrap_name_inner(
                req: &mut potato::HttpRequest,
                once_cache: &mut potato::OnceCache,
                session_cache: &mut potato::SessionCache,
            ) -> anyhow::Result<Option<potato::HttpResponse>>
        },
        (true, false) => quote! {
            async fn #wrap_name_inner(
                req: &mut potato::HttpRequest,
                once_cache: &mut potato::OnceCache,
            ) -> anyhow::Result<Option<potato::HttpResponse>>
        },
        (false, true) => quote! {
            async fn #wrap_name_inner(
                req: &mut potato::HttpRequest,
                session_cache: &mut potato::SessionCache,
            ) -> anyhow::Result<Option<potato::HttpResponse>>
        },
        (false, false) => quote! {
            async fn #wrap_name_inner(
                req: &mut potato::HttpRequest,
            ) -> anyhow::Result<Option<potato::HttpResponse>>
        },
    };

    // 根据实际使用情况调用函数
    let call_body = if is_async {
        match &ret_type[..] {
            "Result<Option<HttpResponse>>" => match (has_once_cache, has_session_cache) {
                (true, true) => {
                    quote! { #fn_name(req, once_cache, session_cache).await }
                }
                (true, false) => quote! { #fn_name(req, once_cache).await },
                (false, true) => quote! { #fn_name(req, session_cache).await },
                (false, false) => quote! { #fn_name(req).await },
            },
            "Option<HttpResponse>" => match (has_once_cache, has_session_cache) {
                (true, true) => quote! { Ok(#fn_name(req, once_cache, session_cache).await) },
                (true, false) => quote! { Ok(#fn_name(req, once_cache).await) },
                (false, true) => quote! { Ok(#fn_name(req, session_cache).await) },
                (false, false) => quote! { Ok(#fn_name(req).await) },
            },
            "Result<()>" => match (has_once_cache, has_session_cache) {
                (true, true) => {
                    quote! { #fn_name(req, once_cache, session_cache).await.map(|_| None) }
                }
                (true, false) => quote! { #fn_name(req, once_cache).await.map(|_| None) },
                (false, true) => quote! { #fn_name(req, session_cache).await.map(|_| None) },
                (false, false) => quote! { #fn_name(req).await.map(|_| None) },
            },
            "()" => match (has_once_cache, has_session_cache) {
                (true, true) => quote! { #fn_name(req, once_cache, session_cache).await; Ok(None) },
                (true, false) => quote! { #fn_name(req, once_cache).await; Ok(None) },
                (false, true) => quote! { #fn_name(req, session_cache).await; Ok(None) },
                (false, false) => quote! { #fn_name(req).await; Ok(None) },
            },
            _ => unreachable!(),
        }
    } else {
        match &ret_type[..] {
            "Result<Option<HttpResponse>>" => match (has_once_cache, has_session_cache) {
                (true, true) => quote! { #fn_name(req, once_cache, session_cache) },
                (true, false) => quote! { #fn_name(req, once_cache) },
                (false, true) => quote! { #fn_name(req, session_cache) },
                (false, false) => quote! { #fn_name(req) },
            },
            "Option<HttpResponse>" => match (has_once_cache, has_session_cache) {
                (true, true) => quote! { Ok(#fn_name(req, once_cache, session_cache)) },
                (true, false) => quote! { Ok(#fn_name(req, once_cache)) },
                (false, true) => quote! { Ok(#fn_name(req, session_cache)) },
                (false, false) => quote! { Ok(#fn_name(req)) },
            },
            "Result<()>" => match (has_once_cache, has_session_cache) {
                (true, true) => quote! { #fn_name(req, once_cache, session_cache).map(|_| None) },
                (true, false) => quote! { #fn_name(req, once_cache).map(|_| None) },
                (false, true) => quote! { #fn_name(req, session_cache).map(|_| None) },
                (false, false) => quote! { #fn_name(req).map(|_| None) },
            },
            "()" => match (has_once_cache, has_session_cache) {
                (true, true) => quote! { #fn_name(req, once_cache, session_cache); Ok(None) },
                (true, false) => quote! { #fn_name(req, once_cache); Ok(None) },
                (false, true) => quote! { #fn_name(req, session_cache); Ok(None) },
                (false, false) => quote! { #fn_name(req); Ok(None) },
            },
            _ => unreachable!(),
        }
    };

    // 生成wrapper函数，根据cache需求调用inner函数
    let wrapper_body = match (has_once_cache, has_session_cache) {
        (true, true) => quote! {
            #wrap_name_inner(
                req,
                once_cache.expect("OnceCache required but not provided"),
                session_cache.expect("SessionCache required but not provided"),
            ).await
        },
        (true, false) => quote! {
            #wrap_name_inner(
                req,
                once_cache.expect("OnceCache required but not provided"),
            ).await
        },
        (false, true) => quote! {
            #wrap_name_inner(
                req,
                session_cache.expect("SessionCache required but not provided"),
            ).await
        },
        (false, false) => quote! {
            #wrap_name_inner(req).await
        },
    };

    quote! {
        #root_fn

        #[doc(hidden)]
        #wrap_signature {
            #call_body
        }

        #[doc(hidden)]
        pub async fn #wrap_name(
            req: &mut potato::HttpRequest,
            once_cache: Option<&mut potato::OnceCache>,
            session_cache: Option<&mut potato::SessionCache>,
        ) -> anyhow::Result<Option<potato::HttpResponse>> {
            #wrapper_body
        }
    }
    .into()
}

fn postprocess_macro(attr: TokenStream, input: TokenStream) -> TokenStream {
    if !attr.is_empty() {
        return input;
    }
    let root_fn = syn::parse_macro_input!(input as syn::ItemFn);
    let fn_name = root_fn.sig.ident.clone();
    let wrap_name = format_ident!("__potato_postprocess_adapter_{}", fn_name);
    let wrap_name_inner = format_ident!("__potato_postprocess_adapter_inner_{}", fn_name);
    let is_async = root_fn.sig.asyncness.is_some();
    let (ret_type, has_once_cache, has_session_cache) = validate_postprocess_signature(&root_fn);

    // 根据是否需要缓存生成不同的函数签名
    let wrap_signature = match (has_once_cache, has_session_cache) {
        (true, true) => quote! {
            async fn #wrap_name_inner(
                req: &mut potato::HttpRequest,
                res: &mut potato::HttpResponse,
                once_cache: &mut potato::OnceCache,
                session_cache: &mut potato::SessionCache,
            ) -> anyhow::Result<()>
        },
        (true, false) => quote! {
            async fn #wrap_name_inner(
                req: &mut potato::HttpRequest,
                res: &mut potato::HttpResponse,
                once_cache: &mut potato::OnceCache,
            ) -> anyhow::Result<()>
        },
        (false, true) => quote! {
            async fn #wrap_name_inner(
                req: &mut potato::HttpRequest,
                res: &mut potato::HttpResponse,
                session_cache: &mut potato::SessionCache,
            ) -> anyhow::Result<()>
        },
        (false, false) => quote! {
            async fn #wrap_name_inner(
                req: &mut potato::HttpRequest,
                res: &mut potato::HttpResponse,
            ) -> anyhow::Result<()>
        },
    };

    // 根据实际使用情况调用函数
    let call_body = if is_async {
        match &ret_type[..] {
            "Result<()>" => {
                if has_once_cache && has_session_cache {
                    quote! {
                        #fn_name(req, res, once_cache, session_cache).await
                    }
                } else if has_once_cache {
                    quote! {
                        #fn_name(req, res, once_cache).await
                    }
                } else if has_session_cache {
                    quote! {
                        #fn_name(req, res, session_cache).await
                    }
                } else {
                    quote! {
                        #fn_name(req, res).await
                    }
                }
            }
            "()" => {
                if has_once_cache && has_session_cache {
                    quote! {
                        #fn_name(req, res, once_cache, session_cache).await;
                        Ok(())
                    }
                } else if has_once_cache {
                    quote! {
                        #fn_name(req, res, once_cache).await;
                        Ok(())
                    }
                } else if has_session_cache {
                    quote! {
                        #fn_name(req, res, session_cache).await;
                        Ok(())
                    }
                } else {
                    quote! {
                        #fn_name(req, res).await;
                        Ok(())
                    }
                }
            }
            _ => unreachable!(),
        }
    } else {
        match &ret_type[..] {
            "Result<()>" => {
                if has_once_cache && has_session_cache {
                    quote! {
                        #fn_name(req, res, once_cache, session_cache)
                    }
                } else if has_once_cache {
                    quote! {
                        #fn_name(req, res, once_cache)
                    }
                } else if has_session_cache {
                    quote! {
                        #fn_name(req, res, session_cache)
                    }
                } else {
                    quote! {
                        #fn_name(req, res)
                    }
                }
            }
            "()" => {
                if has_once_cache && has_session_cache {
                    quote! {
                        #fn_name(req, res, once_cache, session_cache);
                        Ok(())
                    }
                } else if has_once_cache {
                    quote! {
                        #fn_name(req, res, once_cache);
                        Ok(())
                    }
                } else if has_session_cache {
                    quote! {
                        #fn_name(req, res, session_cache);
                        Ok(())
                    }
                } else {
                    quote! {
                        #fn_name(req, res);
                        Ok(())
                    }
                }
            }
            _ => unreachable!(),
        }
    };

    // 生成wrapper函数，根据cache需求调用inner函数
    let wrapper_body = match (has_once_cache, has_session_cache) {
        (true, true) => quote! {
            #wrap_name_inner(
                req,
                res,
                once_cache.expect("OnceCache required but not provided"),
                session_cache.expect("SessionCache required but not provided"),
            ).await
        },
        (true, false) => quote! {
            #wrap_name_inner(
                req,
                res,
                once_cache.expect("OnceCache required but not provided"),
            ).await
        },
        (false, true) => quote! {
            #wrap_name_inner(
                req,
                res,
                session_cache.expect("SessionCache required but not provided"),
            ).await
        },
        (false, false) => quote! {
            #wrap_name_inner(req, res).await
        },
    };

    quote! {
        #root_fn

        #[doc(hidden)]
        #wrap_signature {
            #call_body
        }

        #[doc(hidden)]
        pub async fn #wrap_name(
            req: &mut potato::HttpRequest,
            res: &mut potato::HttpResponse,
            once_cache: Option<&mut potato::OnceCache>,
            session_cache: Option<&mut potato::SessionCache>,
        ) -> anyhow::Result<()> {
            #wrapper_body
        }
    }
    .into()
}

fn http_handler_macro(attr: TokenStream, input: TokenStream, req_name: &str) -> TokenStream {
    let req_name = Ident::new(req_name, Span::call_site());

    // 解析函数，检查是否有 receiver（&self / &mut self）
    let root_fn_for_check = syn::parse::<syn::ItemFn>(input.clone());
    let has_receiver = if let Ok(ref func) = root_fn_for_check {
        func.sig
            .inputs
            .iter()
            .any(|arg| matches!(arg, syn::FnArg::Receiver(_)))
    } else {
        false
    };

    let (route_path, default_headers) = {
        let mut oroute_path = syn::parse::<syn::LitStr>(attr.clone())
            .ok()
            .map(|path| path.value());
        let mut default_headers: Vec<(String, String)> = Vec::new();
        //
        if oroute_path.is_none() {
            let http_parser = syn::meta::parser(|meta| {
                if meta.path.is_ident("path") {
                    if let Ok(arg) = meta.value() {
                        if let Ok(route_path) = arg.parse::<syn::LitStr>() {
                            let route_path = route_path.value();
                            oroute_path = Some(route_path);
                        }
                    }
                    Ok(())
                } else if meta.path.is_ident("header") {
                    // 解析 header(key = value) 格式
                    let content;
                    syn::parenthesized!(content in meta.input);
                    let key: Ident = content.parse()?;
                    let _: syn::Token![=] = content.parse()?;
                    let value: syn::LitStr = content.parse()?;
                    default_headers.push((key.to_string(), value.value()));
                    Ok(())
                } else {
                    Err(meta.error("unsupported annotation property"))
                }
            });
            syn::parse_macro_input!(attr with http_parser);
        }

        // 如果没有提供 path 且有 receiver，可能是 controller 方法
        if oroute_path.is_none() && has_receiver {
            // 这将在后续代码中处理，先设置为空
        } else if oroute_path.is_none() {
            panic!("`path` argument is required for non-controller methods");
        }

        let route_path = oroute_path.unwrap_or_default();

        // 如果是 controller 方法，需要处理路径拼接
        let route_path = if has_receiver {
            if route_path.is_empty() {
                // 没有指定 path，使用 controller base path（稍后在生成的代码中读取常量）
                String::new()
            } else {
                // 指定了 path，需要拼接到 controller base path
                // 这里先标记，稍后在生成的代码中处理
                route_path
            }
        } else {
            if route_path.is_empty() {
                panic!("`path` argument is required for non-controller methods");
            }
            route_path
        };

        if !route_path.is_empty() && !route_path.starts_with('/') {
            panic!("route path must start with '/'");
        }
        (route_path, default_headers)
    };

    // 解析函数上的 #[potato::header(...)] 标注
    let mut root_fn = syn::parse_macro_input!(input as syn::ItemFn);
    let mut fn_headers: Vec<(String, String)> = Vec::new();
    let mut cors_config: Option<CorsAttrConfig> = None;
    let mut max_concurrency: Option<usize> = None;
    let mut remaining_attrs = Vec::new();

    for attr in root_fn.attrs.iter() {
        // 检查是否是 header 标注（支持 #[header(...)] 和 #[potato::header(...)] 两种形式）
        let is_header_attr = attr.path().is_ident("header")
            || (attr.path().segments.len() == 2
                && attr
                    .path()
                    .segments
                    .iter()
                    .next()
                    .map(|s| s.ident.to_string())
                    == Some("potato".to_string())
                && attr
                    .path()
                    .segments
                    .iter()
                    .last()
                    .map(|s| s.ident.to_string())
                    == Some("header".to_string()));

        if is_header_attr {
            if let syn::Meta::List(meta_list) = &attr.meta {
                // 解析 header(Cache_Control = "no-store, no-cache, max-age=0") 或 header(Custom("key") = "value")
                if let Ok((key, value)) = parse_header_attr(&meta_list.tokens) {
                    fn_headers.push((key, value));
                }
            }
            continue;
        }

        // 检查是否是 cors 标注
        let is_cors_attr = attr.path().is_ident("cors")
            || (attr.path().segments.len() == 2
                && attr
                    .path()
                    .segments
                    .iter()
                    .next()
                    .map(|s| s.ident.to_string())
                    == Some("potato".to_string())
                && attr
                    .path()
                    .segments
                    .iter()
                    .last()
                    .map(|s| s.ident.to_string())
                    == Some("cors".to_string()));

        if is_cors_attr {
            if let syn::Meta::List(meta_list) = &attr.meta {
                cors_config = Some(parse_cors_attr(&meta_list.tokens));
            } else {
                // 无参数时使用最小限制配置
                cors_config = Some(CorsAttrConfig {
                    origin: None,
                    methods: None,
                    headers: None,
                    max_age: None,
                    credentials: false,
                    expose_headers: None,
                });
            }
            continue;
        }

        // 检查是否是 max_concurrency 标注
        let is_max_concurrency_attr = attr.path().is_ident("max_concurrency")
            || (attr.path().segments.len() == 2
                && attr
                    .path()
                    .segments
                    .iter()
                    .next()
                    .map(|s| s.ident.to_string())
                    == Some("potato".to_string())
                && attr
                    .path()
                    .segments
                    .iter()
                    .last()
                    .map(|s| s.ident.to_string())
                    == Some("max_concurrency".to_string()));

        if is_max_concurrency_attr {
            if let syn::Meta::List(meta_list) = &attr.meta {
                let tokens = &meta_list.tokens;
                // 直接解析为数字
                if let Ok(lit_int) = syn::parse2::<syn::LitInt>(tokens.clone()) {
                    if let Ok(val) = lit_int.base10_parse::<usize>() {
                        if val == 0 {
                            panic!("max_concurrency must be greater than 0");
                        }
                        max_concurrency = Some(val);
                    } else {
                        panic!("invalid max_concurrency value");
                    }
                } else {
                    panic!(
                        "max_concurrency requires a numeric value, e.g., #[max_concurrency(10)]"
                    );
                }
            } else if let syn::Meta::NameValue(name_value) = &attr.meta {
                if let syn::Expr::Lit(expr_lit) = &name_value.value {
                    if let syn::Lit::Int(lit_int) = &expr_lit.lit {
                        if let Ok(val) = lit_int.base10_parse::<usize>() {
                            if val == 0 {
                                panic!("max_concurrency must be greater than 0");
                            }
                            max_concurrency = Some(val);
                        } else {
                            panic!("invalid max_concurrency value");
                        }
                    } else {
                        panic!("max_concurrency requires a numeric value");
                    }
                } else {
                    panic!("max_concurrency requires a numeric value");
                }
            } else {
                panic!("max_concurrency requires a numeric value, e.g., #[max_concurrency(10)]");
            }
            continue;
        }

        remaining_attrs.push(attr.clone());
    }

    // 合并默认headers和函数headers
    let mut all_headers = default_headers;
    all_headers.extend(fn_headers);

    root_fn.attrs = remaining_attrs;
    let (preprocess_fns, postprocess_fns) = collect_handler_hooks(&mut root_fn);

    // 检测handler自身是否需要缓存
    let handler_has_once_cache = root_fn.sig.inputs.iter().any(|arg| {
        if let syn::FnArg::Typed(arg) = arg {
            arg.ty.to_token_stream().to_string().type_simplify() == "& mut OnceCache"
        } else {
            false
        }
    });
    let handler_has_session_cache = root_fn.sig.inputs.iter().any(|arg| {
        if let syn::FnArg::Typed(arg) = arg {
            arg.ty.to_token_stream().to_string().type_simplify() == "& mut SessionCache"
        } else {
            false
        }
    });

    // 修复：只有当 handler 本身需要缓存时，才设置 need_session_cache 和 need_once_cache
    // preprocess/postprocess 钩子如果需要缓存，它们可以通过参数声明
    // 但如果 handler 不需要缓存，我们不应该强制要求 Authorization header
    // 这样可以避免给不需要认证的 handler 添加不必要的认证要求
    let need_once_cache = handler_has_once_cache;
    let need_session_cache = handler_has_session_cache;

    let preprocess_adapters: Vec<Ident> = preprocess_fns
        .iter()
        .map(|name| format_ident!("__potato_preprocess_adapter_{}", name))
        .collect();
    let postprocess_adapters: Vec<Ident> = postprocess_fns
        .iter()
        .map(|name| format_ident!("__potato_postprocess_adapter_{}", name))
        .collect();
    let doc_show = {
        let mut doc_show = true;
        for attr in root_fn.attrs.iter() {
            if attr.meta.path().get_ident().map(|p| p.to_string()) == Some("doc".to_string()) {
                if let Ok(meta_list) = attr.meta.require_list() {
                    if meta_list.tokens.to_string() == "hidden" {
                        doc_show = false;
                        break;
                    }
                }
            }
        }
        doc_show
    };
    let doc_auth = need_session_cache;
    let doc_summary = {
        let mut docs = vec![];
        for attr in root_fn.attrs.iter() {
            if let Ok(attr) = attr.meta.require_name_value() {
                if attr.path.get_ident().map(|p| p.to_string()) == Some("doc".to_string()) {
                    let mut doc = attr.value.to_token_stream().to_string();
                    if doc.starts_with('\"') {
                        doc.remove(0);
                        doc.pop();
                    }
                    docs.push(doc);
                }
            }
        }
        if docs.iter().all(|d| d.starts_with(' ')) {
            for doc in docs.iter_mut() {
                doc.remove(0);
            }
        }
        docs.join("\n")
    };
    let doc_desp = "";
    let fn_name = root_fn.sig.ident.clone();
    let is_async = root_fn.sig.asyncness.is_some();

    // 检测是否有 receiver（&self / &mut self）- controller 方法
    let has_receiver = root_fn
        .sig
        .inputs
        .iter()
        .any(|arg| matches!(arg, syn::FnArg::Receiver(_)));

    // 生成最终路径（如果是 controller 方法，需要拼接）
    // 注意：由于路由注册需要编译期常量，路径拼接必须在宏展开时完成
    // 但 controller 宏和 http_get 宏是独立展开的，无法直接共享信息
    // 因此这里采用简化方案：直接使用 route_path，路径拼接由用户保证正确
    let final_path = if has_receiver {
        // Controller 方法：如果 route_path 为空，说明用户希望使用 controller 的 base path
        // 但这里无法获取 base path，所以要求用户必须指定完整路径或相对路径
        if route_path.is_empty() {
            // 暂时使用一个占位符，实际应该在 controller 宏中处理
            // 这里我们先要求用户必须提供路径
            panic!("Controller methods must specify a path (e.g., #[potato::http_get(\"/\")])");
        } else {
            route_path
        }
    } else {
        if route_path.is_empty() {
            panic!("`path` argument is required for non-controller methods");
        }
        route_path
    };

    let final_path_expr = quote! { #final_path };

    // 生成 tag 表达式
    let tag_expr = if has_receiver {
        quote! { __POTATO_CONTROLLER_NAME }
    } else {
        quote! { "" }
    };

    let wrap_func_name = random_ident();
    let mut args = vec![];
    let mut arg_names = vec![];
    let mut arg_types = vec![];
    let mut doc_args = vec![];
    for arg in root_fn.sig.inputs.iter() {
        // 支持 receiver 参数（&self / &mut self）- controller 方法
        if let syn::FnArg::Receiver(_receiver) = arg {
            // 跳过 receiver，不生成参数绑定代码
            // controller 实例将在包装函数中创建
            continue;
        }

        if let syn::FnArg::Typed(arg) = arg {
            let arg_type_str = arg
                .ty
                .as_ref()
                .to_token_stream()
                .to_string()
                .type_simplify();
            let arg_name_str = arg.pat.to_token_stream().to_string();
            let arg_value = match &arg_type_str[..] {
                "& mut HttpRequest" => quote! { req },
                "& mut OnceCache" => {
                    quote! { __potato_once_cache.as_mut().expect("OnceCache not available") }
                }
                "& mut SessionCache" => {
                    quote! { __potato_session_cache.as_mut().expect("SessionCache not available") }
                }
                "PostFile" => {
                    doc_args.push(json!({ "name": arg_name_str, "type": arg_type_str }));
                    quote! {
                        match req.body_files.get(&potato::utils::refstr::LocalHipStr<'static>::from_str(#arg_name_str)).cloned() {
                            Some(file) => file,
                            None => return potato::HttpResponse::error(format!("miss arg: {}", #arg_name_str)),
                        }
                    }
                }
                arg_type_str if ARG_TYPES.contains(arg_type_str) => {
                    doc_args.push(json!({ "name": arg_name_str, "type": arg_type_str }));
                    let mut arg_value = quote! {
                        match req.body_pairs
                            .get(&potato::hipstr::LocalHipStr::from(#arg_name_str))
                            .map(|p| p.to_string()) {
                            Some(val) => val,
                            None => match req.url_query
                                .get(&potato::hipstr::LocalHipStr::from(#arg_name_str))
                                .map(|p| p.as_str().to_string()) {
                                Some(val) => val,
                                None => return potato::HttpResponse::error(format!("miss arg: {}", #arg_name_str)),
                            },
                        }
                    };
                    if arg_type_str != "String" {
                        arg_value = quote! {
                            match #arg_value.parse() {
                                Ok(val) => val,
                                Err(err) => return potato::HttpResponse::error(format!("arg[{}] is not {} type", #arg_name_str, #arg_type_str)),
                            }
                        }
                    }
                    arg_value
                }
                _ => panic!("unsupported arg type: [{arg_type_str}]"),
            };
            args.push(arg_value);
            arg_names.push(random_ident());
            // 保存参数类型信息，用于后续生成 call_expr
            arg_types.push(arg_type_str);
        }
    }
    let wrap_func_name2 = random_ident();
    let ret_type = root_fn
        .sig
        .output
        .to_token_stream()
        .to_string()
        .type_simplify();

    // 如果有 receiver，需要生成 __potato_create_controller 函数
    // 通过检查是否存在 controller 常量来判断
    let _controller_create_fn = if has_receiver {
        quote! {
            // 这个函数应该由 controller 宏生成，这里只是引用
            // 如果编译出错，说明没有正确使用 #[potato::controller]
        }
    } else {
        quote! {}
    };

    // 为每个参数生成调用代码
    let call_args: Vec<_> = args
        .iter()
        .enumerate()
        .map(|(i, _arg)| {
            let arg_name = &arg_names[i];
            let arg_type = &arg_types[i];
            // 对于 HttpRequest，直接使用 req，不要通过中间变量
            if arg_type == "& mut HttpRequest" {
                quote! { req }
            } else {
                quote! { #arg_name }
            }
        })
        .collect();

    let call_expr = if has_receiver {
        // Controller 方法：直接调用方法（暂不支持字段注入）
        // 注意：当前版本不支持 controller 字段，方法应该是静态方法
        // 如果要支持字段，需要在包装函数中实例化 controller
        match args.len() {
            0 => quote! { #fn_name() },
            1 => {
                let arg_name = &arg_names[0];
                let arg = &args[0];
                let arg_type = &arg_types[0];
                if arg_type == "& mut HttpRequest" {
                    quote! { #fn_name(req) }
                } else {
                    quote! {{
                        let #arg_name = #arg;
                        #fn_name(#arg_name)
                    }}
                }
            }
            _ => {
                let let_bindings: Vec<_> = arg_types
                    .iter()
                    .zip(arg_names.iter())
                    .zip(args.iter())
                    .filter(|((arg_type, _), _)| *arg_type != "& mut HttpRequest")
                    .map(|((_, arg_name), arg)| quote! { let #arg_name = #arg; })
                    .collect();

                quote! {{
                    #(#let_bindings)*
                    #fn_name(#(#call_args),*)
                }}
            }
        }
    } else {
        // 普通方法：直接调用函数
        match args.len() {
            0 => quote! { #fn_name() },
            1 => {
                let arg_name = &arg_names[0];
                let arg = &args[0];
                let arg_type = &arg_types[0];
                // 检查是否是 HttpRequest 类型
                if arg_type == "& mut HttpRequest" {
                    quote! { #fn_name(req) }
                } else {
                    quote! {{
                        let #arg_name = #arg;
                        #fn_name(#arg_name)
                    }}
                }
            }
            _ => {
                // 只为非 HttpRequest 类型的参数创建中间变量
                let let_bindings: Vec<_> = arg_types
                    .iter()
                    .zip(arg_names.iter())
                    .zip(args.iter())
                    .filter(|((arg_type, _), _)| *arg_type != "& mut HttpRequest")
                    .map(|((_, arg_name), arg)| quote! { let #arg_name = #arg; })
                    .collect();

                quote! {{
                    #(#let_bindings)*
                    #fn_name(#(#call_args),*)
                }}
            }
        }
    };
    let handler_wrap_func_body = if is_async {
        match &ret_type[..] {
            "Result<()>" => quote! {
                match #call_expr.await {
                    Ok(_) => Ok(potato::HttpResponse::text("ok")),
                    Err(err) => Err(err),
                }
            },
            "Result<HttpResponse>" | "anyhow::Result<HttpResponse>" => quote! {
                match #call_expr.await {
                    Ok(ret) => Ok(ret),
                    Err(err) => Err(err),
                }
            },
            "Result<String>" | "anyhow::Result<String>" => quote! {
                match #call_expr.await {
                    Ok(ret) => Ok(potato::HttpResponse::html(ret)),
                    Err(err) => Err(err),
                }
            },
            "Result<& 'static str>" | "anyhow::Result<& 'static str>" => quote! {
                match #call_expr.await {
                    Ok(ret) => Ok(potato::HttpResponse::html(ret)),
                    Err(err) => Err(err),
                }
            },
            "()" => quote! {
                #call_expr.await;
                Ok(potato::HttpResponse::text("ok"))
            },
            "HttpResponse" => quote! {
                Ok(#call_expr.await)
            },
            "String" => quote! {
                Ok(potato::HttpResponse::html(#call_expr.await))
            },
            "& 'static str" => quote! {
                Ok(potato::HttpResponse::html(#call_expr.await))
            },
            _ => panic!("unsupported ret type: {ret_type}"),
        }
    } else {
        match &ret_type[..] {
            "Result<()>" => quote! {
                match #call_expr {
                    Ok(_) => Ok(potato::HttpResponse::text("ok")),
                    Err(err) => Err(err),
                }
            },
            "Result<HttpResponse>" | "anyhow::Result<HttpResponse>" => quote! {
                match #call_expr {
                    Ok(ret) => Ok(ret),
                    Err(err) => Err(err),
                }
            },
            "Result<String>" | "anyhow::Result<String>" => quote! {
                match #call_expr {
                    Ok(ret) => Ok(potato::HttpResponse::html(ret)),
                    Err(err) => Err(err),
                }
            },
            "Result<& 'static str>" | "anyhow::Result<& 'static str>" => quote! {
                match #call_expr {
                    Ok(ret) => Ok(potato::HttpResponse::html(ret)),
                    Err(err) => Err(err),
                }
            },
            "()" => quote! {
                #call_expr;
                Ok(potato::HttpResponse::text("ok"))
            },
            "HttpResponse" => quote! {
                Ok(#call_expr)
            },
            "String" => quote! {
                Ok(potato::HttpResponse::html(#call_expr))
            },
            "& 'static str" => quote! {
                Ok(potato::HttpResponse::html(#call_expr))
            },
            _ => panic!("unsupported ret type: {ret_type}"),
        }
    };
    let doc_args = serde_json::to_string(&doc_args).unwrap();

    // 生成添加headers的代码
    let add_headers_code = if all_headers.is_empty() {
        quote! {}
    } else {
        let header_statements = all_headers.iter().map(|(key, value)| {
            // 将下划线转换为HTTP标准命名 (例如 Cache_Control -> Cache-Control)
            let http_key = key.replace("_", "-");
            quote! {
                __potato_response.add_header(
                    std::borrow::Cow::Borrowed(#http_key),
                    std::borrow::Cow::Borrowed(#value)
                );
            }
        });
        quote! {
            #(#header_statements)*
        }
    };

    // 如果存在CORS配置,生成CORS headers注入代码
    let cors_headers_code = if let Some(cors) = &cors_config {
        let mut statements = vec![];

        // origin: 默认"*"
        let origin_val = cors.origin.as_deref().unwrap_or("*");
        statements.push(quote! {
            __potato_response.add_header(
                "Access-Control-Allow-Origin".into(),
                #origin_val.into()
            );
        });

        // methods: 仅在用户指定时添加,否则由OPTIONS请求自动计算
        if let Some(ref methods) = cors.methods {
            let mut methods_list: Vec<&str> = methods.split(',').map(|s| s.trim()).collect();
            if !methods_list.contains(&"HEAD") {
                methods_list.push("HEAD");
            }
            if !methods_list.contains(&"OPTIONS") {
                methods_list.push("OPTIONS");
            }
            let methods_str = methods_list.join(",");
            statements.push(quote! {
                __potato_response.add_header(
                    "Access-Control-Allow-Methods".into(),
                    #methods_str.into()
                );
            });
        }

        // headers: 默认"*"
        let headers_val = cors.headers.as_deref().unwrap_or("*");
        statements.push(quote! {
            __potato_response.add_header(
                "Access-Control-Allow-Headers".into(),
                #headers_val.into()
            );
        });

        // max_age: 默认"86400"
        if let Some(ref max_age) = cors.max_age {
            statements.push(quote! {
                __potato_response.add_header(
                    "Access-Control-Max-Age".into(),
                    #max_age.into()
                );
            });
        } else {
            statements.push(quote! {
                __potato_response.add_header(
                    "Access-Control-Max-Age".into(),
                    "86400".into()
                );
            });
        }

        if cors.credentials {
            statements.push(quote! {
                __potato_response.add_header(
                    "Access-Control-Allow-Credentials".into(),
                    "true".into()
                );
            });
        }

        if let Some(ref expose_headers) = cors.expose_headers {
            statements.push(quote! {
                __potato_response.add_header(
                    "Access-Control-Expose-Headers".into(),
                    #expose_headers.into()
                );
            });
        }

        quote! { #(#statements)* }
    } else {
        quote! {}
    };

    // 如果存在CORS配置且是PUT/POST/DELETE,自动生成HEAD handler
    let auto_head_handler = if cors_config.is_some()
        && (req_name == "POST" || req_name == "PUT" || req_name == "DELETE")
    {
        let head_wrap_name = format_ident!("__potato_cors_head_{}", fn_name);
        Some(quote! {
            #[doc(hidden)]
            fn #head_wrap_name(req: &mut potato::HttpRequest) -> potato::HttpResponse {
                // HEAD请求直接返回空响应,不执行原handler
                // CORS headers会通过postprocess机制自动添加
                potato::HttpResponse::html("")
            }
        })
    } else {
        None
    };

    // 如果指定了max_concurrency,生成静态信号量
    let semaphore_static = if let Some(max_conn) = max_concurrency {
        let semaphore_name =
            format_ident!("__POTATO_SEMAPHORE_{}", fn_name.to_string().to_uppercase());
        Some(quote! {
            #[doc(hidden)]
            #[allow(non_upper_case_globals)]
            static #semaphore_name: std::sync::LazyLock<tokio::sync::Semaphore> =
                std::sync::LazyLock::new(|| tokio::sync::Semaphore::new(#max_conn));
        })
    } else {
        None
    };

    let wrap_func_body = if is_async {
        if max_concurrency.is_some() {
            let semaphore_name =
                format_ident!("__POTATO_SEMAPHORE_{}", fn_name.to_string().to_uppercase());
            quote! {
                let __potato_permit = #semaphore_name.acquire().await;

                // 获取自定义错误处理器
                let __potato_error_handler: Option<potato::ErrorHandler> = {
                    let mut handler = None;
                    for flag in potato::inventory::iter::<potato::ErrorHandlerFlag> {
                        handler = Some(flag.handler.clone());
                        break;
                    }
                    handler
                };

                // 按需创建缓存对象
                let mut __potato_once_cache: Option<potato::OnceCache> = if #need_once_cache {
                    Some(potato::OnceCache::new())
                } else {
                    None
                };
                let mut __potato_session_cache: Option<potato::SessionCache> = if #need_session_cache {
                    // 从 Authorization header 中提取 Bearer token 并加载 session
                    if let Some(h) = req.headers.get(&potato::utils::refstr::HeaderOrHipStr::from_str("Authorization")) {
                        let header_value = h.as_str();
                        if header_value.starts_with("Bearer ") {
                            potato::SessionCache::from_token(&header_value[7..]).await.ok()
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                };

                // 如果 handler 需要 SessionCache 但没有提供 Authorization header，返回 401
                if #need_session_cache && __potato_session_cache.is_none() {
                    let mut __potato_resp = potato::HttpResponse::text("Unauthorized: Missing or invalid Authorization header");
                    __potato_resp.http_code = 401;
                    return __potato_resp;
                }

                // 自动解析请求中的Cookie
                if let Some(ref mut session_cache) = __potato_session_cache {
                    if let Some(cookie_header) = req.headers.get(&potato::utils::refstr::HeaderOrHipStr::from_str("Cookie")) {
                        session_cache.parse_request_cookies(cookie_header.as_str());
                    }
                }

                let mut __potato_pre_response: Option<potato::HttpResponse> = None;
                #(
                    if __potato_pre_response.is_none() {
                        __potato_pre_response = match #preprocess_adapters(
                            req,
                            __potato_once_cache.as_mut(),
                            __potato_session_cache.as_mut(),
                        ).await {
                            Ok(Some(ret)) => Some(ret),
                            Ok(None) => None,
                            Err(err) => {
                                let handler = &__potato_error_handler;
                                Some(match handler {
                                    Some(potato::ErrorHandler::Async(h)) => h(req, err).await,
                                    Some(potato::ErrorHandler::Sync(h)) => h(req, err),
                                    None => potato::HttpResponse::error(format!("{err:?}")),
                                })
                            }
                        };
                    }
                )*

                let mut __potato_response = match __potato_pre_response {
                    Some(ret) => ret,
                    None => match #handler_wrap_func_body {
                        Ok(resp) => resp,
                        Err(err) => {
                            let handler = &__potato_error_handler;
                            match handler {
                                Some(potato::ErrorHandler::Async(h)) => h(req, err).await,
                                Some(potato::ErrorHandler::Sync(h)) => h(req, err),
                                None => potato::HttpResponse::error(format!("{err:?}")),
                            }
                        }
                    },
                };

                #(
                    if let Err(err) = #postprocess_adapters(
                        req,
                        &mut __potato_response,
                        __potato_once_cache.as_mut(),
                        __potato_session_cache.as_mut(),
                    ).await {
                        drop(__potato_permit);
                        let handler = &__potato_error_handler;
                        return match handler {
                            Some(potato::ErrorHandler::Async(h)) => h(req, err).await,
                            Some(potato::ErrorHandler::Sync(h)) => h(req, err),
                            None => potato::HttpResponse::error(format!("{err:?}")),
                        };
                    }
                )*

                #add_headers_code
                #cors_headers_code

                // 自动应用SessionCache中的cookies到响应
                if let Some(ref session_cache) = __potato_session_cache {
                    session_cache.apply_cookies(&mut __potato_response);
                }

                drop(__potato_permit);
                __potato_response
            }
        } else {
            quote! {
                // 获取自定义错误处理器
                let __potato_error_handler: Option<potato::ErrorHandler> = {
                    let mut handler = None;
                    for flag in potato::inventory::iter::<potato::ErrorHandlerFlag> {
                        handler = Some(flag.handler.clone());
                        break;
                    }
                    handler
                };

                // 按需创建缓存对象
                let mut __potato_once_cache: Option<potato::OnceCache> = if #need_once_cache {
                    Some(potato::OnceCache::new())
                } else {
                    None
                };
                let mut __potato_session_cache: Option<potato::SessionCache> = if #need_session_cache {
                    // 从 Authorization header 中提取 Bearer token 并加载 session
                    if let Some(h) = req.headers.get(&potato::utils::refstr::HeaderOrHipStr::from_str("Authorization")) {
                        let header_value = h.as_str();
                        if header_value.starts_with("Bearer ") {
                            potato::SessionCache::from_token(&header_value[7..]).await.ok()
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                };

                // 如果 handler 需要 SessionCache 但没有提供 Authorization header，返回 401
                if #need_session_cache && __potato_session_cache.is_none() {
                    let mut __potato_resp = potato::HttpResponse::text("Unauthorized: Missing or invalid Authorization header");
                    __potato_resp.http_code = 401;
                    return __potato_resp;
                }

                // 自动解析请求中的Cookie
                if let Some(ref mut session_cache) = __potato_session_cache {
                    if let Some(cookie_header) = req.headers.get(&potato::utils::refstr::HeaderOrHipStr::from_str("Cookie")) {
                        session_cache.parse_request_cookies(cookie_header.as_str());
                    }
                }

                let mut __potato_pre_response: Option<potato::HttpResponse> = None;
                #(
                    if __potato_pre_response.is_none() {
                        __potato_pre_response = match #preprocess_adapters(
                            req,
                            __potato_once_cache.as_mut(),
                            __potato_session_cache.as_mut(),
                        ).await {
                            Ok(Some(ret)) => Some(ret),
                            Ok(None) => None,
                            Err(err) => {
                                let handler = &__potato_error_handler;
                                Some(match handler {
                                    Some(potato::ErrorHandler::Async(h)) => h(req, err).await,
                                    Some(potato::ErrorHandler::Sync(h)) => h(req, err),
                                    None => potato::HttpResponse::error(format!("{err:?}")),
                                })
                            }
                        };
                    }
                )*

                let mut __potato_response = match __potato_pre_response {
                    Some(ret) => ret,
                    None => match #handler_wrap_func_body {
                        Ok(resp) => resp,
                        Err(err) => {
                            let handler = &__potato_error_handler;
                            match handler {
                                Some(potato::ErrorHandler::Async(h)) => h(req, err).await,
                                Some(potato::ErrorHandler::Sync(h)) => h(req, err),
                                None => potato::HttpResponse::error(format!("{err:?}")),
                            }
                        }
                    },
                };

                #(
                    if let Err(err) = #postprocess_adapters(
                        req,
                        &mut __potato_response,
                        __potato_once_cache.as_mut(),
                        __potato_session_cache.as_mut(),
                    ).await {
                        let handler = &__potato_error_handler;
                        return match handler {
                            Some(potato::ErrorHandler::Async(h)) => h(req, err).await,
                            Some(potato::ErrorHandler::Sync(h)) => h(req, err),
                            None => potato::HttpResponse::error(format!("{err:?}")),
                        };
                    }
                )*

                #add_headers_code
                #cors_headers_code

                __potato_response
            }
        }
    } else {
        if max_concurrency.is_some() {
            let semaphore_name =
                format_ident!("__POTATO_SEMAPHORE_{}", fn_name.to_string().to_uppercase());
            quote! {
                let __potato_permit = #semaphore_name.acquire().await;

                // 获取自定义错误处理器
                let __potato_error_handler: Option<potato::ErrorHandler> = {
                    let mut handler = None;
                    for flag in potato::inventory::iter::<potato::ErrorHandlerFlag> {
                        handler = Some(flag.handler.clone());
                        break;
                    }
                    handler
                };

                // 按需创建缓存对象
                let mut __potato_once_cache: Option<potato::OnceCache> = if #need_once_cache {
                    Some(potato::OnceCache::new())
                } else {
                    None
                };
                let mut __potato_session_cache: Option<potato::SessionCache> = if #need_session_cache {
                    // 从 Authorization header 中提取 Bearer token 并加载 session
                    if let Some(h) = req.headers.get(&potato::utils::refstr::HeaderOrHipStr::from_str("Authorization")) {
                        let header_value = h.as_str();
                        if header_value.starts_with("Bearer ") {
                            potato::SessionCache::from_token(&header_value[7..]).await.ok()
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                };

                // 如果 handler 需要 SessionCache 但没有提供 Authorization header，返回 401
                if #need_session_cache && __potato_session_cache.is_none() {
                    let mut __potato_resp = potato::HttpResponse::text("Unauthorized: Missing or invalid Authorization header");
                    __potato_resp.http_code = 401;
                    return __potato_resp;
                }

                // 自动解析请求中的Cookie
                if let Some(ref mut session_cache) = __potato_session_cache {
                    if let Some(cookie_header) = req.headers.get(&potato::utils::refstr::HeaderOrHipStr::from_str("Cookie")) {
                        session_cache.parse_request_cookies(cookie_header.as_str());
                    }
                }

                let mut __potato_pre_response: Option<potato::HttpResponse> = None;
                #(
                    if __potato_pre_response.is_none() {
                        __potato_pre_response = match #preprocess_adapters(
                            req,
                            __potato_once_cache.as_mut(),
                            __potato_session_cache.as_mut(),
                        ).await {
                            Ok(Some(ret)) => Some(ret),
                            Ok(None) => None,
                            Err(err) => {
                                let handler = &__potato_error_handler;
                                Some(match handler {
                                    Some(potato::ErrorHandler::Async(h)) => h(req, err).await,
                                    Some(potato::ErrorHandler::Sync(h)) => h(req, err),
                                    None => potato::HttpResponse::error(format!("{err:?}")),
                                })
                            }
                        };
                    }
                )*

                let mut __potato_response = match __potato_pre_response {
                    Some(ret) => ret,
                    None => match #handler_wrap_func_body {
                        Ok(resp) => resp,
                        Err(err) => {
                            let handler = &__potato_error_handler;
                            match handler {
                                Some(potato::ErrorHandler::Async(h)) => h(req, err).await,
                                Some(potato::ErrorHandler::Sync(h)) => h(req, err),
                                None => potato::HttpResponse::error(format!("{err:?}")),
                            }
                        }
                    },
                };

                #(
                    if let Err(err) = #postprocess_adapters(
                        req,
                        &mut __potato_response,
                        __potato_once_cache.as_mut(),
                        __potato_session_cache.as_mut(),
                    ).await {
                        drop(__potato_permit);
                        let handler = &__potato_error_handler;
                        return match handler {
                            Some(potato::ErrorHandler::Async(h)) => h(req, err).await,
                            Some(potato::ErrorHandler::Sync(h)) => h(req, err),
                            None => potato::HttpResponse::error(format!("{err:?}")),
                        };
                    }
                )*

                #add_headers_code
                #cors_headers_code

                // 自动应用SessionCache中的cookies到响应
                if let Some(ref session_cache) = __potato_session_cache {
                    session_cache.apply_cookies(&mut __potato_response);
                }

                drop(__potato_permit);
                __potato_response
            }
        } else {
            quote! {
                // 获取自定义错误处理器
                let __potato_error_handler: Option<potato::ErrorHandler> = {
                    let mut handler = None;
                    for flag in potato::inventory::iter::<potato::ErrorHandlerFlag> {
                        handler = Some(flag.handler.clone());
                        break;
                    }
                    handler
                };

                // 按需创建缓存对象
                let mut __potato_once_cache: Option<potato::OnceCache> = if #need_once_cache {
                    Some(potato::OnceCache::new())
                } else {
                    None
                };
                let mut __potato_session_cache: Option<potato::SessionCache> = if #need_session_cache {
                    // 从 Authorization header 中提取 Bearer token 并加载 session
                    if let Some(h) = req.headers.get(&potato::utils::refstr::HeaderOrHipStr::from_str("Authorization")) {
                        let header_value = h.as_str();
                        if header_value.starts_with("Bearer ") {
                            potato::SessionCache::from_token(&header_value[7..]).await.ok()
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                };

                // 如果 handler 需要 SessionCache 但没有提供 Authorization header，返回 401
                if #need_session_cache && __potato_session_cache.is_none() {
                    let mut __potato_resp = potato::HttpResponse::text("Unauthorized: Missing or invalid Authorization header");
                    __potato_resp.http_code = 401;
                    return __potato_resp;
                }

                // 自动解析请求中的Cookie
                if let Some(ref mut session_cache) = __potato_session_cache {
                    if let Some(cookie_header) = req.headers.get(&potato::utils::refstr::HeaderOrHipStr::from_str("Cookie")) {
                        session_cache.parse_request_cookies(cookie_header.as_str());
                    }
                }

                let mut __potato_pre_response: Option<potato::HttpResponse> = None;
                #(
                    if __potato_pre_response.is_none() {
                        __potato_pre_response = match #preprocess_adapters(
                            req,
                            __potato_once_cache.as_mut(),
                            __potato_session_cache.as_mut(),
                        ).await {
                            Ok(Some(ret)) => Some(ret),
                            Ok(None) => None,
                            Err(err) => {
                                let handler = &__potato_error_handler;
                                Some(match handler {
                                    Some(potato::ErrorHandler::Async(h)) => h(req, err).await,
                                    Some(potato::ErrorHandler::Sync(h)) => h(req, err),
                                    None => potato::HttpResponse::error(format!("{err:?}")),
                                })
                            }
                        };
                    }
                )*

                let mut __potato_response = match __potato_pre_response {
                    Some(ret) => ret,
                    None => match #handler_wrap_func_body {
                        Ok(resp) => resp,
                        Err(err) => {
                            let handler = &__potato_error_handler;
                            match handler {
                                Some(potato::ErrorHandler::Async(h)) => h(req, err).await,
                                Some(potato::ErrorHandler::Sync(h)) => h(req, err),
                                None => potato::HttpResponse::error(format!("{err:?}")),
                            }
                        }
                    },
                };

                #(
                    if let Err(err) = #postprocess_adapters(
                        req,
                        &mut __potato_response,
                        __potato_once_cache.as_mut(),
                        __potato_session_cache.as_mut(),
                    ).await {
                        let handler = &__potato_error_handler;
                        return match handler {
                            Some(potato::ErrorHandler::Async(h)) => h(req, err).await,
                            Some(potato::ErrorHandler::Sync(h)) => h(req, err),
                            None => potato::HttpResponse::error(format!("{err:?}")),
                        };
                    }
                )*

                #add_headers_code
                #cors_headers_code

                // 自动应用SessionCache中的cookies到响应
                if let Some(ref session_cache) = __potato_session_cache {
                    session_cache.apply_cookies(&mut __potato_response);
                }

                __potato_response
            }
        }
    };

    if is_async {
        quote! {
            #root_fn

            #auto_head_handler

            #semaphore_static

            #[doc(hidden)]
            async fn #wrap_func_name2(req: &mut potato::HttpRequest) -> potato::HttpResponse {
                #wrap_func_body
            }

            #[doc(hidden)]
            fn #wrap_func_name(req: &mut potato::HttpRequest) -> std::pin::Pin<Box<dyn std::future::Future<Output = potato::HttpResponse> + Send + '_>> {
                Box::pin(#wrap_func_name2(req))
            }

            potato::inventory::submit!{potato::RequestHandlerFlag::new(
                potato::HttpMethod::#req_name,
                #final_path_expr,
                potato::HttpHandler::Async(#wrap_func_name),
                potato::RequestHandlerFlagDoc::new(#doc_show, #doc_auth, #doc_summary, #doc_desp, #doc_args, #tag_expr)
            )}
        }
        .into()
    } else {
        quote! {
            #root_fn

            #auto_head_handler

            #semaphore_static

            #[doc(hidden)]
            async fn #wrap_func_name2(req: &mut potato::HttpRequest) -> potato::HttpResponse {
                #wrap_func_body
            }

            #[doc(hidden)]
            fn #wrap_func_name(req: &mut potato::HttpRequest) -> std::pin::Pin<Box<dyn std::future::Future<Output = potato::HttpResponse> + Send + '_>> {
                Box::pin(#wrap_func_name2(req))
            }

            potato::inventory::submit!{potato::RequestHandlerFlag::new(
                potato::HttpMethod::#req_name,
                #final_path_expr,
                potato::HttpHandler::Async(#wrap_func_name),
                potato::RequestHandlerFlagDoc::new(#doc_show, #doc_auth, #doc_summary, #doc_desp, #doc_args, #tag_expr)
            )}
        }
        .into()
    }
    //}.to_string();
    //panic!("{content}");
    //todo!()
}

#[proc_macro_attribute]
pub fn http_get(attr: TokenStream, input: TokenStream) -> TokenStream {
    http_handler_macro(attr, input, "GET")
}

#[proc_macro_attribute]
pub fn http_post(attr: TokenStream, input: TokenStream) -> TokenStream {
    http_handler_macro(attr, input, "POST")
}

#[proc_macro_attribute]
pub fn http_put(attr: TokenStream, input: TokenStream) -> TokenStream {
    http_handler_macro(attr, input, "PUT")
}

#[proc_macro_attribute]
pub fn http_delete(attr: TokenStream, input: TokenStream) -> TokenStream {
    http_handler_macro(attr, input, "DELETE")
}

#[proc_macro_attribute]
pub fn http_options(attr: TokenStream, input: TokenStream) -> TokenStream {
    http_handler_macro(attr, input, "OPTIONS")
}

#[proc_macro_attribute]
pub fn http_head(attr: TokenStream, input: TokenStream) -> TokenStream {
    http_handler_macro(attr, input, "HEAD")
}

/// Controller 属性宏 - 定义控制器结构体
///
/// # 功能
/// - 为结构体的 impl 块中的所有方法提供统一的路由前缀
/// - 支持 preprocess/postprocess 中间件继承
/// - 自动为 Swagger 文档分组（tag 为结构体名称）
///
/// # 结构体字段限制
/// 只能包含以下类型的字段（0个或多个）：
/// - `&'a potato::OnceCache`
/// - `&'a potato::SessionCache`
///
/// # 示例
/// ```rust,ignore
/// #[potato::controller("/api/users")]
/// pub struct UsersController<'a> {
///     pub once_cache: &'a potato::OnceCache,
///     pub sess_cache: &'a potato::SessionCache,
/// }
///
/// #[potato::preprocess(my_preprocess)]
/// impl<'a> UsersController<'a> {
///     #[potato::http_get] // 地址为 "/api/users"
///     pub async fn get(&self) -> anyhow::Result<&'static str> {
///         Ok("get users data")
///     }
///
///     #[potato::http_get("/any")] // 地址为 "/api/users/any"
///     pub async fn get_any(&self) -> anyhow::Result<&'static str> {
///         Ok("get any data")
///     }
/// }
/// ```
#[proc_macro_attribute]
pub fn controller(attr: TokenStream, input: TokenStream) -> TokenStream {
    controller_macro(attr, input)
}

fn controller_macro(attr: TokenStream, input: TokenStream) -> TokenStream {
    // 尝试解析为 impl 块
    let input_clone = input.clone();
    if let Ok(item_impl) = syn::parse::<syn::ItemImpl>(input_clone) {
        // 这是 impl 块，需要提取方法并生成路由注册
        return controller_impl_macro(attr, item_impl);
    }

    // 否则解析为结构体
    let item_struct = syn::parse_macro_input!(input as syn::ItemStruct);

    // 解析 base path（结构体上的 controller 可以没有 path，由 impl 块指定）
    let base_path = if attr.is_empty() {
        // 结构体上没有 path，不生成常量，由 impl 块上的 controller 指定
        quote! {}
    } else {
        let attr_str = attr.to_string();
        let base_path = attr_str.trim_matches('"').to_string();
        quote! {
            #[doc(hidden)]
            const __POTATO_CONTROLLER_BASE_PATH: &str = #base_path;
        }
    };

    // 验证结构体字段并获取字段信息
    let (has_once_cache, has_session_cache) = validate_controller_struct(&item_struct);
    let struct_name = &item_struct.ident;
    let struct_name_str = struct_name.to_string();

    // 生成结构体定义、常量和 inventory 提交
    // 同时生成隐藏的 controller 创建辅助函数
    let controller_creation_fn = if has_session_cache {
        // 结构体有 SessionCache 字段，生成包含鉴权的创建函数
        // 直接创建并返回 Box<Self>
        quote! {
            #[doc(hidden)]
            #[allow(dead_code)]
            async fn __potato_create_controller(req: &potato::HttpRequest) -> Result<Box<Self>, potato::HttpResponse> {
                // 在堆上分配缓存
                let once_cache = Box::leak(Box::new(potato::OnceCache::new()));

                // 从 Authorization header 中提取 Bearer token 并加载 session
                let session_cache = {
                    if let Some(h) = req.headers.get(&potato::utils::refstr::HeaderOrHipStr::from_str("Authorization")) {
                        let header_value = h.as_str();
                        if header_value.starts_with("Bearer ") {
                            potato::SessionCache::from_token(&header_value[7..]).await.ok()
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                };

                let session_cache = match session_cache {
                    Some(cache) => cache,
                    None => {
                        let mut resp = potato::HttpResponse::text("Unauthorized: Missing or invalid Authorization header");
                        resp.http_code = 401;
                        return Err(resp);
                    }
                };
                let session_cache = Box::leak(Box::new(session_cache));

                // 创建 controller 实例
                let controller = Self {
                    once_cache,
                    sess_cache: session_cache,
                };

                Ok(Box::new(controller))
            }
        }
    } else {
        // 结构体没有 SessionCache 字段，生成不包含鉴权的创建函数
        // 创建临时的 SessionCache（但不使用），返回 Box<Self>
        quote! {
            #[doc(hidden)]
            #[allow(dead_code)]
            async fn __potato_create_controller(_req: &potato::HttpRequest) -> Result<Box<Self>, potato::HttpResponse> {
                // 在堆上分配缓存
                let once_cache = Box::leak(Box::new(potato::OnceCache::new()));

                // 创建临时的 SessionCache（不需要鉴权，也不使用）
                let _temp_session_cache = Box::leak(Box::new(potato::SessionCache::new()));

                // 创建 controller 实例（不包含 sess_cache 字段）
                let controller = Self {
                    once_cache,
                };

                Ok(Box::new(controller))
            }
        }
    };

    // 提取结构体的泛型参数（包括生命周期）
    let struct_generics = &item_struct.generics;
    let (impl_generics, type_generics, where_clause) = struct_generics.split_for_impl();

    let output = quote! {
        #item_struct

        #base_path

        #[doc(hidden)]
        const __POTATO_CONTROLLER_NAME: &str = #struct_name_str;

        // 提交 Controller 结构体字段信息到 inventory
        potato::inventory::submit! {
            potato::ControllerStructFlag::new(
                #struct_name_str,
                potato::ControllerStructFieldInfo {
                    has_once_cache: #has_once_cache,
                    has_session_cache: #has_session_cache,
                }
            )
        }

        // 生成隐藏的 controller 创建辅助函数
        impl #impl_generics #struct_name #type_generics #where_clause {
            #controller_creation_fn
        }
    };

    output.into()
}

/// 处理 impl 块的 controller 宏
fn controller_impl_macro(attr: TokenStream, item_impl: syn::ItemImpl) -> TokenStream {
    // 解析 base path（如果 attr 为空，则从常量读取）
    let base_path_str = if attr.is_empty() {
        // 从结构体上的 controller 宏生成的常量读取
        // 这种情况暂不支持，因为宏展开时无法读取常量值
        None
    } else {
        let attr_str = attr.to_string();
        Some(attr_str.trim_matches('"').to_string())
    };

    // 从 impl 块中提取类型名称
    let self_type = &item_impl.self_ty;

    // 提取不带生命周期参数的类型名称（用于实例化）
    let self_type_name = match &*item_impl.self_ty {
        syn::Type::Path(type_path) => {
            // 获取路径的最后一段（类型名称），不包含泛型参数
            if let Some(segment) = type_path.path.segments.last() {
                let ident = &segment.ident;
                quote! { #ident }
            } else {
                quote! { #self_type }
            }
        }
        _ => quote! { #self_type },
    };

    // 提取不带生命周期参数的类型名称字符串（用于 Swagger tag）
    let self_type_tag = match &*item_impl.self_ty {
        syn::Type::Path(type_path) => {
            // 获取路径的最后一段（类型名称），不包含泛型参数
            if let Some(segment) = type_path.path.segments.last() {
                segment.ident.to_string()
            } else {
                self_type.to_token_stream().to_string()
            }
        }
        _ => self_type.to_token_stream().to_string(),
    };

    // 创建清理后的 impl 块（移除方法上的 http_* 标注）
    let mut cleaned_items = Vec::new();
    let mut generated_code = Vec::new();

    for item in &item_impl.items {
        if let syn::ImplItem::Fn(method) = item {
            // 检查是否有 http_* 标注
            let has_http_attr = method.attrs.iter().any(|attr| {
                let attr_name = attr.path().to_token_stream().to_string();
                attr_name.contains("http_get")
                    || attr_name.contains("http_post")
                    || attr_name.contains("http_put")
                    || attr_name.contains("http_delete")
                    || attr_name.contains("http_head")
                    || attr_name.contains("http_patch")
                    || attr_name.contains("http_options")
            });

            if has_http_attr {
                // 有 http_* 标注，创建清理后的方法（移除 http_* 标注）
                let mut cleaned_method = method.clone();
                cleaned_method.attrs = method
                    .attrs
                    .iter()
                    .filter(|attr| {
                        let attr_name = attr.path().to_token_stream().to_string();
                        !attr_name.contains("http_get")
                            && !attr_name.contains("http_post")
                            && !attr_name.contains("http_put")
                            && !attr_name.contains("http_delete")
                            && !attr_name.contains("http_head")
                            && !attr_name.contains("http_patch")
                            && !attr_name.contains("http_options")
                    })
                    .cloned()
                    .collect();

                cleaned_items.push(syn::ImplItem::Fn(cleaned_method));

                // 为每个 http_* 标注生成包装函数和路由注册
                for attr in &method.attrs {
                    let attr_name = attr.path().to_token_stream().to_string();
                    if attr_name.contains("http_get")
                        || attr_name.contains("http_post")
                        || attr_name.contains("http_put")
                        || attr_name.contains("http_delete")
                        || attr_name.contains("http_head")
                        || attr_name.contains("http_patch")
                        || attr_name.contains("http_options")
                    {
                        // 提取 HTTP 方法名
                        let http_method = if attr_name.contains("http_get") {
                            "GET"
                        } else if attr_name.contains("http_post") {
                            "POST"
                        } else if attr_name.contains("http_put") {
                            "PUT"
                        } else if attr_name.contains("http_delete") {
                            "DELETE"
                        } else if attr_name.contains("http_head") {
                            "HEAD"
                        } else if attr_name.contains("http_patch") {
                            "PATCH"
                        } else {
                            "OPTIONS"
                        };

                        // 提取 path 参数
                        let method_path = match &attr.meta {
                            syn::Meta::List(list) => {
                                if let Ok(lit_str) =
                                    syn::parse::<syn::LitStr>(list.tokens.clone().into())
                                {
                                    lit_str.value()
                                } else {
                                    String::new()
                                }
                            }
                            _ => String::new(),
                        };

                        // 拼接路径（在宏展开时完成）
                        let final_path = if let Some(ref base_path) = base_path_str {
                            // base path 已知，直接在宏展开时拼接
                            if method_path.is_empty() {
                                base_path.clone()
                            } else {
                                format!("{}{}", base_path, method_path)
                            }
                        } else {
                            // base path 未知（从常量读取），这种情况暂不支持
                            panic!("impl block controller must specify a base path, e.g., #[potato::controller(\"/api/users\")]");
                        };

                        // 将 final_path 转换为 LitStr
                        let final_path_lit =
                            syn::LitStr::new(&final_path, proc_macro2::Span::call_site());

                        // 生成包装函数名
                        let fn_name = &method.sig.ident;
                        let wrapper_fn_name = quote::format_ident!("__potato_ctrl_{}", fn_name);
                        let is_async = method.sig.asyncness.is_some();

                        // 检测方法是否有 receiver，以及是否是 mutable
                        let (has_receiver, _is_mut_receiver) = method
                            .sig
                            .inputs
                            .iter()
                            .filter_map(|arg| {
                                if let syn::FnArg::Receiver(recv) = arg {
                                    Some((true, recv.mutability.is_some()))
                                } else {
                                    None
                                }
                            })
                            .next()
                            .unwrap_or((false, false));

                        // 提取非 receiver 参数
                        let other_params: Vec<_> = method
                            .sig
                            .inputs
                            .iter()
                            .filter_map(|arg| {
                                if let syn::FnArg::Typed(pat_type) = arg {
                                    Some(pat_type.clone())
                                } else {
                                    None
                                }
                            })
                            .collect();

                        // 检测非 receiver 参数中是否包含 SessionCache
                        let method_has_session_cache = other_params.iter().any(|pat_type| {
                            pat_type.ty.to_token_stream().to_string().type_simplify()
                                == "& mut SessionCache"
                        });

                        // 如果方法有 receiver（&self 或 &mut self），或者参数包含 SessionCache，则需要鉴权
                        let doc_auth = has_receiver || method_has_session_cache;

                        // 提取参数名
                        let param_names: Vec<_> = other_params
                            .iter()
                            .filter_map(|pat_type| {
                                if let syn::Pat::Ident(pat_ident) = &*pat_type.pat {
                                    Some(pat_ident.ident.clone())
                                } else {
                                    None
                                }
                            })
                            .collect();

                        // 生成方法调用
                        let method_call = if has_receiver {
                            // 有 receiver，需要实例化 controller
                            // 使用结构体生成的 __potato_create_controller 函数
                            // 返回 Box<Self>

                            if param_names.is_empty() {
                                if is_async {
                                    quote! {
                                        {
                                            let mut controller = match #self_type_name::__potato_create_controller(req).await {
                                                Ok(boxed) => boxed,
                                                Err(resp) => return resp,
                                            };
                                            controller.#fn_name().await
                                        }
                                    }
                                } else {
                                    quote! {
                                        {
                                            let mut controller = match #self_type_name::__potato_create_controller(req).await {
                                                Ok(boxed) => boxed,
                                                Err(resp) => return resp,
                                            };
                                            controller.#fn_name()
                                        }
                                    }
                                }
                            } else {
                                if is_async {
                                    quote! {
                                        {
                                            let mut controller = match #self_type_name::__potato_create_controller(req).await {
                                                Ok(boxed) => boxed,
                                                Err(resp) => return resp,
                                            };
                                            controller.#fn_name(#(#param_names),*).await
                                        }
                                    }
                                } else {
                                    quote! {
                                        {
                                            let mut controller = match #self_type_name::__potato_create_controller(req).await {
                                                Ok(boxed) => boxed,
                                                Err(resp) => return resp,
                                            };
                                            controller.#fn_name(#(#param_names),*)
                                        }
                                    }
                                }
                            }
                        } else {
                            // 没有 receiver，直接调用关联函数
                            // 但仍需要处理参数（如 SessionCache）

                            if param_names.is_empty() {
                                if is_async {
                                    quote! { #self_type_name::#fn_name().await }
                                } else {
                                    quote! { #self_type_name::#fn_name() }
                                }
                            } else {
                                // 有参数，需要根据参数类型生成绑定代码
                                // 生成参数绑定
                                let mut param_bindings = Vec::new();
                                for (i, param) in other_params.iter().enumerate() {
                                    let param_type_str =
                                        param.ty.to_token_stream().to_string().type_simplify();
                                    let param_name = &param_names[i];

                                    match &param_type_str[..] {
                                        "& mut OnceCache" => {
                                            param_bindings.push(quote! {
                                                let #param_name = &mut __potato_once_cache;
                                            });
                                        }
                                        "& mut SessionCache" => {
                                            param_bindings.push(quote! {
                                                let #param_name = &mut __potato_session_cache;
                                            });
                                        }
                                        _ => {
                                            // 其他参数类型暂不支持
                                        }
                                    }
                                }

                                // 生成 SessionCache 加载逻辑（从 Authorization header）
                                let needs_session_cache = other_params.iter().any(|p| {
                                    p.ty.to_token_stream().to_string().type_simplify()
                                        == "& mut SessionCache"
                                });

                                let session_cache_init = if needs_session_cache {
                                    quote! {
                                        {
                                            // 从 Authorization header 中提取 Bearer token 并加载 session
                                            if let Some(h) = req.headers.get(&potato::utils::refstr::HeaderOrHipStr::from_str("Authorization")) {
                                                let header_value = h.as_str();
                                                if header_value.starts_with("Bearer ") {
                                                    potato::SessionCache::from_token(&header_value[7..]).await.ok()
                                                } else {
                                                    None
                                                }
                                            } else {
                                                None
                                            }
                                        }
                                    }
                                } else {
                                    quote! { None }
                                };

                                if is_async {
                                    quote! {
                                        {
                                            let mut __potato_once_cache = potato::OnceCache::new();
                                            let mut __potato_session_cache = #session_cache_init.unwrap_or_else(|| potato::SessionCache::new());
                                            #(#param_bindings)*
                                            #self_type_name::#fn_name(#(#param_names),*).await
                                        }
                                    }
                                } else {
                                    quote! {
                                        {
                                            let mut __potato_once_cache = potato::OnceCache::new();
                                            let mut __potato_session_cache = #session_cache_init.unwrap_or_else(|| potato::SessionCache::new());
                                            #(#param_bindings)*
                                            #self_type_name::#fn_name(#(#param_names),*)
                                        }
                                    }
                                }
                            }
                        };

                        // 生成包装函数 - 简化版，直接返回文本
                        let wrapper_fn = if is_async {
                            quote! {
                                #[doc(hidden)]
                                fn #wrapper_fn_name(req: &mut potato::HttpRequest) -> std::pin::Pin<Box<dyn std::future::Future<Output = potato::HttpResponse> + Send + '_>> {
                                    Box::pin(async move {
                                        match #method_call {
                                            Ok(resp) => potato::HttpResponse::text(resp.to_string()),
                                            Err(err) => potato::HttpResponse::error(err.to_string()),
                                        }
                                    })
                                }
                            }
                        } else {
                            quote! {
                                #[doc(hidden)]
                                fn #wrapper_fn_name(req: &mut potato::HttpRequest) -> std::pin::Pin<Box<dyn std::future::Future<Output = potato::HttpResponse> + Send + '_>> {
                                    Box::pin(async move {
                                        match #method_call {
                                            Ok(resp) => potato::HttpResponse::text(resp.to_string()),
                                            Err(err) => potato::HttpResponse::error(err.to_string()),
                                        }
                                    })
                                }
                            }
                        };

                        generated_code.push(wrapper_fn);

                        // 生成路由注册
                        let http_method_ident = quote::format_ident!("{}", http_method);

                        let route_register = quote! {
                            potato::inventory::submit! {
                                potato::RequestHandlerFlag::new(
                                    potato::HttpMethod::#http_method_ident,
                                    #final_path_lit,
                                    potato::HttpHandler::Async(#wrapper_fn_name),
                                    potato::RequestHandlerFlagDoc::new(true, #doc_auth, "", "", "", #self_type_tag)
                                )
                            }
                        };

                        generated_code.push(route_register);
                    }
                }
            } else {
                // 没有 http_* 标注，保留原方法
                cleaned_items.push(item.clone());
            }
        } else {
            // 非方法项，直接保留
            cleaned_items.push(item.clone());
        }
    }

    // 生成清理后的 impl 块
    let mut cleaned_impl = item_impl.clone();
    cleaned_impl.items = cleaned_items;

    // 生成最终代码：清理后的 impl 块 + 生成的独立函数和路由注册
    let output = quote! {
        #cleaned_impl

        #(#generated_code)*
    };

    output.into()
}

#[proc_macro_attribute]
pub fn preprocess(attr: TokenStream, input: TokenStream) -> TokenStream {
    preprocess_macro(attr, input)
}

#[proc_macro_attribute]
pub fn postprocess(attr: TokenStream, input: TokenStream) -> TokenStream {
    postprocess_macro(attr, input)
}

/// handle_error 属性宏 - 定义全局错误处理函数
///
/// # 签名要求
/// - 参数1: `req: &mut HttpRequest`
/// - 参数2: `err: anyhow::Error`
/// - 返回: `HttpResponse`
/// - 支持 async fn 和普通 fn
///
/// # 示例
/// ```rust,ignore
/// #[potato::handle_error]
/// async fn handle_error(req: &mut HttpRequest, err: anyhow::Error) -> HttpResponse {
///     HttpResponse::json(serde_json::json!({
///         "error": format!("{}", err)
///     }))
/// }
/// ```
fn handle_error_macro(attr: TokenStream, input: TokenStream) -> TokenStream {
    if !attr.is_empty() {
        return input;
    }

    let root_fn = syn::parse_macro_input!(input as syn::ItemFn);
    let fn_name = root_fn.sig.ident.clone();
    let is_async = root_fn.sig.asyncness.is_some();

    // 验证函数签名
    if root_fn.sig.inputs.len() != 2 {
        panic!("`handle_error` function must accept exactly two arguments");
    }

    let mut arg_types = vec![];
    for arg in root_fn.sig.inputs.iter() {
        match arg {
            syn::FnArg::Typed(arg) => {
                arg_types.push(arg.ty.to_token_stream().to_string().type_simplify())
            }
            _ => panic!("`handle_error` function does not support receiver argument"),
        }
    }

    if arg_types[0] != "& mut HttpRequest" {
        panic!(
            "`handle_error` first argument must be `&mut potato::HttpRequest`, got `{}`",
            arg_types[0]
        );
    }
    if arg_types[1] != "anyhow::Error" {
        panic!(
            "`handle_error` second argument must be `anyhow::Error`, got `{}`",
            arg_types[1]
        );
    }

    let ret_type = root_fn
        .sig
        .output
        .to_token_stream()
        .to_string()
        .type_simplify();
    if ret_type != "HttpResponse" {
        panic!(
            "`handle_error` return type must be `potato::HttpResponse`, got `{}`",
            ret_type
        );
    }

    // 生成适配器函数
    let wrap_name = format_ident!("__potato_error_handler_adapter_{}", fn_name);
    let wrap_name_inner = format_ident!("__potato_error_handler_adapter_inner_{}", fn_name);

    // 生成内部函数
    let call_body = if is_async {
        quote! { #fn_name(req, err).await }
    } else {
        quote! { #fn_name(req, err) }
    };

    quote! {
        #root_fn

        #[doc(hidden)]
        async fn #wrap_name_inner(
            req: &mut potato::HttpRequest,
            err: anyhow::Error,
        ) -> potato::HttpResponse {
            #call_body
        }

        #[doc(hidden)]
        pub fn #wrap_name(
            req: &mut potato::HttpRequest,
            err: anyhow::Error,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = potato::HttpResponse> + Send + '_>> {
            Box::pin(#wrap_name_inner(req, err))
        }

        potato::inventory::submit! {
            potato::ErrorHandlerFlag::new(
                potato::ErrorHandler::Async(#wrap_name)
            )
        }
    }
    .into()
}

#[proc_macro_attribute]
pub fn handle_error(attr: TokenStream, input: TokenStream) -> TokenStream {
    handle_error_macro(attr, input)
}

/// limit_size 属性宏 - 为 handler 设置独立的请求体大小限制
///
/// # 参数
/// * 单个值: `#[potato::limit_size(1024 * 1024 * 1024)]` - 仅限制 body 为 1GB
/// * 命名参数: `#[potato::limit_size(header = 2 * 1024 * 1024, body = 500 * 1024 * 1024)]`
///
/// # 示例
/// ```rust,ignore
/// // 限制 body 为 1GB
/// #[potato::http_post("/upload")]
/// #[potato::limit_size(1024 * 1024 * 1024)]
/// async fn large_upload(req: &mut potato::HttpRequest) -> potato::HttpResponse {
///     todo!()
/// }
///
/// // 分别限制 header 和 body
/// #[potato::http_post("/upload")]
/// #[potato::limit_size(header = 2 * 1024 * 1024, body = 500 * 1024 * 1024)]
/// async fn medium_upload(req: &mut potato::HttpRequest) -> potato::HttpResponse {
///     todo!()
/// }
/// ```
#[proc_macro_attribute]
pub fn limit_size(attr: TokenStream, input: TokenStream) -> TokenStream {
    limit_size_macro(attr, input)
}

fn limit_size_macro(attr: TokenStream, input: TokenStream) -> TokenStream {
    // 解析参数
    let (_max_header, max_body) = {
        let attr_tokens: proc_macro2::TokenStream = attr.clone().into();
        if attr_tokens.is_empty() {
            // 默认值: 不限制 header，使用全局 body 限制
            (None, None)
        } else {
            // 尝试解析为命名参数或单值
            let result = syn::parse::Parser::parse2(
                |input: syn::parse::ParseStream| -> syn::Result<(Option<syn::Expr>, Option<syn::Expr>)> {
                    let mut header_expr = None;
                    let mut body_expr = None;

                    // 尝试解析命名参数
                    while !input.is_empty() {
                        let ident: Ident = input.parse()?;
                        input.parse::<Token![=]>()?;
                        let value: syn::Expr = input.parse()?;

                        match ident.to_string().as_str() {
                            "header" => header_expr = Some(value),
                            "body" => body_expr = Some(value),
                            _ => return Err(syn::Error::new(ident.span(), "expected 'header' or 'body'")),
                        }

                        // 可选的逗号
                        if input.peek(Token![,]) {
                            input.parse::<Token![,]>()?;
                        }
                    }

                    Ok((header_expr, body_expr))
                },
                attr_tokens.clone(),
            );

            match result {
                Ok((h, b)) => (h, b),
                Err(_) => {
                    // 解析失败，尝试作为单值（body 限制）
                    if let Ok(expr) = syn::parse2::<syn::Expr>(attr_tokens) {
                        (None, Some(expr))
                    } else {
                        (None, None)
                    }
                }
            }
        }
    };

    let root_fn = syn::parse_macro_input!(input as syn::ItemFn);

    // 生成检查代码
    let body_check = if let Some(body_expr) = max_body {
        quote! {
            // 检查 body 大小
            let body_len = req.body.len();
            if body_len > #body_expr {
                let mut res = potato::HttpResponse::text(format!(
                    "Payload Too Large: body size {} bytes exceeds limit {} bytes",
                    body_len, #body_expr
                ));
                res.http_code = 413;
                return res;
            }
        }
    } else {
        quote! {}
    };

    // 克隆整个函数，然后修改 block
    let mut wrapped_fn = root_fn.clone();
    let original_block = root_fn.block.as_ref();
    let new_block: syn::Block = syn::parse_quote!({
        #body_check
        #original_block
    });
    wrapped_fn.block = Box::new(new_block);

    quote! {
        #wrapped_fn
    }
    .into()
}

/// header 属性宏 - 这是一个占位宏，实际解析在 http_handler_macro 中完成
/// 这个宏的存在使得 #[potato::header(...)] 语法能够被编译器识别
#[proc_macro_attribute]
pub fn header(_attr: TokenStream, input: TokenStream) -> TokenStream {
    // 直接返回原始函数，不做任何修改
    // 实际的 header 解析和处理在 http_get/http_post 等宏中完成
    input
}

/// cors 属性宏 - 这是一个占位宏，实际解析在 http_handler_macro 中完成
/// 这个宏的存在使得 #[potato::cors(...)] 语法能够被编译器识别
#[proc_macro_attribute]
pub fn cors(_attr: TokenStream, input: TokenStream) -> TokenStream {
    // 直接返回原始函数，不做任何修改
    // 实际的 cors 解析和处理在 http_handler_macro 中完成
    input
}

#[proc_macro]
pub fn embed_dir(input: TokenStream) -> TokenStream {
    let path = syn::parse_macro_input!(input as syn::LitStr).value();
    quote! {{
        #[derive(potato::rust_embed::Embed)]
        #[folder = #path]
        struct Asset;

        potato::load_embed::<Asset>()
    }}
    .into()
}

#[proc_macro_derive(StandardHeader)]
pub fn standard_header_derive(input: TokenStream) -> TokenStream {
    let root_enum = syn::parse_macro_input!(input as syn::ItemEnum);
    let enum_name = root_enum.ident;
    let mut try_from_str_items = vec![];
    let mut to_str_items = vec![];
    let mut headers_items = vec![];
    let mut headers_apply_items = vec![];
    for root_field in root_enum.variants.iter() {
        let name = root_field.ident.clone();
        if root_field.fields.iter().next().is_some() {
            panic!("unsupported enum type");
        }
        let str_name = name.to_string().replace("_", "-");
        let len = str_name.len();
        try_from_str_items
            .push(quote! { #len if value.eq_ignore_ascii_case(#str_name) => Some(Self::#name), });
        to_str_items.push(quote! { Self::#name => #str_name, });
        headers_items.push(quote! { #name(String), });
        headers_apply_items
            .push(quote! { Headers::#name(s) => self.set_header(HeaderItem::#name.to_str(), s), });
    }
    let r = quote! {
        impl #enum_name {
            pub fn try_from_str(value: &str) -> Option<Self> {
                match value.len() {
                    #( #try_from_str_items )*
                    _ => None,
                }
            }

            pub fn to_str(&self) -> &'static str {
                match self {
                    #( #to_str_items )*
                }
            }
        }

        pub enum Headers {
            #( #headers_items )*
            Custom((String, String)),
        }

        impl HttpRequest {
            pub fn apply_header(&mut self, header: Headers) {
                match header {
                    #( #headers_apply_items )*
                    Headers::Custom((k, v)) => self.set_header(&k[..], v),
                }
            }
        }
    };
    r.into()
}
