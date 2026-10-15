//! The Rust side of C++ class templates. Each C++ class template has a home module, which defines
//! a Rust alias for its instantiations, and a trait through which each instantiation names its
//! Rust struct:
//!
//! ```ignore
//! // In the home (e.g. `generated::std`):
//! pub trait __ZngurCppTemplate_unique_ptr { type Type; }
//! pub type unique_ptr<T0> = <T0 as __ZngurCppTemplate_unique_ptr>::Type;
//!
//! // In the module that declares `c++::std::unique_ptr<c++::a::B>`:
//! pub struct __zngur_cpp_std__unique_ptr__a__B { ... }
//! impl std::__ZngurCppTemplate_unique_ptr for a::B { type Type = __zngur_cpp_std__unique_ptr__a__B; }
//! ```
//!
//! So Rust code names instantiations the way C++ code does (`unique_ptr<a::B>`), while each
//! instantiation is a plain struct in the crate that declares it, with ordinary inherent and trait
//! impls. The orphan rule allows implementing the home's trait for the template arguments when
//! one of them is declared in that crate, which is where the parser allows instantiations outside
//! the home. Templates with more arguments pass the others to the trait:
//! `pub type map<T0, T1> = <T0 as __ZngurCppTemplate_map<T1>>::Type;`.

use std::collections::HashMap;

use zngur_def::{CppTemplateHome, CppTypePath, ModuleImport, RustType, ZngurType};

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
    /// Emits the alias and trait of each template in `homes`, re-exports the aliases of templates
    /// whose home is an imported module, and registers the Rust structs of the instantiations in
    /// `types` as the targets of their inherent `impl` blocks.
    pub fn new(
        homes: &[CppTemplateHome],
        imported_modules: &[ModuleImport],
        types: &[ZngurType],
        rust_file: &mut RustFile,
        cpp_tree: &mut CppModuleTree,
    ) -> Self {
        let mut trait_paths = HashMap::new();
        // Templates instantiated in `types` without a known home (e.g. in a spec that wasn't
        // parsed) are homed here too, like the parser does.
        let mut homes = homes.to_vec();
        for ty_def in types {
            if let Some(template) = cpp_template(&ty_def.ty)
                && !homes.iter().any(|h| h.path == template.path)
                && !imported_modules
                    .iter()
                    .flat_map(|m| &m.cpp_template_homes)
                    .any(|h| h.path == template.path)
            {
                homes.push(CppTemplateHome {
                    path: template.path.clone(),
                    arity: template.generics.len(),
                    explicit: false,
                });
            }
        }
        for CppTemplateHome { path, arity, .. } in &homes {
            let (name, module_path) = path.split_last().expect("C++ template paths aren't empty");
            let trait_name = trait_name(path);
            let params = (0..*arity).map(|n| format!("T{n}")).collect::<Vec<_>>();
            let trait_params = match &params[1..] {
                [] => String::new(),
                rest => format!("<{}>", rest.join(", ")),
            };
            cpp_tree.insert(
                module_path,
                &format!(
                    r#"
    /// Names the Rust struct of each instantiation of the C++ class template `{name}`.
    #[doc(hidden)]
    #[allow(non_camel_case_types)]
    pub trait {trait_name}{trait_params} {{
        type Type;
    }}

    /// The instantiations of the C++ class template `{name}`.
    #[allow(non_camel_case_types)]
    pub type {name}<{}> = <T0 as {trait_name}{trait_params}>::Type;
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
        for module in imported_modules {
            let Some(crate_name) = &module.crate_name else {
                continue;
            };
            let crate_name = crate_name.replace('-', "_");
            for CppTemplateHome { path, .. } in &module.cpp_template_homes {
                let (_, module_path) = path.split_last().expect("C++ template paths aren't empty");
                cpp_tree.insert(
                    module_path,
                    &format!(
                        "\n#[allow(unused_imports)]\npub use ::{crate_name}::generated::{};\n",
                        path.join("::")
                    ),
                );
                trait_paths.entry(path.clone()).or_insert_with(|| {
                    format!(
                        "::{crate_name}::generated::{}",
                        module_path
                            .iter()
                            .chain([&trait_name(path)])
                            .cloned()
                            .collect::<Vec<_>>()
                            .join("::")
                    )
                });
            }
        }
        for ty_def in types {
            if let Some(template) = cpp_template(&ty_def.ty) {
                rust_file
                    .impl_targets
                    .insert(ty_def.ty.to_string(), struct_name(template));
            }
        }
        CppTemplates { trait_paths }
    }

    /// `struct_name`.
    pub fn add_instantiation(&self, rust_file: &mut RustFile, ty: &CppTypePath) {
        let trait_path = &self.trait_paths[&ty.path];
        let (first, rest) = ty
            .generics
            .split_first()
            .expect("instantiations have template arguments");
        let trait_args = match rest {
            [] => String::new(),
            rest => format!(
                "<{}>",
                rest.iter()
                    .map(|t| t.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        };
        let struct_name = struct_name(ty);
        rust_file.text.push_str(&format!(
            "\nimpl {trait_path}{trait_args} for {first} {{\n    type Type = {struct_name};\n}}\n"
        ));
    }
}

/// The name of the Rust struct of the instantiation `ty`, at the generated code's root. Users
/// name it through the template's alias instead.
pub fn struct_name(ty: &CppTypePath) -> String {
    let rust_type = RustType::Cpp(ty.clone()).to_string();
    let mut name = String::from("__zngur_cpp_");
    for c in rust_type
        .replace("::<", "__")
        .replace("::", "__")
        .replace(", ", "__")
        .chars()
    {
        match c {
            c if c.is_ascii_alphanumeric() || c == '_' => name.push(c),
            '<' | '>' | ' ' => {}
            _ => name.push('_'),
        }
    }
    name
}
