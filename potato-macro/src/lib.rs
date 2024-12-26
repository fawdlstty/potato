use lazy_static::lazy_static;
use proc_macro::TokenStream;
use proc_macro2::{Ident, Span};
use quote::{quote, ToTokens};
use rand::Rng;
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

// ItemFn {
//     sig: Signature {
//         ident: Ident { ident: "hello", span: #0 bytes(312..317) },
//         inputs: [FnArg::Typed(
//             PatType {
//                 pat: Pat::Ident { ident: Ident { ident: "req", span: #0 bytes(318..321) }, },
//                 ty: Type::Path { path: Path { segments: [
//                     PathSegment {
//                         ident: Ident { ident: "HttpRequest", span: #0 bytes(323..334) },
//                         arguments: PathArguments::None
//                     }
//                 ] } }
//             }
//         )],
//         output: ReturnType::Type( Type::Path { path: Path { segments: [
//             PathSegment {
//                 ident: Ident { ident: "HttpResponse", span: #0 bytes(339..351) },
//                 arguments: PathArguments::None
//             }
//         ] } } )
//     },
// }

fn http_handler_macro(attr: TokenStream, input: TokenStream, req_name: &str) -> TokenStream {
    let req_name = Ident::new(req_name, Span::call_site());
    let route_path = parse_macro_input!(attr as LitStr);
    let root_fn = parse_macro_input!(input as ItemFn);
    let fn_name = root_fn.sig.ident.clone();
    let wrap_func_name = random_ident();
    let mut args = vec![];
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
            potato::HttpMethod::#req_name, #route_path, #wrap_func_name
        )}
    }.into()
    //}.to_string();
    //panic!("{}", content);
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
