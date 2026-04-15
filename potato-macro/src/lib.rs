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
    let mut config = CorsAttrConfig {
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
        Err(e) => panic!("Failed to parse cors attributes: {}", e),
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

fn validate_preprocess_signature(root_fn: &syn::ItemFn) -> String {
    if root_fn.sig.inputs.len() != 1 {
        panic!("`preprocess` function must accept exactly one argument");
    }
    let arg = root_fn.sig.inputs.first().unwrap();
    let arg_type_str = match arg {
        syn::FnArg::Typed(arg) => arg.ty.to_token_stream().to_string().type_simplify(),
        _ => panic!("`preprocess` function does not support receiver argument"),
    };
    if arg_type_str != "& mut HttpRequest" {
        panic!(
            "`preprocess` argument type must be `&mut potato::HttpRequest`, got `{arg_type_str}`"
        );
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
    ret_type
}

fn validate_postprocess_signature(root_fn: &syn::ItemFn) -> String {
    if root_fn.sig.inputs.len() != 2 {
        panic!("`postprocess` function must accept exactly two arguments");
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
    ret_type
}

fn preprocess_macro(attr: TokenStream, input: TokenStream) -> TokenStream {
    if !attr.is_empty() {
        return input;
    }
    let root_fn = syn::parse_macro_input!(input as syn::ItemFn);
    let fn_name = root_fn.sig.ident.clone();
    let wrap_name = format_ident!("__potato_preprocess_adapter_{}", fn_name);
    let is_async = root_fn.sig.asyncness.is_some();
    let ret_type = validate_preprocess_signature(&root_fn);
    let body = if is_async {
        match &ret_type[..] {
            "Result<Option<HttpResponse>>" => quote! { #fn_name(req).await },
            "Option<HttpResponse>" => quote! { Ok(#fn_name(req).await) },
            "Result<()>" => quote! { #fn_name(req).await.map(|_| None) },
            "()" => quote! {
                #fn_name(req).await;
                Ok(None)
            },
            _ => unreachable!(),
        }
    } else {
        match &ret_type[..] {
            "Result<Option<HttpResponse>>" => quote! { #fn_name(req) },
            "Option<HttpResponse>" => quote! { Ok(#fn_name(req)) },
            "Result<()>" => quote! { #fn_name(req).map(|_| None) },
            "()" => quote! {
                #fn_name(req);
                Ok(None)
            },
            _ => unreachable!(),
        }
    };
    quote! {
        #root_fn

        #[doc(hidden)]
        async fn #wrap_name(req: &mut potato::HttpRequest) -> anyhow::Result<Option<potato::HttpResponse>> {
            #body
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
    let is_async = root_fn.sig.asyncness.is_some();
    let ret_type = validate_postprocess_signature(&root_fn);
    let body = if is_async {
        match &ret_type[..] {
            "Result<()>" => quote! { #fn_name(req, res).await },
            "()" => quote! {
                #fn_name(req, res).await;
                Ok(())
            },
            _ => unreachable!(),
        }
    } else {
        match &ret_type[..] {
            "Result<()>" => quote! { #fn_name(req, res) },
            "()" => quote! {
                #fn_name(req, res);
                Ok(())
            },
            _ => unreachable!(),
        }
    };
    quote! {
        #root_fn

        #[doc(hidden)]
        async fn #wrap_name(req: &mut potato::HttpRequest, res: &mut potato::HttpResponse) -> anyhow::Result<()> {
            #body
        }
    }
    .into()
}

fn http_handler_macro(attr: TokenStream, input: TokenStream, req_name: &str) -> TokenStream {
    let req_name = Ident::new(req_name, Span::call_site());
    let (route_path, oauth_arg, default_headers) = {
        let mut oroute_path = syn::parse::<syn::LitStr>(attr.clone())
            .ok()
            .map(|path| path.value());
        let mut oauth_arg = None;
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
                } else if meta.path.is_ident("auth_arg") {
                    if let Ok(arg) = meta.value() {
                        if let Ok(tmp_field) = arg.parse::<Ident>() {
                            oauth_arg = Some(tmp_field.to_string());
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
        if oroute_path.is_none() {
            panic!("`path` argument is required");
        }
        let route_path = oroute_path.unwrap();
        if !route_path.starts_with('/') {
            panic!("route path must start with '/'");
        }
        (route_path, oauth_arg, default_headers)
    };

    // 解析函数上的 #[potato::header(...)] 标注
    let mut root_fn = syn::parse_macro_input!(input as syn::ItemFn);
    let mut fn_headers: Vec<(String, String)> = Vec::new();
    let mut cors_config: Option<CorsAttrConfig> = None;
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

        remaining_attrs.push(attr.clone());
    }

    // 合并默认headers和函数headers
    let mut all_headers = default_headers;
    all_headers.extend(fn_headers);

    root_fn.attrs = remaining_attrs;
    let (preprocess_fns, postprocess_fns) = collect_handler_hooks(&mut root_fn);
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
    let doc_auth = oauth_arg.is_some();
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
    let wrap_func_name = random_ident();
    let mut args = vec![];
    let mut arg_names = vec![];
    let mut doc_args = vec![];
    let mut arg_auth_mark = false;
    for arg in root_fn.sig.inputs.iter() {
        if let syn::FnArg::Typed(arg) = arg {
            let arg_type_str = arg
                .ty
                .as_ref()
                .to_token_stream()
                .to_string()
                .type_simplify();
            let arg_name_str = arg.pat.to_token_stream().to_string();
            args.push(match &arg_type_str[..] {
                "& mut HttpRequest" => quote! { req },
                "PostFile" => {
                    doc_args.push(json!({ "name": arg_name_str, "type": arg_type_str }));
                    quote! {
                        match req.body_files.get(&potato::utils::refstr::LocalHipStr<'static>::from_str(#arg_name_str)).cloned() {
                            Some(file) => file,
                            None => return potato::HttpResponse::error(format!("miss arg: {}", #arg_name_str)),
                        }
                    }
                },
                arg_type_str if ARG_TYPES.contains(arg_type_str) => {
                    let is_auth_arg = match oauth_arg.as_ref() {
                        Some(auth_arg) => auth_arg == &arg_name_str,
                        None => false,
                    };
                    if is_auth_arg {
                        if arg_type_str != "String" {
                            panic!("auth_arg argument is must String type");
                        }
                        arg_auth_mark = true;
                        if is_async {
                            quote! {
                                match req.headers
                                    .get(&potato::utils::refstr::HeaderOrHipStr::from_str("Authorization"))
                                    .map(|v| v.to_str()) {
                                    Some(mut auth) => {
                                        if auth.starts_with("Bearer ") {
                                            auth = &auth[7..];
                                        }
                                        match potato::ServerAuth::jwt_check(&auth).await {
                                            Ok(payload) => payload,
                                            Err(err) => return potato::HttpResponse::error(format!("auth failed: {err:?}")),
                                        }
                                    }
                                    None => return potato::HttpResponse::error("miss header : Authorization"),
                                }
                            }
                        } else {
                            quote! {
                                match req.headers
                                    .get(&potato::utils::refstr::HeaderOrHipStr::from_str("Authorization"))
                                    .map(|v| v.to_str()) {
                                    Some(mut auth) => {
                                        if auth.starts_with("Bearer ") {
                                            auth = &auth[7..];
                                        }
                                        match tokio::task::block_in_place(|| {
                                            tokio::runtime::Handle::current().block_on(potato::ServerAuth::jwt_check(&auth))
                                        }) {
                                            Ok(payload) => payload,
                                            Err(err) => return potato::HttpResponse::error(format!("auth failed: {err:?}")),
                                        }
                                    }
                                    None => return potato::HttpResponse::error("miss header : Authorization"),
                                }
                            }
                        }
                    } else {
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
                },
                _ => panic!("unsupported arg type: [{arg_type_str}]"),
            });
            arg_names.push(random_ident());
        } else {
            panic!("unsupported: {}", arg.to_token_stream());
        }
    }
    if !arg_auth_mark && doc_auth {
        panic!("`auth_arg` attribute is must point to an existing argument");
    }
    let wrap_func_name2 = random_ident();
    let ret_type = root_fn
        .sig
        .output
        .to_token_stream()
        .to_string()
        .type_simplify();
    let call_expr = match args.len() {
        0 => quote! { #fn_name() },
        1 => quote! {{
            let #(#arg_names),* = #(#args),*;
            #fn_name(#(#arg_names),*)
        }},
        _ => quote! {{
            let (#(#arg_names),*) = (#(#args),*);
            #fn_name(#(#arg_names),*)
        }},
    };
    let handler_wrap_func_body = if is_async {
        match &ret_type[..] {
            "Result<()>" => quote! {
                match #call_expr.await {
                    Ok(_) => potato::HttpResponse::text("ok"),
                    Err(err) => potato::HttpResponse::error(format!("{err:?}")),
                }
            },
            "Result<HttpResponse>" | "anyhow::Result<HttpResponse>" => quote! {
                match #call_expr.await {
                    Ok(ret) => ret,
                    Err(err) => potato::HttpResponse::error(format!("{err:?}")),
                }
            },
            "Result<String>" | "anyhow::Result<String>" => quote! {
                match #call_expr.await {
                    Ok(ret) => potato::HttpResponse::html(ret),
                    Err(err) => potato::HttpResponse::error(format!("{err:?}")),
                }
            },
            "Result<& 'static str>" | "anyhow::Result<& 'static str>" => quote! {
                match #call_expr.await {
                    Ok(ret) => potato::HttpResponse::html(ret),
                    Err(err) => potato::HttpResponse::error(format!("{err:?}")),
                }
            },
            "()" => quote! {
                #call_expr.await;
                potato::HttpResponse::text("ok")
            },
            "HttpResponse" => quote! {
                #call_expr.await
            },
            "String" => quote! {
                potato::HttpResponse::html(#call_expr.await)
            },
            "& 'static str" => quote! {
                potato::HttpResponse::html(#call_expr.await)
            },
            _ => panic!("unsupported ret type: {ret_type}"),
        }
    } else {
        match &ret_type[..] {
            "Result<()>" => quote! {
                match #call_expr {
                    Ok(_) => potato::HttpResponse::text("ok"),
                    Err(err) => potato::HttpResponse::error(format!("{err:?}")),
                }
            },
            "Result<HttpResponse>" | "anyhow::Result<HttpResponse>" => quote! {
                match #call_expr {
                    Ok(ret) => ret,
                    Err(err) => potato::HttpResponse::error(format!("{err:?}")),
                }
            },
            "Result<String>" | "anyhow::Result<String>" => quote! {
                match #call_expr {
                    Ok(ret) => potato::HttpResponse::html(ret),
                    Err(err) => potato::HttpResponse::error(format!("{err:?}")),
                }
            },
            "Result<& 'static str>" | "anyhow::Result<& 'static str>" => quote! {
                match #call_expr {
                    Ok(ret) => potato::HttpResponse::html(ret),
                    Err(err) => potato::HttpResponse::error(format!("{err:?}")),
                }
            },
            "()" => quote! {
                #call_expr;
                potato::HttpResponse::text("ok")
            },
            "HttpResponse" => quote! {
                #call_expr
            },
            "String" => quote! {
                potato::HttpResponse::html(#call_expr)
            },
            "& 'static str" => quote! {
                potato::HttpResponse::html(#call_expr)
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

    let wrap_func_body = if is_async {
        quote! {
            let mut __potato_pre_response: Option<potato::HttpResponse> = None;
            #(
                if __potato_pre_response.is_none() {
                    __potato_pre_response = match #preprocess_adapters(req).await {
                        Ok(Some(ret)) => Some(ret),
                        Ok(None) => None,
                        Err(err) => Some(potato::HttpResponse::error(format!("{err:?}"))),
                    };
                }
            )*

            let mut __potato_response = match __potato_pre_response {
                Some(ret) => ret,
                None => #handler_wrap_func_body,
            };

            #(
                if let Err(err) = #postprocess_adapters(req, &mut __potato_response).await {
                    return potato::HttpResponse::error(format!("{err:?}"));
                }
            )*

            #add_headers_code
            #cors_headers_code

            __potato_response
        }
    } else {
        quote! {
            let mut __potato_pre_response: Option<potato::HttpResponse> = None;
            #(
                if __potato_pre_response.is_none() {
                    __potato_pre_response = match tokio::task::block_in_place(|| {
                        tokio::runtime::Handle::current().block_on(#preprocess_adapters(req))
                    }) {
                        Ok(Some(ret)) => Some(ret),
                        Ok(None) => None,
                        Err(err) => Some(potato::HttpResponse::error(format!("{err:?}"))),
                    };
                }
            )*

            let mut __potato_response = match __potato_pre_response {
                Some(ret) => ret,
                None => #handler_wrap_func_body,
            };

            #(
                if let Err(err) = tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current().block_on(#postprocess_adapters(req, &mut __potato_response))
                }) {
                    return potato::HttpResponse::error(format!("{err:?}"));
                }
            )*

            #add_headers_code
            #cors_headers_code

            __potato_response
        }
    };

    if is_async {
        quote! {
            #root_fn

            #auto_head_handler

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
                #route_path,
                potato::HttpHandler::Async(#wrap_func_name),
                potato::RequestHandlerFlagDoc::new(#doc_show, #doc_auth, #doc_summary, #doc_desp, #doc_args)
            )}
        }
        .into()
    } else {
        quote! {
            #root_fn

            #auto_head_handler

            #[doc(hidden)]
            fn #wrap_func_name2(req: &mut potato::HttpRequest) -> potato::HttpResponse {
                #wrap_func_body
            }

            potato::inventory::submit!{potato::RequestHandlerFlag::new(
                potato::HttpMethod::#req_name,
                #route_path,
                potato::HttpHandler::Sync(#wrap_func_name2),
                potato::RequestHandlerFlagDoc::new(#doc_show, #doc_auth, #doc_summary, #doc_desp, #doc_args)
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

#[proc_macro_attribute]
pub fn preprocess(attr: TokenStream, input: TokenStream) -> TokenStream {
    preprocess_macro(attr, input)
}

#[proc_macro_attribute]
pub fn postprocess(attr: TokenStream, input: TokenStream) -> TokenStream {
    postprocess_macro(attr, input)
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
