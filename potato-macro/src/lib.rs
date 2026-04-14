mod utils;

use proc_macro::TokenStream;
use proc_macro2::{Ident, Span};
use quote::{format_ident, quote, ToTokens};
use rand::Rng;
use serde_json::json;
use std::{collections::HashSet, sync::LazyLock};
use utils::StringExt as _;

static ARG_TYPES: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    [
        "String", "bool", "u8", "u16", "u32", "u64", "usize", "i8", "i16", "i32", "i64", "isize",
        "f32", "f64",
    ]
    .into_iter()
    .collect()
});

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
    let (route_path, oauth_arg) = {
        let mut oroute_path = syn::parse::<syn::LitStr>(attr.clone())
            .ok()
            .map(|path| path.value());
        let mut oauth_arg = None;
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
        (route_path, oauth_arg)
    };
    let mut root_fn = syn::parse_macro_input!(input as syn::ItemFn);
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

            __potato_response
        }
    };
    let doc_args = serde_json::to_string(&doc_args).unwrap();
    if is_async {
        quote! {
            #root_fn

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
