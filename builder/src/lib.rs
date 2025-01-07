use core::alloc;

use proc_macro::TokenStream;
use quote::quote;
use syn::{
    parse::{Parse, ParseStream},
    LitStr, Token,
};
use syn::{parse_macro_input, DeriveInput, Ident};

fn is_a(to_match_on: String, ty: &syn::Type) -> bool {
    if let Some(ident) = get_ident_from_type(ty) {
        return ident.to_string() == to_match_on;
    }

    false
}

fn get_ident_from_type(ty: &syn::Type) -> Option<syn::Ident> {
    if let syn::Type::Path(path) = ty {
        if let Some(ident) = &path.path.segments.first() {
            return Some(ident.ident.clone());
        }
    }
    None
}

fn get_in_angle_bracket(ty: &syn::Type) -> Option<syn::Ident> {
    if let syn::Type::Path(path) = ty {
        if let syn::PathArguments::AngleBracketed(angle) =
            &path.path.segments.first().unwrap().arguments
        {
            let args = angle.args.first().unwrap();
            if let syn::GenericArgument::Type(ty) = args {
                let ident = get_ident_from_type(ty).unwrap();

                return Some(ident);
            }
            return None;
        }
        return None;
    }
    None
}

#[derive(Debug)]
struct BuilderAttr {
    key: syn::Ident,
    _eq_token: Token![=],
    value: LitStr,
}

impl Parse for BuilderAttr {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        Ok(BuilderAttr {
            key: input.parse()?,
            _eq_token: input.parse()?,
            value: input.parse()?,
        })
    }
}

/// I don't want to think about, so why not
enum EachAttrResult {
    Success(String),
    None,
    Error(TokenStream),
}

fn find_each_attr(attribute: &[syn::Attribute]) -> EachAttrResult {
    if let Some(attr) = attribute.first() {
        if let syn::Meta::List(meta_list) = &attr.meta {
            if meta_list.path.is_ident("builder") {
                // Attempt to parse the tokens

                match syn::parse::<BuilderAttr>(meta_list.tokens.clone().into()) {
                    Ok(parsed) => {
                        let key = parsed.key.to_string();

                        if key != "each" {
                            let error = syn::Error::new_spanned(
                                meta_list,
                                "expected `builder(each = \"...\")`",
                            );

                            return EachAttrResult::Error(error.into_compile_error().into());
                        }

                        return EachAttrResult::Success(parsed.value.value());
                    }

                    Err(_) => {
                        let error = syn::Error::new_spanned(
                            meta_list,
                            "expected `builder(each = \"...\")`",
                        );

                        return EachAttrResult::Error(error.into_compile_error().into());
                    }
                }
            }
        }
    }

    EachAttrResult::None
}

#[proc_macro_derive(Builder, attributes(builder))]
pub fn derive(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    println!("{:#?}", input);

    let struct_ident = &input.ident;

    let builder_ident = Ident::new(&format!("{}Builder", struct_ident), struct_ident.span());

    let fields = match &input.data {
        syn::Data::Struct(data_struct) => match &data_struct.fields {
            syn::Fields::Named(fields) => &fields.named,
            _ => panic!("Only structs with named fields are supported"),
        },
        _ => panic!("Only structs are supported"),
    };
    // First, check for any errors in attributes

    let mut error_tokens = TokenStream::new();

    for field in fields {
        if is_a("Vec".into(), &field.ty) {
            if let EachAttrResult::Error(error) = find_each_attr(&field.attrs) {
                error_tokens.extend(error);
            }
        }
    }

    if !error_tokens.is_empty() {
        return error_tokens;
    }

    let builder_fields = fields.iter().map(|field| {
        let name = field.ident.as_ref().expect("Couldn't get the field");
        let ty = &field.ty;

        let is_option = is_a("Option".into(), ty);

        if is_option {
            quote! {
                #name: #ty
            }
        } else {
            quote! {
                #name: ::std::option::Option<#ty>
            }
        }
    });

    let builder_setters = fields.iter().map(|field| {
        let name = field.ident.as_ref().expect("Couldn't get the field");
        let ty = &field.ty;

        let is_option = is_a("Option".into(), ty);

        if is_option {
            let inner_ty = get_in_angle_bracket(ty).unwrap();
            quote! {
                pub fn #name(&mut self, #name: #inner_ty) -> &mut Self {
                    self.#name = ::std::option::Option::Some(#name);
                    self
                    }
            }
        } else {
            let each_attr = find_each_attr(&field.attrs);

            let each_attr = match each_attr {
                EachAttrResult::Success(s) => Some(s),
                EachAttrResult::None => None,
                _ => None,
            };

            let is_vec = is_a("Vec".into(), ty);

            if each_attr.is_some() && is_vec {
                let each_value = each_attr.unwrap();
                let each_ident = syn::Ident::new(&each_value, name.span());

                let inside_vec_value = get_in_angle_bracket(ty).unwrap();

                quote! {
                    pub fn #each_ident(&mut self, #each_ident: #inside_vec_value) -> &mut Self {
                        self.#name.get_or_insert_with(Vec::new).push(#each_ident);
                        self
                    }
                }
            } else {
                quote! {
                    pub fn #name(&mut self, #name: #ty) -> &mut Self {
                        self.#name = ::std::option::Option::Some(#name);
                        self
                    }
                }
            }
        }
    });

    let builder_body = fields.iter().map(|field| {
        let name = field.ident.as_ref().expect("Couldn't get the field");

        quote! {
            #name: None
        }
    });

    let build_body = fields.iter().map(|field| {
        let name = field.ident.as_ref().expect("Couldn't get the field");
        let name_str = name.to_string();

        let ty = &field.ty;

        let is_option = is_a("Option".into(), ty);
        let is_vec = is_a("Vec".into(), ty);

        if is_option {
            quote! {
                #name: self.#name.clone()
            }
        } else if is_vec {
            quote! {
                #name: self.#name.take().unwrap_or_default()
            }
        } else {
            quote! {
                #name: self.#name.take().ok_or_else(|| format!("Field '{}' is not set", #name_str))?
            }
        }
    });

    quote! {
        pub struct #builder_ident {
            #(#builder_fields,)*
        }

        impl #struct_ident {
            pub fn builder() -> #builder_ident {
                #builder_ident {
                    #(#builder_body,)*
                }
            }
        }

        impl #builder_ident {
            #(#builder_setters)*

            pub fn build(&mut self) -> ::std::result::Result<#struct_ident, ::std::boxed::Box<(dyn ::std::error::Error + 'static)>> {
                ::std::result::Result::Ok(#struct_ident {
                    #(#build_body,)*
                })
            }
        }
    }
    .into()
}
