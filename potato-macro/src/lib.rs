use lazy_static::lazy_static;
use proc_macro::TokenStream;
use proc_macro2::{Ident, Span};
use quote::{quote, ToTokens};
use rand::Rng;
use serde_json::json;
use std::collections::HashSet;
use syn::{parse_macro_input, FnArg, ItemFn, LitStr};

lazy_static! {
    static ref ARG_TYPES: HashSet<&'static str> = [
        "String", "bool", "u8", "u16", "u32", "u64", "usize", "i8", "i16", "i32", "i64", "f32",
        "f64", "isize",
    ]
    .into_iter()
    .collect();
}

fn random_ident() -> Ident {
    let mut rng = rand::thread_rng();
    let value = format!("__potato_id_{}", rng.gen::<u64>());
    Ident::new(&value, Span::call_site())
}

fn http_handler_macro(attr: TokenStream, input: TokenStream, req_name: &str) -> TokenStream {
    let req_name = Ident::new(req_name, Span::call_site());
    let route_path = parse_macro_input!(attr as LitStr);
    if !route_path.value().starts_with('/') {
        panic!("route path must start with '/'");
    }
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
                "PostFile" => quote! {
                    match req.body_files.get(#arg_name_str).cloned() {
                        Some(file) => file,
                        None => return HttpResponse::error(format!("miss arg: {}", #arg_name_str)),
                    }
                },
                arg_type_str if ARG_TYPES.contains(arg_type_str) => {
                    doc_args.push(json!({ "name": arg_name_str, "type": arg_type_str }));
                    let mut arg_value = quote! {
                        match req.body_pairs.get(#arg_name_str).cloned() {
                            Some(val) => val,
                            None => match req.url_query.get(#arg_name_str).cloned() {
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
                },
                _ => panic!("unsupported arg type: [{}]", arg_type_str),
            });
        } else {
            panic!("unsupported: {}", arg.to_token_stream().to_string());
        }
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
                Ok(ret) => HttpResponse::empty(),
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
            HttpResponse::empty()
        },
        "HttpResponse" => quote! {
            #fn_name(#(#args),*).await
        },
        _ => panic!("unsupported ret type: {}", ret_type),
    };
    let doc_args = serde_json::to_string(&doc_args).unwrap();
    //let mut content =
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
            potato::RequestHandlerFlagDoc::new(#doc_show, false, #doc_summary, #doc_desp, #doc_args)
        )}
    }.into()
    // }.to_string();
    // panic!("{}", content);
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

#[proc_macro]
pub fn declare_doc_path(input: TokenStream) -> TokenStream {
    let doc = parse_macro_input!(input as LitStr).value();
    if !doc.starts_with('/') {
        panic!("route path must start with '/'");
    }
    if !doc.ends_with('/') {
        panic!("route path must ends with '/'");
    }
    let doc_index_htm = format!("{doc}index.htm");
    let doc_index_html = format!("{doc}index.html");
    let doc_index_css = format!("{doc}index.css");
    let doc_swagger_ui_css = format!("{doc}swagger-ui.css");
    let doc_swagger_ui_bundle_js = format!("{doc}swagger-ui-bundle.js");
    let doc_swagger_ui_standalone_preset_js = format!("{doc}swagger-ui-standalone-preset.js");
    let doc_swagger_initializer_js = format!("{doc}swagger-initializer.js");
    //let doc_favicon_32x32_png = format!("{doc}favicon-32x32.png");
    //let doc_favicon_16x16_png = format!("{doc}favicon-16x16.png");
    let doc_index_json = format!("{doc}index.json");

    quote! {
        #[doc(hidden)]
        #[http_get(#doc)]
        #[http_get(#doc_index_htm)]
        #[http_get(#doc_index_html)]
        async fn doc_index() -> HttpResponse {
            HttpResponse::html(potato::DocResource::load_str("index.html"))
        }

        #[doc(hidden)]
        #[http_get(#doc_index_css)]
        async fn doc_index_css() -> HttpResponse {
            HttpResponse::css(potato::DocResource::load_str("index.css"))
        }

        #[doc(hidden)]
        #[http_get(#doc_swagger_ui_css)]
        async fn doc_swagger_ui_css() -> HttpResponse {
            HttpResponse::css(potato::DocResource::load_str("swagger-ui.css"))
        }

        #[doc(hidden)]
        #[http_get(#doc_swagger_ui_bundle_js)]
        async fn doc_swagger_ui_bundle_js() -> HttpResponse {
            HttpResponse::js(potato::DocResource::load_str("swagger-ui-bundle.js"))
        }

        #[doc(hidden)]
        #[http_get(#doc_swagger_ui_standalone_preset_js)]
        async fn doc_swagger_ui_standalone_preset_js() -> HttpResponse {
            HttpResponse::js(potato::DocResource::load_str("swagger-ui-standalone-preset.js"))
        }

        #[doc(hidden)]
        #[http_get(#doc_swagger_initializer_js)]
        async fn doc_swagger_initializer_js() -> HttpResponse {
            HttpResponse::js(potato::DocResource::load_str("swagger-initializer.js"))
        }

        // #[doc(hidden)]
        // #[http_get(#doc_favicon_32x32_png)]
        // async fn doc_favicon_32x32_png() -> HttpResponse {
        //     HttpResponse::png(include_bytes!("../swagger_res/favicon-32x32.png"))
        // }

        // #[doc(hidden)]
        // #[http_get(#doc_favicon_16x16_png)]
        // async fn doc_favicon_16x16_png() -> HttpResponse {
        //     HttpResponse::png(include_bytes!("../swagger_res/favicon-16x16.png"))
        // }

        #[doc(hidden)]
        #[http_get(#doc_index_json)]
        async fn doc_doc_json() -> HttpResponse {
            let mut any_use_auth = false;
            let contact = {
                let re = potato::regex::Regex::new(r"([[:word:]]+)\s*<([^>]+)>").unwrap();
                match re.captures(env!("CARGO_PKG_AUTHORS")) {
                    Some(caps) => {
                        let name = caps.get(1).map_or("", |m| m.as_str());
                        let email = caps.get(2).map_or("", |m| m.as_str());
                        potato::serde_json::json!({ "name": name, "email": email })
                    }
                    None => potato::serde_json::json!({}),
                }
            };
            let paths = {
                let mut paths = std::collections::HashMap::new();
                for flag in inventory::iter::<RequestHandlerFlag> {
                    if !flag.doc.show {
                        continue;
                    }
                    let mut root_cur_path = potato::serde_json::json!({
                        "summary": flag.doc.summary,
                        "description": flag.doc.desp,
                        "responses": {
                            "200": { "description": "OK" },
                            "500": { "description": "Internal Error" },
                        },
                    });
                    let mut arg_pairs = {
                        let mut arg_pairs = vec![];
                        if let Ok(args) = potato::serde_json::from_str::<potato::serde_json::Value>(flag.doc.args) {
                            if let Some(args) = args.as_array() {
                                for arg in args.iter() {
                                    let arg_name = arg["name"].as_str().unwrap_or("");
                                    let arg_type = {
                                        let arg_type = arg["type"].as_str().unwrap_or("");
                                        match arg_type.starts_with('i') || arg_type.starts_with('u') {
                                            true => "number",
                                            false => "string"
                                        }
                                    };
                                    arg_pairs.push((arg_name.to_string(), arg_type.to_string()));
                                }
                            }
                        }
                        arg_pairs
                    };
                    if !arg_pairs.is_empty() {
                        if flag.method == potato::HttpMethod::GET {
                            let mut parameters = vec![];
                            for (arg_name, arg_type) in arg_pairs.iter() {
                                parameters.push(potato::serde_json::json!({
                                    "name": arg_name,
                                    "in": "query",
                                    "description": "",
                                    "required": true,
                                    "schema": { "type": arg_type },
                                }));
                            }
                            root_cur_path["parameters"] = potato::serde_json::Value::Array(parameters);
                        } else {
                            let mut properties = potato::serde_json::json!({});
                            let mut required = vec![];
                            for (arg_name, arg_type) in arg_pairs.iter() {
                                properties[arg_name] = potato::serde_json::json!({ "type": arg_type });
                                required.push(arg_name);
                            }
                            root_cur_path["requestBody"]["content"] = potato::serde_json::json!({
                                "application/x-www-form-urlencoded": {
                                    "schema": {
                                        "type": "object",
                                        "properties": properties,
                                        "required": required
                                    }
                                }
                            });
                        }
                    }
                    if flag.doc.auth {
                        root_cur_path["security"] = potato::serde_json::json!([{ "bearerAuth": [] }]);
                        any_use_auth = true;
                    }
                    paths.entry(flag.path).or_insert_with(std::collections::HashMap::new)
                        .insert(flag.method.to_string().to_lowercase(), root_cur_path);
                }
                paths
            };
            let mut root = potato::serde_json::json!({
                "openapi": "3.1.0",
                "info": {
                    "title": env!("CARGO_PKG_NAME"),
                    "version": env!("CARGO_PKG_VERSION"),
                    "description": env!("CARGO_PKG_DESCRIPTION"),
                    "contact": contact,
                },
                "paths": paths,
            });
            if any_use_auth {
                root["components"]["securitySchemes"]["bearerAuth"] = potato::serde_json::json!({
                    "description": "Bearer token using a JWT",
                    "type": "http",
                    "scheme": "Bearer",
                    "bearerFormat": "JWT",
                });
            }
            HttpResponse::json(potato::serde_json::to_string(&root).unwrap_or("{}".to_string()))
        }
    }
    .into()
}

trait StringExt {
    fn type_simplify(&self) -> String;
}

impl StringExt for String {
    fn type_simplify(&self) -> String {
        self.replace("potato :: ", "")
            .replace("std :: ", "")
            .replace("net :: ", "")
            .replace("anyhow :: ", "")
            .replace("-> ", "")
    }
}
