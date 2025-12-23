//! Derive macros for the Incan programming language standard library.
//!
//! These macros generate boilerplate implementations for Incan language features:
//! - `IncanClass`: Generates `__class__()` and `__fields__()` reflection methods
//! - `FieldInfo`: Generates `FieldInfo` trait implementation for reflection
//! - `IncanReflect`: Alias for IncanClass (for clarity in generated code)
//! - `IncanJson`: Generates JSON serialization helpers

use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, Data, DeriveInput, Fields};

/// Generates reflection methods for Incan classes/models.
///
/// This derive macro adds:
/// - `__class_name__() -> &'static str`: Returns the struct name
/// - `__fields__() -> Vec<&'static str>`: Returns field names
///
/// # Example
/// ```ignore
/// #[derive(IncanClass)]
/// struct User {
///     id: i64,
///     name: String,
/// }
///
/// // Generates:
/// impl User {
///     pub fn __class_name__(&self) -> &'static str { "User" }
///     pub fn __fields__(&self) -> Vec<&'static str> { vec!["id", "name"] }
/// }
/// ```
#[proc_macro_derive(IncanClass)]
pub fn derive_incan_class(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;
    let name_str = name.to_string();

    // Get field names
    let field_names: Vec<String> = match &input.data {
        Data::Struct(data) => match &data.fields {
            Fields::Named(fields) => fields
                .named
                .iter()
                .filter_map(|f| f.ident.as_ref().map(|i| i.to_string()))
                .collect(),
            Fields::Unnamed(fields) => (0..fields.unnamed.len()).map(|i| i.to_string()).collect(),
            Fields::Unit => vec![],
        },
        _ => vec![],
    };

    let expanded = quote! {
        impl #name {
            // TODO: consider all python class' dunders:
            //      '__class__',          -> implemented below
            //      '__delattr__',        -> since we keep things immutable by default, we might not need this.
            //      '__dict__',           -> probably not needed in favor of __fields__()
            //      '__dir__',             directory of the object (all available attributes)
            //      '__doc__',             documentation string
            //      '__gt__',              greater than
            //      '__hash__',            hash of the object
            //      '__init__',           -> not needed, we use the constructor to initialize the object.
            //      '__init_subclass__',  -> not needed, we use the constructor to initialize the object.
            //      '__le__',              less than or equal to
            //      '__lt__',              less than
            //      '__module__',          what module is this class in?
            //      '__ne__',              not equal to
            //      '__new__',             new object
            //      '__reduce__',         -> investigate if needed
            //      '__reduce_ex__',      -> investigate if needed
            //      '__repr__',            essentially the same as the Debug trait
            //      '__setattr__',        -> since we keep things immutable by default, we might not need this.
            //      '__sizeof__',         -> investigate if needed
            //      '__str__',             essentially the same as the Display trait
            //      '__subclasshook__',   -> not needed as we don't have an MRO to worry about
            //      '__weakref__'         -> investigate if needed
            //
            //      We should implement all of the applicable ones in some way, shape, or form.

            /// Returns the name of this class/struct. Inspired by Python's __class__ dunder.
            pub fn __class__(&self) -> &'static str {
                #name_str
                // TODO: make the format like: <class '__main__.SomeClass'> or <class 'SomeClass'>
                //     This is used to get the class name in the runtime.
                //     We can use the module name and the class name to get the full class name.
                //     We should distinguishes between different class types, like model, class, enum, etc.
            }

            /// Alias for __class__() - more explicit naming
            pub fn __class_name__(&self) -> &'static str {
                self.__class__()
            }

            /// Returns the field names of this class/struct. Inspired by Pydantic's __fields__().
            pub fn __fields__(&self) -> Vec<&'static str> {
                vec![#(#field_names),*]
            }
        }
    };

    TokenStream::from(expanded)
}

/// Alias for IncanClass - generates reflection methods
#[proc_macro_derive(IncanReflect)]
pub fn derive_incan_reflect(input: TokenStream) -> TokenStream {
    derive_incan_class(input)
}

/// Generates the `FieldInfo` trait implementation for reflection.
///
/// This derive macro implements the `incan_stdlib::FieldInfo` trait, providing
/// static methods to query field names and types at runtime.
///
/// # Example
/// ```ignore
/// #[derive(FieldInfo)]
/// struct Person {
///     name: String,
///     age: i64,
/// }
///
/// // Generates:
/// impl FieldInfo for Person {
///     fn field_names() -> Vec<&'static str> { vec!["name", "age"] }
///     fn field_types() -> Vec<&'static str> { vec!["String", "i64"] }
/// }
/// ```
#[proc_macro_derive(FieldInfo)]
pub fn derive_field_info(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;

    // Get field names and types
    let (field_names, field_types): (Vec<String>, Vec<String>) = match &input.data {
        Data::Struct(data) => {
            match &data.fields {
                // Named fields (e.g. `struct User { name: String, age: i64 }`)
                Fields::Named(fields) => fields
                    .named
                    .iter()
                    .filter_map(|f| {
                        let field_name = f.ident.as_ref()?.to_string();
                        let ty = &f.ty;
                        let field_type = quote!(#ty).to_string();
                        Some((field_name, field_type))
                    })
                    .unzip(),
                // Unnamed fields (e.g. `struct User(String, i64)`)
                Fields::Unnamed(fields) => fields
                    .unnamed
                    .iter()
                    .enumerate()
                    .map(|(i, f)| {
                        let field_name = i.to_string();
                        let ty = &f.ty;
                        let field_type = quote!(#ty).to_string();
                        (field_name, field_type)
                    })
                    .unzip(),
                // Unit fields (e.g. `struct User()`)
                Fields::Unit => (vec![], vec![]),
            }
        }
        _ => (vec![], vec![]),
    };

    let expanded = quote! {
        impl incan_stdlib::FieldInfo for #name {
            fn field_names() -> Vec<&'static str> {
                vec![#(#field_names),*]
            }

            fn field_types() -> Vec<&'static str> {
                vec![#(#field_types),*]
            }
        }
    };

    TokenStream::from(expanded)
}

/// Generates JSON serialization helpers for Incan models.
///
/// This derive macro adds:
/// - `to_json(&self) -> String`: Serializes to JSON
/// - `to_json_pretty(&self) -> String`: Serializes to pretty-printed JSON
/// - `from_json(s: &str) -> Result<Self, serde_json::Error>`: Deserializes from JSON
///
/// Requires the struct to also derive `serde::Serialize` and `serde::Deserialize`.
#[proc_macro_derive(IncanJson)]
pub fn derive_incan_json(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;

    let expanded = quote! {
        impl #name {
            /// Serializes this instance to a JSON string
            pub fn to_json(&self) -> String {
                serde_json::to_string(self).unwrap_or_else(|e| {
                    panic!("Failed to serialize {}: {}", stringify!(#name), e)
                })
            }

            /// Deserializes an instance from a JSON string
            pub fn from_json(s: &str) -> Result<Self, serde_json::Error> {
                serde_json::from_str(s)
            }

            /// Serializes this instance to a pretty-printed JSON string
            pub fn to_json_pretty(&self) -> String {
                serde_json::to_string_pretty(self).unwrap_or_else(|e| {
                    panic!("Failed to serialize {}: {}", stringify!(#name), e)
                })
            }
        }
    };

    TokenStream::from(expanded)
}
