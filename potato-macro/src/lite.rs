use proc_macro2::TokenStream;
use quote::{format_ident, quote, ToTokens};
use syn::ItemFn;

fn type_simplify(s: &str) -> String {
    s.replace(" :: ", "::")
        .replace(" <", "<")
        .replace("< ", "<")
        .replace(" >", ">")
        .replace("> ", ">")
}

/// Generates the lightweight route wrapper for no_std mode.
pub fn lite_http_handler(attr: TokenStream, input: TokenStream, method: &str) -> TokenStream {
    let root_fn = match syn::parse2::<ItemFn>(input) {
        Ok(root_fn) => root_fn,
        Err(err) => return err.to_compile_error(),
    };
    let crate_path = quote!(::potato);
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

    let route_path = {
        let tokens: Vec<_> = attr.into_iter().collect();
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
    let call_expr = if has_request_arg {
        if is_async {
            quote! { #fn_name(req).await }
        } else {
            quote! { #fn_name(req) }
        }
    } else {
        if is_async {
            quote! { #fn_name().await }
        } else {
            quote! { #fn_name() }
        }
    };
    let wrapper_body = if is_async {
        quote! {
            Some(#crate_path::block_on(async move { #call_expr }))
        }
    } else {
        quote! {
            Some(#call_expr)
        }
    };

    quote! {
        #root_fn

        #[doc(hidden)]
        #[allow(unused_variables)]
        fn #wrap_func_name(req: &mut #crate_path::HttpRequest) -> Option<#crate_path::HttpResponse> {
            #wrapper_body
        }

        #[doc(hidden)]
        #[::linkme::distributed_slice(#crate_path::ROUTE_HANDLERS)]
        static #route_info_name: #crate_path::RouteHandler = #crate_path::RouteHandler {
            method: #method,
            path: #route_path_static,
            handler: #wrap_func_name,
        };
    }
}
