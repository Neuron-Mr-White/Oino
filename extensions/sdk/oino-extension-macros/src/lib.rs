//! Proc macros for the Oino guest SDK.
//!
//! `#[oino_extension::tool]` on an `async fn name(&'static self, args: A, cx: ToolCtx) ->
//! Result<ToolResult>` inside an `impl` block generates a sibling `fn name_tool() -> Tool<Self>`
//! whose description is the doc comment and whose parameters come from `A: JsonSchema`.
//!
//! ```ignore
//! impl Notify {
//!     /// Send a notification to the user.
//!     #[oino_extension::tool(label = "Notify user", kind = Other)]
//!     async fn notify_user(&'static self, args: NotifyArgs, cx: ToolCtx) -> Result<ToolResult> { … }
//! }
//! // in init: cx.tool(Self::notify_user_tool());
//! ```

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::parse::Parser;
use syn::punctuated::Punctuated;
use syn::{Expr, ExprLit, FnArg, ImplItemFn, Lit, Meta, Pat, Token};

/// Generates `fn <name>_tool() -> ::oino_extension::tool::Tool<Self>` for a tool method.
///
/// Arguments: `name = "..."` (default: fn name), `label = "..."`, `kind = Read|Edit|…`.
#[proc_macro_attribute]
pub fn tool(attr: TokenStream, item: TokenStream) -> TokenStream {
    match expand(attr.into(), item.into()) {
        Ok(ts) => ts.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

fn lit_str(expr: &Expr) -> syn::Result<String> {
    match expr {
        Expr::Lit(ExprLit {
            lit: Lit::Str(s), ..
        }) => Ok(s.value()),
        other => Err(syn::Error::new_spanned(other, "expected a string literal")),
    }
}

fn expand(
    attr: proc_macro2::TokenStream,
    item: proc_macro2::TokenStream,
) -> syn::Result<proc_macro2::TokenStream> {
    let func: ImplItemFn = syn::parse2(item)?;
    let args = Punctuated::<Meta, Token![,]>::parse_terminated.parse2(attr)?;
    let fn_name = func.sig.ident.clone();
    let mut tool_name = fn_name.to_string();
    let mut label = None;
    let mut kind = None;
    for meta in &args {
        let Meta::NameValue(nv) = meta else {
            return Err(syn::Error::new_spanned(meta, "expected `key = value`"));
        };
        let key = nv
            .path
            .get_ident()
            .map(ToString::to_string)
            .unwrap_or_default();
        match key.as_str() {
            "name" => tool_name = lit_str(&nv.value)?,
            "label" => label = Some(lit_str(&nv.value)?),
            "kind" => kind = Some(nv.value.clone()),
            _ => {
                return Err(syn::Error::new_spanned(
                    &nv.path,
                    "unknown key (name, label, kind)",
                ));
            }
        }
    }
    let doc: Vec<String> = func
        .attrs
        .iter()
        .filter(|a| a.path().is_ident("doc"))
        .filter_map(|a| match &a.meta {
            Meta::NameValue(nv) => lit_str(&nv.value).ok().map(|s| s.trim().to_owned()),
            _ => None,
        })
        .collect();
    let description = doc.join("\n").trim().to_owned();
    let inputs: Vec<&FnArg> = func.sig.inputs.iter().collect();
    let arg_ty = match inputs.get(1) {
        Some(FnArg::Typed(pt)) if matches!(&*pt.pat, Pat::Ident(_)) => pt.ty.clone(),
        _ => {
            return Err(syn::Error::new_spanned(
                &func.sig,
                "expected `(&'static self, args: A, cx: ToolCtx)`",
            ));
        }
    };
    let builder = format_ident!("{}_tool", fn_name);
    let label = label.map(|l| quote!(.label(#l)));
    let kind = kind.map(|k| quote!(.kind(::oino_extension::tool::ToolKind::#k)));
    Ok(quote! {
        #func

        #[doc = concat!("Tool definition generated for [`Self::", stringify!(#fn_name), "`].")]
        fn #builder() -> ::oino_extension::tool::Tool<Self> {
            ::oino_extension::tool::Tool::new(#tool_name)
                .description(#description)
                #label
                #kind
                .args::<#arg_ty>()
                .run(|ext: &'static Self, args: #arg_ty, cx| ext.#fn_name(args, cx))
        }
    })
}
