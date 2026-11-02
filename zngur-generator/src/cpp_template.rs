//! The Rust side of C++ class templates. Each C++ class template has a home module, which defines
//! a Rust alias for its instantiations, and a trait through which each instantiation names its
//! Rust struct (generic over its constness, like every C++ type's struct):
//!
//! ```ignore
//! // In the home (e.g. `generated::std`):
//! pub trait __ZngurCppTemplate_unique_ptr<C: Constness = NonConst> { type Type; }
//! pub type unique_ptr<T0, C = NonConst> = <T0 as __ZngurCppTemplate_unique_ptr<C>>::Type;
//!
//! // In the module that declares `c++::std::unique_ptr<c++::a::B>`:
//! pub struct __zngur_std_unique_ptr_a_B_cpp_template_m22e30s10s22<C: Constness = NonConst> { ... }
//! impl<C: Constness> std::__ZngurCppTemplate_unique_ptr<C> for a::B {
//!     type Type = __zngur_std_unique_ptr_a_B_cpp_template_m22e30s10s22<C>;
//! }
//! ```
//!
//! So Rust code names instantiations the way C++ code does (`unique_ptr<a::B>`), while each
//! instantiation is a plain struct in the crate that declares it, with ordinary inherent and trait
//! impls. The orphan rule allows implementing the home's trait for the template arguments when
//! one of them is declared in that crate, which is where the parser allows instantiations outside
//! the home. Templates with more arguments pass the others to the trait:
//! `pub type map<T0, T1, C = NonConst> = <T0 as __ZngurCppTemplate_map<T1, C>>::Type;`. The last
//! parameter is the constness: `unique_ptr<a::B, Const>` is a `const std::unique_ptr<a::B>`.

use std::collections::{HashMap, HashSet};

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
    /// whose home or instantiations come from an imported module, and registers the Rust structs
    /// of the instantiations in `types` as the targets of their inherent `impl` blocks.
    pub fn new(
        homes: &[CppTemplateHome],
        imported_modules: &[ModuleImport],
        types: &[ZngurType],
        local_cpp_fn_paths: &HashSet<Vec<String>>,
        rust_file: &mut RustFile,
        cpp_tree: &mut CppModuleTree,
    ) -> Self {
        let mut trait_paths = HashMap::new();
        let mut emitted_aliases: HashSet<Vec<String>> = HashSet::new();
        // Templates instantiated in `types` without a known home (e.g. in a spec that wasn't
        // parsed) are homed here too, like the parser does.
        let mut homes = homes.to_vec();
        for ty_def in types {
            let erased = ty_def.ty.with_anonymous_lifetimes();
            if let Some(template) = cpp_template(&erased)
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
            if !emitted_aliases.insert(path.clone()) {
                continue;
            }
            let (name, module_path) = path.split_last().expect("C++ template paths aren't empty");
            let trait_name = trait_name(path);
            let params = (0..*arity).map(|n| format!("T{n}")).collect::<Vec<_>>();
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
        let reexport_alias_item = |crate_name: &str, path: &[String], arity: Option<usize>| {
            let (name, _) = path.split_last().expect("C++ template paths aren't empty");
            if local_cpp_fn_paths.contains(path)
                && let Some(arity) = arity
            {
                let params = (0..arity)
                    .map(|n| format!("T{n}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "\n#[allow(dead_code, non_camel_case_types)]\npub type {name}<{params}, C = \
                     ::zngur_lib::NonConst> = ::{crate_name}::generated::{}<{params}, C>;\n",
                    path.join("::")
                )
            } else {
                format!(
                    "\n#[allow(unused_imports)]\npub use ::{crate_name}::generated::{};\n",
                    path.join("::")
                )
            }
        };
        for module in imported_modules {
            let Some(crate_name) = &module.crate_name else {
                continue;
            };
            let crate_name = crate_name.replace('-', "_");
            for CppTemplateHome { path, arity, .. } in &module.cpp_template_homes {
                let (_, module_path) = path.split_last().expect("C++ template paths aren't empty");
                if emitted_aliases.insert(path.clone()) {
                    cpp_tree.insert(
                        module_path,
                        &reexport_alias_item(&crate_name, path, Some(*arity)),
                    );
                }
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
        // Also re-export template aliases from imported modules that instantiate (or re-export) a
        // C++ template whose home isn't directly imported here, so signatures using those
        // imported instantiations resolve in Rust.
        for module in imported_modules {
            let Some(crate_name) = &module.crate_name else {
                continue;
            };
            let crate_name = crate_name.replace('-', "_");
            for path in &module.cpp_templates {
                let Some((_, module_path)) = path.split_last() else {
                    continue;
                };
                if emitted_aliases.insert(path.clone()) {
                    let arity = module
                        .cpp_template_homes
                        .iter()
                        .find(|h| &h.path == path)
                        .map(|h| h.arity)
                        .or_else(|| {
                            module
                                .cpp_template_instantiations
                                .iter()
                                .find(|i| &i.path == path)
                                .map(|i| i.generics.len())
                        })
                        .or_else(|| {
                            imported_modules.iter().find_map(|m| {
                                m.cpp_template_homes
                                    .iter()
                                    .find(|h| &h.path == path)
                                    .map(|h| h.arity)
                                    .or_else(|| {
                                        m.cpp_template_instantiations
                                            .iter()
                                            .find(|i| &i.path == path)
                                            .map(|i| i.generics.len())
                                    })
                            })
                        });
                    cpp_tree.insert(module_path, &reexport_alias_item(&crate_name, path, arity));
                }
            }
        }
        for ty_def in types {
            let erased = ty_def.ty.with_anonymous_lifetimes();
            if let Some(template) = cpp_template(&erased) {
                rust_file
                    .impl_targets
                    .insert(erased.to_string(), struct_name(template));
            }
        }
        CppTemplates { trait_paths }
    }

    /// Implements the template's trait for `ty`, mapping its parameters to [`struct_name`].
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
