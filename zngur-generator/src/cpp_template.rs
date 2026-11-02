//! The Rust side of C++ class templates. Each C++ class template defines a Rust alias for its
//! instantiations, and a trait through which each instantiation names its Rust struct (generic
//! over its constness, like every C++ type's struct):
//!
//! ```ignore
//! // In the template's module (e.g. `generated::std`):
//! pub trait __ZngurCppTemplate_unique_ptr<C: Constness = NonConst> { type Type; }
//! pub type unique_ptr<T0, C = NonConst> = <T0 as __ZngurCppTemplate_unique_ptr<C>>::Type;
//!
//! // At the root, for `c++::std::unique_ptr<c++::a::B>`:
//! pub struct __zngur_std_unique_ptr_a_B_cpp_template_m22e30s10s22<C: Constness = NonConst> { ... }
//! impl<C: Constness> std::__ZngurCppTemplate_unique_ptr<C> for a::B {
//!     type Type = __zngur_std_unique_ptr_a_B_cpp_template_m22e30s10s22<C>;
//! }
//! ```
//!
//! So Rust code names instantiations the way C++ code does (`unique_ptr<a::B>`), while each
//! instantiation is a plain struct, with ordinary inherent and trait impls. Templates with more
//! arguments pass the others to the trait:
//! `pub type map<T0, T1, C = NonConst> = <T0 as __ZngurCppTemplate_map<T1, C>>::Type;`. The last
//! parameter is the constness: `unique_ptr<a::B, Const>` is a `const std::unique_ptr<a::B>`.

use std::collections::HashMap;

use zngur_def::{CppTypePath, RustType, ZngurType};

use crate::CppModuleTree;
use crate::rust::RustFile;

/// The template path and arguments of `ty`, if it's an instantiation of a C++ class template.
pub fn cpp_template(ty: &RustType) -> Option<&CppTypePath> {
    match ty {
        RustType::Cpp(path) if !path.generics.is_empty() => Some(path),
        _ => None,
    }
}

/// The C++ class templates that a module uses, and where their traits are.
pub struct CppTemplates {
    /// Template path -> path of its trait, relative to the generated code's root.
    trait_paths: HashMap<Vec<String>, String>,
}

fn trait_name(path: &[String]) -> String {
    format!(
        "__ZngurCppTemplate_{}",
        path.last().expect("C++ template paths aren't empty")
    )
}

impl CppTemplates {
    /// Emits the alias and trait of each C++ class template instantiated in `types`, and registers
    /// the Rust structs of the instantiations as the targets of their inherent `impl` blocks.
    pub fn new(
        types: &[ZngurType],
        rust_file: &mut RustFile,
        cpp_tree: &mut CppModuleTree,
    ) -> Self {
        let mut trait_paths = HashMap::new();
        for ty_def in types {
            let erased = ty_def.ty.with_anonymous_lifetimes();
            let Some(template) = cpp_template(&erased) else {
                continue;
            };
            rust_file
                .impl_targets
                .insert(erased.to_string(), struct_name(template));
            if trait_paths.contains_key(&template.path) {
                continue;
            }
            let path = &template.path;
            let arity = template.generics.len();
            let (name, module_path) = path.split_last().expect("C++ template paths aren't empty");
            let trait_name = trait_name(path);
            let params = (0..arity).map(|n| format!("T{n}")).collect::<Vec<_>>();
            let other_params: String = params[1..].iter().map(|p| format!("{p}, ")).collect();
            cpp_tree.insert(
                module_path,
                &format!(
                    r#"
    /// Names the Rust struct of each instantiation of the C++ class template `{name}`, with
    /// constness `C`.
    #[doc(hidden)]
    #[allow(non_camel_case_types)]
    pub trait {trait_name}<{other_params}C: ::zngur_lib::Constness = ::zngur_lib::NonConst> {{
        type Type;
    }}

    /// The instantiations of the C++ class template `{name}`. The last parameter is the constness
    /// (see `zngur_lib::Constness`).
    #[allow(non_camel_case_types)]
    pub type {name}<{}, C = ::zngur_lib::NonConst> = <T0 as {trait_name}<{other_params}C>>::Type;
"#,
                    params.join(", ")
                ),
            );
            trait_paths.insert(
                path.clone(),
                module_path
                    .iter()
                    .chain([&trait_name])
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("::"),
            );
        }
        CppTemplates { trait_paths }
    }

    pub fn add_instantiation(&self, rust_file: &mut RustFile, ty: &CppTypePath) {
        let trait_path = &self.trait_paths[&ty.path];
        let (first, rest) = ty
            .generics
            .split_first()
            .expect("instantiations have template arguments");
        let first = rust_file.impl_target(first);
        let other_args: String = rest
            .iter()
            .map(|t| format!("{}, ", rust_file.impl_target(t)))
            .collect();
        let struct_name = struct_name(ty);
        rust_file.text.push_str(&format!(
            "\nimpl<C: ::zngur_lib::Constness> {trait_path}<{other_args}C> for {first} {{\n    type \
             Type = {struct_name}<C>;\n}}\n"
        ));
    }
}

/// The name of the Rust struct of the instantiation `ty`, at the generated code's root. Users
/// name it through the template's alias instead.
pub fn struct_name(ty: &CppTypePath) -> String {
    let rust_type = RustType::Cpp(ty.clone()).to_string();
    format!(
        "_{}",
        crate::rust::mangle_name(&format!("{rust_type}=cpp_template"), "")
    )
}
