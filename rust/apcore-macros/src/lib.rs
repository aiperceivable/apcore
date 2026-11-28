use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::meta::ParseNestedMeta;
use syn::visit_mut::{self, VisitMut};
use syn::{
    parse_macro_input, parse_quote, Error, Expr, FnArg, GenericArgument, ItemFn, Lit, LitBool,
    LitStr, Meta, Path, PathArguments, ReturnType, Type, Visibility,
};

/// Declare an async typed APCore module and its registration and invocation handles.
///
/// The handler accepts an input and optionally an immutable execution context.
/// It returns `Result<Output, apcore::ModuleError>`. The input and output use
/// Serde and Schemars 0.8. For inputs, Serde's serialized representation must
/// include the arguments the business code relies on.
///
/// Supply `id = "group.name"` and either a doc comment or an explicit
/// `description`. The macro creates a sibling module named after the function
/// with `DECLARATION` and `HANDLE` constants.
#[proc_macro_attribute]
pub fn module(args: TokenStream, item: TokenStream) -> TokenStream {
    let mut options = Options::default();
    let parser = syn::meta::parser(|meta| options.parse(&meta));
    parse_macro_input!(args with parser);
    let function = parse_macro_input!(item as ItemFn);
    expand(options, &function)
        .unwrap_or_else(|error| error.into_compile_error())
        .into()
}

#[derive(Default)]
struct Options {
    id: Option<LitStr>,
    description: Option<LitStr>,
    sdk: Option<Path>,
    readonly: Option<LitBool>,
    destructive: Option<LitBool>,
    idempotent: Option<LitBool>,
    open_world: Option<LitBool>,
    requires_approval: Option<LitBool>,
    discoverable: Option<LitBool>,
}

impl Options {
    fn parse(&mut self, meta: &ParseNestedMeta<'_>) -> syn::Result<()> {
        match meta.path.get_ident().map(ToString::to_string).as_deref() {
            Some("id") => parse_option(meta, &mut self.id),
            Some("description") => parse_option(meta, &mut self.description),
            Some("crate") => parse_option(meta, &mut self.sdk),
            Some("readonly") => parse_option(meta, &mut self.readonly),
            Some("destructive") => parse_option(meta, &mut self.destructive),
            Some("idempotent") => parse_option(meta, &mut self.idempotent),
            Some("open_world") => parse_option(meta, &mut self.open_world),
            Some("requires_approval") => parse_option(meta, &mut self.requires_approval),
            Some("discoverable") => parse_option(meta, &mut self.discoverable),
            _ => Err(meta.error("unsupported APCore module option")),
        }
    }

    fn annotations(&self) -> impl Iterator<Item = TokenStream2> + '_ {
        [
            ("readonly", self.readonly.as_ref()),
            ("destructive", self.destructive.as_ref()),
            ("idempotent", self.idempotent.as_ref()),
            ("open_world", self.open_world.as_ref()),
            ("requires_approval", self.requires_approval.as_ref()),
            ("discoverable", self.discoverable.as_ref()),
        ]
        .into_iter()
        .filter_map(|(name, value)| {
            value.map(|value| {
                let field = syn::Ident::new(name, value.span());
                quote!(#field: #value,)
            })
        })
    }
}

fn parse_option<T: syn::parse::Parse>(
    meta: &ParseNestedMeta<'_>,
    target: &mut Option<T>,
) -> syn::Result<()> {
    if target.is_some() {
        return Err(meta.error("duplicate APCore module option"));
    }
    *target = Some(meta.value()?.parse()?);
    Ok(())
}

fn expand(options: Options, function: &ItemFn) -> syn::Result<TokenStream2> {
    let signature = &function.sig;
    let name = &signature.ident;
    if signature.asyncness.is_none()
        || signature.unsafety.is_some()
        || signature.abi.is_some()
        || !signature.generics.params.is_empty()
        || signature.generics.where_clause.is_some()
    {
        return Err(Error::new_spanned(
            signature,
            "APCore module handler must be a safe, non-generic async function",
        ));
    }
    let (mut input_type, context_arg) = match signature.inputs.iter().collect::<Vec<_>>().as_slice()
    {
        [FnArg::Typed(input)] => ((*input.ty).clone(), quote!()),
        [FnArg::Typed(input), FnArg::Typed(_)] => ((*input.ty).clone(), quote!(, context)),
        _ => {
            return Err(Error::new_spanned(
                signature,
                "APCore module handler takes an input and an optional &Context",
            ));
        }
    };
    let mut output_type = result_output(&signature.output)?.clone();
    MoveIntoChild.visit_type_mut(&mut input_type);
    MoveIntoChild.visit_type_mut(&mut output_type);

    let id = options
        .id
        .as_ref()
        .filter(|id| !id.value().is_empty())
        .ok_or_else(|| Error::new(name.span(), "APCore module requires id = \"group.name\""))?;
    let (description, documentation) = documentation(options.description.as_ref(), function)?;
    let annotations = options.annotations();
    let mut sdk: Path = options
        .sdk
        .clone()
        .unwrap_or_else(|| parse_quote!(::apcore));
    MoveIntoChild.visit_path_mut(&mut sdk);
    let visibility = &function.vis;
    let item_visibility = if matches!(visibility, Visibility::Inherited) {
        quote!(pub(super))
    } else {
        quote!(pub)
    };
    let cfg = function
        .attrs
        .iter()
        .filter(|attr| attr.path().is_ident("cfg"));

    Ok(quote! {
        #function

        #(#cfg)*
        #[doc = "Typed handle and registration for this APCore module."]
        #visibility mod #name {
            use super::*;

            #[doc = "Registration for this module."]
            #item_visibility const DECLARATION: #sdk::ModuleDeclaration =
                #sdk::ModuleDeclaration::new(#id, build_module);

            #[doc = "Governed invocation through the registered module ID."]
            #item_visibility const HANDLE: #sdk::TypedHandle<#input_type, #output_type> =
                #sdk::TypedHandle::new(#id);

            #[allow(clippy::result_large_err)]
            fn build_module() -> ::core::result::Result<
                ::std::boxed::Box<dyn #sdk::Module>,
                #sdk::ModuleError,
            > {
                #sdk::__declare_module::<#input_type, #output_type>(
                    #description,
                    #documentation,
                    #sdk::ModuleAnnotations {
                        #(#annotations)*
                        ..::core::default::Default::default()
                    },
                    |input, context| ::std::boxed::Box::pin(super::#name(input #context_arg)),
                )
            }
        }
    })
}

fn result_output(output: &ReturnType) -> syn::Result<&Type> {
    let error = || {
        Error::new_spanned(
            output,
            "APCore module handler must return Result<Output, ModuleError>",
        )
    };
    let ReturnType::Type(_, output) = output else {
        return Err(error());
    };
    let Type::Path(path) = output.as_ref() else {
        return Err(error());
    };
    let segment = path
        .path
        .segments
        .last()
        .filter(|segment| segment.ident == "Result")
        .ok_or_else(error)?;
    let PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return Err(error());
    };
    match arguments.args.first() {
        Some(GenericArgument::Type(output)) => Ok(output),
        _ => Err(error()),
    }
}

fn documentation(
    description: Option<&LitStr>,
    function: &ItemFn,
) -> syn::Result<(LitStr, TokenStream2)> {
    let docs = function.attrs.iter().filter_map(|attr| {
        let Meta::NameValue(meta) = &attr.meta else {
            return None;
        };
        if meta.path.is_ident("doc") {
            if let Expr::Lit(expr) = &meta.value {
                if let Lit::Str(line) = &expr.lit {
                    let line = line.value();
                    return Some(line.strip_prefix(' ').unwrap_or(&line).to_owned());
                }
            }
        }
        None
    });
    let docs = docs.collect::<Vec<_>>().join("\n");
    let docs = docs.trim();
    let description = match description {
        Some(description) => description.value(),
        None => docs
            .lines()
            .take_while(|line| !line.trim().is_empty())
            .map(str::trim)
            .collect::<Vec<_>>()
            .join(" "),
    };
    if description.trim().is_empty() {
        return Err(Error::new(
            function.sig.ident.span(),
            "APCore module requires a doc comment or a nonempty description",
        ));
    }
    let span = function.sig.ident.span();
    let description = LitStr::new(description.trim(), span);
    let documentation = if docs.is_empty() {
        quote!(::core::option::Option::None)
    } else {
        let docs = LitStr::new(docs, span);
        quote!(::core::option::Option::Some(#docs))
    };
    Ok((description, documentation))
}

struct MoveIntoChild;

impl VisitMut for MoveIntoChild {
    fn visit_path_mut(&mut self, path: &mut Path) {
        if let Some(first) = path.segments.first_mut() {
            if first.ident == "self" {
                first.ident = syn::Ident::new("super", first.ident.span());
            } else if first.ident == "super" {
                path.segments.insert(0, parse_quote!(super));
            }
        }
        visit_mut::visit_path_mut(self, path);
    }
}
