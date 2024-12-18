use proc_macro::TokenStream;
use proc_macro2::{Ident, Span};
use quote::quote;
use rand::Rng;
use syn::{parse_macro_input, ItemFn, LitStr};

fn random_ident() -> Ident {
    let mut rng = rand::thread_rng();
    let value = format!("__potato_id_{}", rng.gen::<u64>());
    Ident::new(&value, Span::call_site())
}

// #[proc_macro_attribute]
// pub fn http_get(attr: TokenStream, input: TokenStream) -> TokenStream {
//     let route_path = parse_macro_input!(attr as LitStr);
//     let root_fn = parse_macro_input!(input as ItemFn);
//     let fn_name = root_fn.sig.ident.clone();
//     let wrap_func_name = random_ident();
//     quote! {
//         #root_fn

//         fn #wrap_func_name(_ctx: potato::RequestContext) ->
//             std::pin::Pin<Box<dyn std::future::Future<Output = potato::HttpResponse> + Send + 'static>> {
//             Box::pin(#fn_name(_ctx))
//         }

//         potato::inventory::submit!{potato::RequestHandlerFlag::new(
//             potato::HttpMethod::GET, #route_path, #wrap_func_name
//         )}
//     }.into()
// }

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

                fn #wrap_func_name(_ctx: potato::RequestContext) ->
                    std::pin::Pin<Box<dyn std::future::Future<Output = potato::HttpResponse> + Send + 'static>> {
                    Box::pin(#fn_name(_ctx))
                }

                potato::inventory::submit!{potato::RequestHandlerFlag::new(
                    potato::HttpMethod::$method, #route_path, #wrap_func_name
                )}
            }.into()
        }
    };
}

define_handler_macro!(http_get, GET);
define_handler_macro!(http_post, POST);
define_handler_macro!(http_put, PUT);
define_handler_macro!(http_delete, DELETE);
define_handler_macro!(http_options, OPTIONS);
define_handler_macro!(http_head, HEAD);
