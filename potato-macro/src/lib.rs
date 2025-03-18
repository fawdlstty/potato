use proc_macro::TokenStream;
use proc_macro2::{Ident, Span};
use quote::{quote, ToTokens};
use rand::Rng;
use serde_json::json;
use std::{collections::HashSet, sync::LazyLock};
use syn::{parse_macro_input, FnArg, ItemFn, LitStr};

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
    let value = format!("__potato_id_{}", rng.gen::<u64>());
    Ident::new(&value, Span::call_site())
}

fn http_handler_macro(attr: TokenStream, input: TokenStream, req_name: &str) -> TokenStream {
    let req_name = Ident::new(req_name, Span::call_site());
    let (route_path, oauth_arg) = {
        let mut oroute_path = syn::parse::<LitStr>(attr.clone())
            .ok()
            .map(|path| path.value());
        let mut oauth_arg = None;
        //
        if oroute_path.is_none() {
            let http_parser = syn::meta::parser(|meta| {
                if meta.path.is_ident("path") {
                    if let Ok(arg) = meta.value() {
                        if let Ok(route_path) = arg.parse::<LitStr>() {
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
            parse_macro_input!(attr with http_parser);
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
    let root_fn = parse_macro_input!(input as ItemFn);
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
    let wrap_func_name = random_ident();
    let mut args = vec![];
    let mut doc_args = vec![];
    let mut arg_auth_mark = false;
    for arg in root_fn.sig.inputs.iter() {
        if let FnArg::Typed(arg) = arg {
            let arg_type_str = arg
                .ty
                .as_ref()
                .to_token_stream()
                .to_string()
                .type_simplify();
            let arg_name_str = arg.pat.to_token_stream().to_string();
            args.push(match &arg_type_str[..] {
                "HttpRequest" => quote! { req },
                "SocketAddr" => quote! { client },
                "& mut WebsocketContext" => quote! { wsctx },
                "PostFile" => {
                    doc_args.push(json!({ "name": arg_name_str, "type": arg_type_str }));
                    quote! {
                        match req.body_files.get(&potato::utils::refstr::RefStr::from_str(#arg_name_str)).cloned() {
                            Some(file) => file,
                            None => return HttpResponse::error(format!("miss arg: {}", #arg_name_str)),
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
                        quote! {
                            match req.headers
                                .get(&potato::utils::refstr::HeaderRefStr::from_str("Authorization"))
                                .map(|v| v.to_str()) {
                                Some(mut auth) => {
                                    if auth.starts_with("Bearer ") {
                                        auth = &auth[7..];
                                    }
                                    match potato::ServerAuth::jwt_check(&auth).await {
                                        Ok(payload) => payload,
                                        Err(err) => return HttpResponse::error(format!("auth failed: {:?}", err)),
                                    }
                                }
                                None => return HttpResponse::error("miss header : Authorization"),
                            }
                        }
                    } else {
                        doc_args.push(json!({ "name": arg_name_str, "type": arg_type_str }));
                        let mut arg_value = quote! {
                            match req.body_pairs
                                .get(&potato::utils::refstr::RefStrOrString::from_str(#arg_name_str))
                                .map(|p| p.to_string()) {
                                Some(val) => val,
                                None => match req.url_query
                                    .get(&potato::utils::refstr::RefStr::from_str(#arg_name_str))
                                    .map(|p| p.to_str().to_string()) {
                                    Some(val) => val,
                                    None => return HttpResponse::error(format!("miss arg: {}", #arg_name_str)),
                                },
                            }
                        };
                        if arg_type_str != "String" {
                            arg_value = quote! {
                                match #arg_value.parse() {
                                    Ok(val) => val,
                                    Err(err) => return HttpResponse::error(format!("arg[{}] is not {} type", #arg_name_str, #arg_type_str)),
                                }
                            }
                        }
                        arg_value
                    }
                },
                _ => panic!("unsupported arg type: [{arg_type_str}]"),
            });
        } else {
            panic!("unsupported: {}", arg.to_token_stream().to_string());
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
    let wrap_func_body = match &ret_type[..] {
        "Result < () >" => quote! {
            match #fn_name(#(#args),*).await {
                Ok(ret) => HttpResponse::text("ok"),
                Err(err) => HttpResponse::error(format!("{err:?}")),
            }
        },
        "Result < HttpResponse >" => quote! {
            match #fn_name(#(#args),*).await {
                Ok(ret) => ret,
                Err(err) => HttpResponse::error(format!("{err:?}")),
            }
        },
        "()" => quote! {
            #fn_name(#(#args),*).await;
            HttpResponse::text("ok")
        },
        "HttpResponse" => quote! {
            #fn_name(#(#args),*).await
        },
        _ => panic!("unsupported ret type: {ret_type}"),
    };
    let doc_args = serde_json::to_string(&doc_args).unwrap();
    // let mut content =
    quote! {
        #root_fn

        #[doc(hidden)]
        async fn #wrap_func_name2(
            req: potato::HttpRequest, client: std::net::SocketAddr, wsctx: &mut potato::WebsocketContext
        ) -> potato::HttpResponse {
            #wrap_func_body
        }

        #[doc(hidden)]
        fn #wrap_func_name<'a>(
            req: potato::HttpRequest, client: std::net::SocketAddr, wsctx: &'a mut potato::WebsocketContext
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = potato::HttpResponse> + Send + 'a>> {
            Box::pin(#wrap_func_name2(req, client, wsctx))
        }

        potato::inventory::submit!{potato::RequestHandlerFlag::new(
            potato::HttpMethod::#req_name,
            #route_path,
            #wrap_func_name,
            potato::RequestHandlerFlagDoc::new(#doc_show, #doc_auth, #doc_summary, #doc_desp, #doc_args)
        )}
    }.into()
    // }.to_string();
    // panic!("{content}");
    // todo!()
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

trait StringExt {
    fn type_simplify(&self) -> String;
}

impl StringExt for String {
    fn type_simplify(&self) -> String {
        let ret = self
            .replace("potato :: ", "")
            .replace("std :: ", "")
            .replace("net :: ", "")
            .replace("anyhow :: ", "")
            .replace("-> ", "");
        match ret.is_empty() {
            true => "()".to_string(),
            false => ret,
        }
    }
}

#[proc_macro]
pub fn embed_dir(input: TokenStream) -> TokenStream {
    let path = parse_macro_input!(input as LitStr).value();
    quote! {{
        #[derive(potato::rust_embed::Embed)]
        #[folder = #path]
        struct Asset;

        potato::load_embed::<Asset>()
    }}
    .into()
}
