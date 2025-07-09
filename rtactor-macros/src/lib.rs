//! Proc macros for the rtactor library.
//!
//! # Generate a Response enum from a Request enum with `derive(ResponseEnum)`
//! ```rs
//! #[derive(ResponseEnum)]
//! pub enum Request {
//!     SetValue{val: i32},
//!    
//!     #[response_val(i32)]
//!     GetValue{},
//! }
//! ```
//!
//! Will generate:
//! ```
//! pub enum Response
//! {
//!     SetValue(),
//!     GetValue(i32)
//! }
//! ```
//!
//! # Generate a synchronous access trait from a Notification enum with `SyncNotifier`
//!
//! ```rs
//! #[derive(SyncNotifier)]
//! pub enum Notification {
//!     TemperatureChanged{temp: float}
//! }
//! ```
//!
//! Will generate:
//!
//! ```rs
//! pub trait SyncNotifier : ::rtactor::SyncAccessData
//! {
//!  temperature_changed(&mut self, temp: float) -> Result<(), ::rtactor::Error> {[...]}
//! }
//! ```
//!
//! A structure can add the generated methods by deriving `SyncNotifier` and
//! implementing the methods of `SyncAccessData`. The macro `define_sync_accessor!()`
//! found in create `rtactor` can be used to generate a struct that
//! allows easy access with its internal ActiveActor:
//! ```rs
//! define_sync_accessor!(MyNotifSyncAccessor, SyncNotifier)
//!
//!
//! fn test(addr: rtactor::Addr)
//! {
//!     let accessor = MyNotifSyncAccessor::new(&addr);
//!     accessor.temperature_changed(13.2f32).unwrap();
//! }
//! ```
//!
//! # Generate a synchronous access trait from a Request enum with `SyncRequester`
//!
//! ```rs
//! #[derive(ResponseEnum, SyncRequester)]
//! pub enum Request {
//!     SetValue{val: i32},
//!    
//!     #[response_val(i32)]
//!     GetValue{},
//! }
//! ```
//!
//! Will generate for the `SyncRequester` part:
//! ```rs
//! pub trait SyncRequester: ::rtactor::SyncAccessor
//! {
//!     fn set_value(&mut self, val: i32) -> Result<(), ::rtactor::Error> {[...]}
//!     fn get_value(&mut self) -> Result<i32, ::rtactor::Error> {[...]}
//! }
//! ```
//!
//! A structure can add the generated methods by deriving `SyncRequester` and
//! implementing the methods of `SyncAccessor`. The macro `define_sync_notifier!()`
//! found in create `rtactor` can be used to generate a struct that
//! allows easy access with its internal ActiveActor:
//! ```rs
//! define_sync_accessor!(MySyncAccessor, SyncNotifier, SyncRequester)
//!
//! fn test(addr: rtactor::Addr)
//! {
//!     let accessor = MyNotifSyncAccessor::new(&addr);
//!     accessor.temperature_changed(13.2f32).unwrap();
//!     accessor.set_value(72).unwrap();
//!     assert!(accessor.get_value().unwrap() == 72);
//! }
//! ```
//!

extern crate proc_macro;

use convert_case::{Case, Casing};
use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote, ToTokens};
use syn::{parse_macro_input, Data, DeriveInput, Fields};

// Set to true to print to std::out the generated macro code.
const PRINT_GENERATED_MACRO_CODE: bool = false;

// see for attributes():
// https://stackoverflow.com/questions/42484062/how-do-i-process-enum-struct-field-attributes-in-a-procedural-macro
#[proc_macro_derive(ResponseEnum, attributes(response_val))]
pub fn derive_response_enum(input: TokenStream) -> TokenStream {
    // See https://doc.servo.org/syn/derive/struct.DeriveInput.html
    let input: DeriveInput = parse_macro_input!(input as DeriveInput);

    // get enum name
    let enum_name = &input.ident;
    let data = &input.data;

    let mut response_variants;

    // data is of type syn::Data
    // See https://doc.servo.org/syn/enum.Data.html
    match data {
        // Only if data is an enum, we do parsing
        Data::Enum(data_enum) => {
            // data_enum is of type syn::DataEnum
            // https://doc.servo.org/syn/struct.DataEnum.html

            response_variants = TokenStream2::new();

            // Iterate over enum variants
            // `variants` if of type `Punctuated` which implements IntoIterator
            //
            // https://doc.servo.org/syn/punctuated/struct.Punctuated.html
            // https://doc.servo.org/syn/struct.Variant.html
            for variant in &data_enum.variants {
                // Variant's name
                let variant_name = &variant.ident;

                // construct an identifier named <variant_name> for function name
                // We convert it to snake case using `to_case(Case::Snake)`
                // For example, if variant is `HelloWorld`, it will generate `is_hello_world`
                let mut request_func_name =
                    format_ident!("{}", variant_name.to_string().to_case(Case::Snake));
                request_func_name.set_span(variant_name.span());

                if let Some(ref a) = variant.attrs.iter().find(|a| match a.path.get_ident() {
                    Some(ident) => ident == "response_val",
                    None => false,
                }) {
                    if let Ok(response_val_type) = a.parse_args::<syn::Type>() {
                        let response_val_type = response_val_type.to_token_stream();

                        response_variants.extend(quote!(
                            #variant_name(#response_val_type),
                        ));
                    } else if a.parse_args::<syn::parse::Nothing>().is_ok() {
                        response_variants.extend(quote!(
                            #variant_name(),
                        ));
                    } else {
                        panic!(
                            "attribute '{}' parsing failed for variant '{}'",
                            a.to_token_stream(),
                            variant_name
                        );
                    }
                } else {
                    response_variants.extend(quote!(
                        #variant_name(),
                    ));
                };
            }
        }
        _ => panic!(
            "ResponseEnum is only valid for enums and '{}' is not one.",
            enum_name
        ),
    };

    let response_enum_name = format_ident!("{}", "Response");

    let expanded = quote! {
        pub enum #response_enum_name {
            #response_variants
        }
    };

    if PRINT_GENERATED_MACRO_CODE {
        println!("expanded='{}'", expanded);
    }
    TokenStream::from(expanded)
}

#[proc_macro_derive(SyncNotifier)]
pub fn derive_sync_notifier(input: TokenStream) -> TokenStream {
    // See https://doc.servo.org/syn/derive/struct.DeriveInput.html
    let input: DeriveInput = parse_macro_input!(input as DeriveInput);

    // get enum name
    let enum_name = &input.ident;
    let data = &input.data;

    let mut variant_notifier_functions;

    // data is of type syn::Data
    // See https://doc.servo.org/syn/enum.Data.html
    match data {
        // Only if data is an enum, we do parsing
        Data::Enum(data_enum) => {
            // data_enum is of type syn::DataEnum
            // https://doc.servo.org/syn/struct.DataEnum.html

            variant_notifier_functions = TokenStream2::new();

            // Iterate over enum variants
            // `variants` if of type `Punctuated` which implements IntoIterator
            //
            // https://doc.servo.org/syn/punctuated/struct.Punctuated.html
            // https://doc.servo.org/syn/struct.Variant.html
            for variant in &data_enum.variants {
                // Variant's name
                let variant_name = &variant.ident;

                // construct an identifier named <variant_name> for function name
                // We convert it to snake case using `to_case(Case::Snake)`
                // For example, if variant is `HelloWorld`, it will generate `is_hello_world`
                let mut notify_func_name =
                    format_ident!("{}", variant_name.to_string().to_case(Case::Snake));
                notify_func_name.set_span(variant_name.span());

                // Variant can have unnamed fields like `Variant(i32, i64)`
                // Variant can have named fields like `Variant {x: i32, y: i32}`
                // Variant can be named Unit like `Variant`
                match &variant.fields {
                    Fields::Named(fields) => {
                        let field_name: Vec<_> =
                            fields.named.iter().map(|field| &field.ident).collect();
                        let field_type: Vec<_> =
                            fields.named.iter().map(|field| &field.ty).collect();

                        variant_notifier_functions.extend(quote!(
                            fn #notify_func_name( &mut self, #( #field_name : #field_type, )*) -> Result<(), ::rtactor::Error> {
                                self.send_notification(#enum_name::#variant_name { #( #field_name : #field_name, )*})
                            }
                        ));
                    }
                    Fields::Unnamed(_) =>
                    panic!("SyncNotifier is not valid for Unnamed variant like '{}', use Named (i.e. 'MyVariant{{arg1: i32}}') variant.", variant_name),
                    Fields::Unit => {
                        variant_notifier_functions.extend(quote!(
                            fn #notify_func_name(&mut self) -> Result<(), ::rtactor::Error> {
                                self.send_notification(#enum_name::#variant_name)
                            }
                        ));
                    }
                };
            }
        }
        _ => panic!(
            "SyncNotifier is only valid for enums and '{}' is not one.",
            enum_name
        ),
    };

    let trait_name = format_ident!("{}", "SyncNotifier");

    let expanded = quote! {
        pub trait #trait_name : ::rtactor::SyncAccessor {
            #variant_notifier_functions
        }
    };

    if PRINT_GENERATED_MACRO_CODE {
        println!("expanded='{}'", expanded);
    }
    TokenStream::from(expanded)
}

// see for attributes():
// https://stackoverflow.com/questions/42484062/how-do-i-process-enum-struct-field-attributes-in-a-procedural-macro
#[proc_macro_derive(SyncRequester, attributes(response_val))]
pub fn derive_sync_requester(input: TokenStream) -> TokenStream {
    // See https://doc.servo.org/syn/derive/struct.DeriveInput.html
    let input: DeriveInput = parse_macro_input!(input as DeriveInput);

    // get enum name
    let enum_name = &input.ident;
    let data = &input.data;

    let mut variant_requester_functions;

    // data is of type syn::Data
    // See https://doc.servo.org/syn/enum.Data.html
    match data {
        // Only if data is an enum, we do parsing
        Data::Enum(data_enum) => {
            // data_enum is of type syn::DataEnum
            // https://doc.servo.org/syn/struct.DataEnum.html

            variant_requester_functions = TokenStream2::new();

            // Iterate over enum variants
            // `variants` if of type `Punctuated` which implements IntoIterator
            //
            // https://doc.servo.org/syn/punctuated/struct.Punctuated.html
            // https://doc.servo.org/syn/struct.Variant.html
            for variant in &data_enum.variants {
                // Variant's name
                let variant_name = &variant.ident;

                // construct an identifier named <variant_name> for function name
                // We convert it to snake case using `to_case(Case::Snake)`
                // For example, if variant is `HelloWorld`, it will generate `is_hello_world`
                let mut request_func_name =
                    format_ident!("{}", variant_name.to_string().to_case(Case::Snake));
                request_func_name.set_span(variant_name.span());

                let return_type = if let Some(ref a) =
                    variant.attrs.iter().find(|a| match a.path.get_ident() {
                        Some(ident) => ident == "response_val",
                        None => false,
                    }) {
                    if let Ok(types) = a.parse_args::<syn::Type>() {
                        Some(types)
                    } else if a.parse_args::<syn::parse::Nothing>().is_ok() {
                        None
                    } else {
                        panic!(
                            "attribute '{}' parsing failed for variant '{}'",
                            a.to_token_stream(),
                            variant_name
                        );
                    }
                } else {
                    None
                };

                let method_return_type = match return_type.clone() {
                    Some(ret_type) => {
                        let token_stream = ret_type.into_token_stream();
                        quote!(#token_stream)
                    }
                    None => quote!(()),
                };

                let ok_var_name = match return_type.clone() {
                    Some(_) => quote!(variant_data),
                    None => quote!(),
                };

                let ok_ret_value = match return_type.clone() {
                    Some(_) => quote!(variant_data),
                    None => quote!(()),
                };

                // Variant can have unnamed fields like `Variant(i32, i64)`
                // Variant can have named fields like `Variant {x: i32, y: i32}`
                // Variant can be named Unit like `Variant`
                match &variant.fields {
                    Fields::Named(fields) => {
                        let field_name: Vec<_> =
                            fields.named.iter().map(|field| &field.ident).collect();
                        let field_type: Vec<_> =
                            fields.named.iter().map(|field| &field.ty).collect();

                        variant_requester_functions.extend(quote!(
                            fn #request_func_name( &mut self, #( #field_name : #field_type, )* duration: std::time::Duration) -> Result<#method_return_type, ::rtactor::Error> {
                                match self.request_for::<#enum_name, Response>(#enum_name::#variant_name { #( #field_name : #field_name, )*}, duration)
                                {
                                    Ok(Response::#variant_name(#ok_var_name)) => Ok(#ok_ret_value),
                                    Ok(_) => Err(::rtactor::Error::DowncastFailed),
                                    Err(err) => Err(err),
                                }
                            }
                        ));
                    }
                    Fields::Unnamed(_) => {
                        panic!(
                            "SyncRequester do not accept Unnamed variant and '{}' is one.",
                            variant_name
                        );
                    }
                    Fields::Unit => {
                        variant_requester_functions.extend(quote!(
                            fn #request_func_name( &mut self, duration: std::time::Duration) -> Result<#method_return_type, ::rtactor::Error> {
                                match self.request_for::<#enum_name, Response>(Request::#variant_name, duration)
                                {
                                    Ok(Response::#variant_name(#ok_var_name)) => Ok(#ok_ret_value),
                                    Ok(_) => Err(::rtactor::Error::DowncastFailed),
                                    Err(err) => Err(err),
                                }
                            }
                        ));
                    }
                };
            }
        }
        _ => panic!(
            "SyncRequester is only valid for enums and '{}' is not one.",
            enum_name
        ),
    };

    let trait_name = format_ident!("{}", "SyncRequester");

    let expanded = quote! {
        pub trait #trait_name : ::rtactor::SyncAccessor {
            #variant_requester_functions
        }
    };

    if PRINT_GENERATED_MACRO_CODE {
        println!("expanded='{}'", expanded);
    }
    TokenStream::from(expanded)
}
