use std::any::Any;

use proc_macro::TokenStream;
use proc_macro2::{Ident, Span};
use quote::{quote, ToTokens};
use rand::Rng;
use syn::{parse_macro_input, FnArg, ItemFn, LitStr};

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

#[proc_macro_attribute]
pub fn http_get(attr: TokenStream, input: TokenStream) -> TokenStream {
    let route_path = parse_macro_input!(attr as LitStr);
    let root_fn = parse_macro_input!(input as ItemFn);
    let fn_name = root_fn.sig.ident.clone();
    let wrap_func_name = random_ident();
    let mut arg_types = vec![];
    for arg in root_fn.sig.inputs.iter() {
        if let FnArg::Typed(arg) = arg {
            let arg_type = arg.ty.as_ref().to_token_stream().to_string();
            arg_types.push(arg_type);
        } else {
            panic!("unsupported: {}", arg.to_token_stream().to_string());
        }
    }
    quote! {
        #root_fn

        #[doc(hidden)]
        fn #wrap_func_name(req: potato::HttpRequest, conn: &mut potato::HttpConnection) ->
            std::pin::Pin<Box<dyn std::future::Future<Output = potato::HttpResponse> + Send + 'static>> {
            Box::pin(#fn_name(req))
        }

        potato::inventory::submit!{potato::RequestHandlerFlag::new(
            potato::HttpMethod::GET, #route_path, #wrap_func_name
        )}
    }.into()
}

macro_rules! define_handler_macro {
    ($fn_name:ident, $method:ident) => {
        #[proc_macro_attribute]
        pub fn $fn_name(attr: TokenStream, input: TokenStream) -> TokenStream {
            let route_path = parse_macro_input!(attr as LitStr);
            let root_fn = parse_macro_input!(input as ItemFn);
            let fn_name = root_fn.sig.ident.clone();
            let wrap_func_name = random_ident();
            quote! {
                #root_fn

                #[doc(hidden)]
                fn #wrap_func_name(req: potato::HttpRequest, conn: &mut potato::HttpConnection) ->
                    std::pin::Pin<Box<dyn std::future::Future<Output = potato::HttpResponse> + Send + 'static>> {
                    Box::pin(#fn_name(req))
                }

                potato::inventory::submit!{potato::RequestHandlerFlag::new(
                    potato::HttpMethod::$method, #route_path, #wrap_func_name
                )}
            }.into()
        }
    };
}

//define_handler_macro!(http_get, GET);
define_handler_macro!(http_post, POST);
define_handler_macro!(http_put, PUT);
define_handler_macro!(http_delete, DELETE);
define_handler_macro!(http_options, OPTIONS);
define_handler_macro!(http_head, HEAD);
