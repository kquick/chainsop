//! This crate allows auto-derivation of the `chainsop::FilesPrep` trait for a
//! structure which contains a `chainsop::FileTransformation` object (the latter
//! already has an impl of the `chainsop::FilesPrep` trait.
//!
//! Example:
//!
//! ```
//! #[derive(FilesTransformationPrep)]
//! pub struct SomeStruct {
//!     foo : Vec<OsString>,
//!     files : FileTransformation,
//!     bar : i32,
//! }
//! ```
//!
//! will create an impl of the `chainsop::FilesPrep` trait for `SomeStruct` that
//! passes the `chainsop::FilesPrep` methods to the `files` field.

use proc_macro::TokenStream;
use quote::quote;
use syn;


#[proc_macro_derive(FilesTransformationPrep)]
pub fn filesxprep_macro_derive(input: TokenStream) -> TokenStream {
    let ast = syn::parse(input)
        .expect("Parsing structure for FilesTransformationPrep derivation");
    impl_filesxprep_macro(&ast)
}

fn find_file_transformation_field(data: &syn::Data) -> syn::Ident {
    match &data {
        syn::Data::Struct(s) => {
            for f in &s.fields {
                match &f.ty {
                    syn::Type::Path(tp) =>
                        if tp.path.is_ident("FileTransformation") {
                            return f.ident.clone()
                                .expect("FileTransformation field name")
                        }
                    syn::Type::Reference(_tr) => todo!("type ref for {:?}", f.ident),
                    _ => todo!("type ? for {:?}", f.ident),
                }
            }
            match &s.fields {
                syn::Fields::Named(_nf) => todo!("handle data struct named field"),
                syn::Fields::Unnamed(_unf) => todo!("handle data struct unnamed field"),
                syn::Fields::Unit => todo!("handle data struct unit field"),
            };
        }
        syn::Data::Enum(_e) => todo!("handle data enum"),
        syn::Data::Union(_u) => todo!("handle data union"),
    };
}

fn impl_filesxprep_macro(ast: &syn::DeriveInput) -> TokenStream {
    let name = &ast.ident;
    let field = find_file_transformation_field(&ast.data);
    let gen = quote! {
        impl FilesPrep for #name {
            fn set_dir<T>(&mut self, tgtdir: T) -> &mut Self
            where T: AsRef<Path>
            {
                self.#field.set_dir(tgtdir);
                self
            }
            fn set_input_file(&mut self, fname: &FileArg) -> &mut Self
            {
                self.#field.set_input_file(fname);
                self
            }
            fn add_input_file(&mut self, fname: &FileArg) -> &mut Self
            {
                self.#field.add_input_file(fname);
                self
            }
            fn has_input_file(&self) -> bool
            {
                self.#field.has_input_file()
            }
            fn set_output_file(&mut self, fname: &FileArg) -> &mut Self
            {
                self.#field.set_output_file(fname);
                self
            }
            fn has_explicit_output_file(&self) -> bool
            {
                self.#field.has_explicit_output_file()
            }
        }
    };
    gen.into()
}
