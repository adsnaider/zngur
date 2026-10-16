use cpp::CppExportedFnDefinition;
use cpp::CppExportedImplDefinition;
use cpp::CppFile;
use cpp::CppFnDefinition;
use cpp::CppFnSig;
use cpp::CppMethod;
use cpp::CppPath;
use cpp::CppTraitDefinition;
use cpp::CppTypeDefinition;
use cpp::CppVariant;
use cpp::cpp_handle_keyword;
use cpp::{CppRef, CppStackOwned};
use cpp_template::{CppTemplates, cpp_template, struct_name};
use indexmap::map::Entry;
use itertools::Itertools;
use rust::CoroSupport;
use rust::IntoCpp;

pub mod cpp;
mod cpp_method;
mod cpp_template;
mod rust;
mod template;

use askama::Template;
pub use rust::RustFile;
pub use zngur_parser::{
    ParseReport, ParseResult, ParsedZngFile, ReportEntry, ReportSink, ReportSpan, SourceCache,
    SourceId, StdErrReportSink, cfg,
};

pub use zngur_def::*;

use crate::template::ZngHeaderTemplate;

#[derive(Default)]
struct CppModuleTree {
    children: indexmap::IndexMap<String, CppModuleTree>,
    content: String,
}

impl CppModuleTree {
    fn insert(&mut self, path: &[String], content: &str) {
        match path.split_first() {
            None => self.content.push_str(content),
            Some((head, rest)) => self
                .children
                .entry(head.clone())
                .or_default()
                .insert(rest, content),
        }
    }

    fn render(&self) -> String {
        let mut out = self.content.clone();
        for (name, child) in &self.children {
            out.push_str(&format!("\npub mod {name} {{\n{}\n}}\n", child.render()));
        }
        out
    }
}

pub struct ZngurGenerator(pub ZngurSpec, pub String);

impl ZngurGenerator {
    pub fn build_from_zng(zng: ZngurSpec, crate_name: String) -> Self {
        ZngurGenerator(zng, crate_name)
    }

    pub fn render(self, zng_header_in_place: bool) -> (String, String, Option<String>) {
        let zng = self.0;
        let mut cpp_file = CppFile::default();
        cpp_file.header_file_name = zng.cpp_include_header_name.clone();
        cpp_file.additional_includes = zng.additional_includes.0;
        cpp_file.zng_header_in_place = zng_header_in_place;
        for module in &zng.imported_modules {
            cpp_file
                .additional_includes
                .push_str(&format!("\n#include \"{}.h\"", module.path.display()));
        }
        let default_ns = zng.cpp_namespace.as_deref().unwrap_or("rust");
        let sanitized_crate_name = self.1.replace('-', "_");
        let mut rust_file = RustFile::new(&zng.mangling_base);
        rust_file.panic_to_exception = zng.convert_panic_to_exception.0;
        let coro_support = CoroSupport::from_types(
            zng.types.iter().map(|td| &td.ty),
            &zng.mangling_base,
            default_ns,
            &sanitized_crate_name,
        );
        if let Some(coro) = &coro_support {
            rust_file.add_coro_support(coro);
        }
        cpp_file.trait_defs = zng
            .traits
            .iter()
            .map(|(key, value)| {
                (
                    key.clone(),
                    rust_file.add_builder_for_dyn_trait(value, default_ns, &sanitized_crate_name),
                )
            })
            .collect();
        cpp_file.panic_to_exception = zng.convert_panic_to_exception.0;
        cpp_file
            .rust_cfg_defines
            .extend(zng.rust_cfg.iter().map(|(key, value)| {
                format!(
                    "ZNGUR_CFG_{}{}",
                    key.to_uppercase(),
                    value
                        .as_ref()
                        .and_then(|value| if value.trim().is_empty() {
                            None
                        } else {
                            Some(format!(
                                "_{}",
                                value
                                    .chars()
                                    .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
                                    .collect::<String>()
                                    .to_uppercase()
                            ))
                        })
                        .unwrap_or_default()
                )
            }));
        let mut cpp_tree = CppModuleTree::default();
        cpp_file.coro_support = coro_support;
        // C++ types that Rust holds by value, from this module and the ones it imports.
        let stack_owned_types: cpp_method::StackOwnedTypes = zng
            .imported_modules
            .iter()
            .flat_map(|module| module.cpp_by_value_types.iter().cloned())
            .chain(zng.types.iter().filter_map(|ty_def| {
                ty_def.cpp_by_value_layout()?;
                Some((ty_def.ty.clone(), ty_def.cpp_name()?))
            }))
            .collect();
        let bridges = cpp_method::BridgeRenderer {
            stack_owned: &stack_owned_types,
            namespace: default_ns,
            crate_name: &sanitized_crate_name,
        };
        let cpp_templates = CppTemplates::new(
            &zng.cpp_template_homes,
            &zng.imported_modules,
            &zng.types,
            &mut rust_file,
            &mut cpp_tree,
        );
        for ty_def in zng.types {
            let ty = &ty_def.ty;
            // What Rust can do with a C++ (`c++::`) type: hold it by value if it has a layout and
            // is trivially relocatable, and otherwise only refer to it.
            let cpp_name = ty_def.cpp_name();
            let cpp_stack_owned = ty_def
                .cpp_by_value_layout()
                .and(cpp_name.clone())
                .map(CppStackOwned);
            let cpp_ref = match (&cpp_name, &cpp_stack_owned) {
                (Some(name), None) => Some(CppRef {
                    name: name.clone(),
                    layout: ty_def.layout,
                }),
                _ => None,
            };
            // C++ member functions that Rust calls: a Rust function, and a C++ function that
            // calls the member function.
            if let Some(owner_cpp_type) = &cpp_name {
                for method in &ty_def.cpp_methods {
                    let this = match method.receiver {
                        ZngurMethodReceiver::Ref(m) => {
                            Some(RustType::cpp_ptr(m, CppIndirection::Reference, ty.clone()))
                        }
                        ZngurMethodReceiver::Static | ZngurMethodReceiver::Move => None,
                    };
                    let mangled_name = rust_file.add_cpp_method(
                        ty,
                        method.rust_name(),
                        this,
                        &method.inputs,
                        &method.output,
                        method.is_safe,
                    );
                    cpp_file.cpp_method_bridges.push(bridges.method(
                        &mangled_name,
                        ty,
                        owner_cpp_type,
                        method,
                    ));
                }
                // Field accessors: like `const` / non-`const` member functions that return a
                // reference to the field.
                for field in &ty_def.cpp_fields {
                    let this =
                        RustType::cpp_ptr(field.mutability, CppIndirection::Reference, ty.clone());
                    let mangled_name = rust_file.add_cpp_method(
                        ty,
                        field.rust_name(),
                        Some(this),
                        &[],
                        &field.as_cpp_method().output,
                        // `this` refers to a valid object, so its field is valid too.
                        true,
                    );
                    cpp_file.cpp_method_bridges.push(bridges.field(
                        &mangled_name,
                        ty,
                        owner_cpp_type,
                        field,
                    ));
                }
                // C++ constructors (by-value types only, checked by the parser): a
                // `ZngCppConstruct` impl, and a C++ function that runs the constructor in place.
                for (index, constructor) in ty_def.cpp_constructors.iter().enumerate() {
                    let mangled_name = rust_file.add_cpp_constructor(ty, index, constructor);
                    cpp_file.cpp_method_bridges.push(bridges.constructor(
                        &mangled_name,
                        owner_cpp_type,
                        constructor,
                    ));
                }
            }
            let is_copy = ty_def.wellknown_traits.contains(&ZngurWellknownTrait::Copy);
            // In Rust, C++ types that are only referred to are zero-sized. (Their layout, if
            // any, is only checked in C++.)
            let layout = if cpp_ref.is_some() {
                LayoutPolicy::ZERO_SIZED_TYPE
            } else {
                ty_def
                    .layout
                    .expect("Every type should have a defined layout policy before rendering")
            };
            match layout {
                LayoutPolicy::StackAllocated { size, align } => {
                    rust_file.add_static_size_assert(&ty, size);
                    rust_file.add_static_align_assert(&ty, align);
                }
                LayoutPolicy::Conservative { size, align } => {
                    rust_file.add_static_size_upper_bound_assert(&ty, size);
                    rust_file.add_static_align_upper_bound_assert(&ty, align);
                }
                LayoutPolicy::HeapAllocated => (),
                LayoutPolicy::OnlyByRef => (),
            }
            if is_copy {
                rust_file.add_static_is_copy_assert(&ty);
            }
            // The Rust wrapper of a C++ type is in the module of its `c++::` path, except for
            // instantiations of C++ class templates, which are structs at the root that their
            // template's alias names.
            let template = cpp_template(ty);
            let (type_name, module_path) = match &ty {
                RustType::Cpp(path) if template.is_some() => (struct_name(path), vec![]),
                RustType::Cpp(path) => {
                    let (name, path) = path
                        .path
                        .split_last()
                        .expect("Cpp path must have at least one segment");
                    (name.clone(), path.to_vec())
                }
                _ => (String::new(), vec![]),
            };
            let mut insert_struct = |rust_file: &mut RustFile, def: &str| {
                if template.is_some() {
                    rust_file.text.push_str(def);
                } else {
                    cpp_tree.insert(&module_path, def);
                }
            };
            let mut stack_owned_destructor = None;
            if cpp_stack_owned.is_some() {
                // The C++ side defines this function under the returned (mangled) name, so it
                // includes the mangling base like every other bridge function. It's derived from
                // the full path, so that same-named types in different modules don't collide.
                let sanitized_path = ty
                    .to_string()
                    .chars()
                    .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
                    .collect::<String>();
                let destructor_name = format!("_zngur_{sanitized_path}_destructor");
                let mangled_name = rust_file.add_extern_cpp_function(
                    &destructor_name,
                    &[RustType::Ref(Mutability::Mut, Box::new(ty.clone()))],
                    &RustType::Tuple(vec![]),
                    true,
                );
                stack_owned_destructor = Some(mangled_name.clone());
                let (size, align) = match layout {
                    LayoutPolicy::StackAllocated { size, align }
                    | LayoutPolicy::Conservative { size, align } => (size, align),
                    _ => unreachable!("C++ types held by value have a size and alignment"),
                };
                let struct_def = format!(
                    r#"
    #[repr(C)]
    #[repr(align({align}))]
    #[allow(non_camel_case_types)]
    pub struct {type_name} {{
      pub(crate) buffer: ::core::cell::UnsafeCell<::core::mem::MaybeUninit<[u8; {size}]>>,
      _no_auto_traits: ::core::marker::PhantomData<*mut ()>,
      _pinned: ::core::marker::PhantomPinned,
    }}

    unsafe impl ::zngur_lib::ZngCppObject for {type_name} {{}}
    unsafe impl ::zngur_lib::ZngCppStackObject for {type_name} {{}}

    unsafe impl ::zngur_lib::ZngCppDestruct for {type_name} {{
        unsafe fn destruct(&mut self) {{
            unsafe extern "C" {{
                fn {mangled_name}(i0: *mut u8, o: *mut u8);
            }}
            let mut dummy = ();
            unsafe {{
                {mangled_name}(self.buffer.get() as *mut _, &mut dummy as *mut () as *mut u8);
            }}
        }}
    }}

    impl Drop for {type_name} {{
        fn drop(&mut self) {{
            use ::zngur_lib::ZngCppDestruct;
            unsafe {{
                self.destruct();
            }}
        }}
    }}

    impl {type_name} {{
        /// A C++ reference to `const` (`const T&`) to this object.
        #[allow(dead_code, clippy::should_implement_trait)]
        pub fn as_ref(&self) -> ::zngur_lib::CppRef<::zngur_lib::Const<Self>> {{
            ::zngur_lib::CppRef::from(self)
        }}

        /// A C++ reference (`T&`) to this object.
        #[allow(dead_code, clippy::should_implement_trait)]
        pub fn as_mut(&mut self) -> ::zngur_lib::CppRef<Self> {{
            ::zngur_lib::CppRef::from(self)
        }}
    }}
"#
                );
                insert_struct(&mut rust_file, &struct_def);
            }
            if cpp_ref.is_some() {
                insert_struct(
                    &mut rust_file,
                    &format!(
                        r#"
    #[allow(non_camel_case_types)]
    pub struct {type_name}(());

    unsafe impl ::zngur_lib::ZngCppObject for {type_name} {{}}
"#
                    ),
                );
            }
            if let Some(template) = template {
                cpp_templates.add_instantiation(&mut rust_file, template);
            }
            let mut cpp_methods = vec![];
            let mut constructor = None;
            let mut variants = vec![];
            let mut fields = vec![];
            let mut wellknown_traits = vec![];
            if let Some(constructor_def) = ty_def.constructor {
                let rust_link_name = rust_file.add_constructor(
                    &ty.to_string(),
                    constructor_def.inputs.iter().map(|(name, ty)| (name, ty)),
                );
                constructor = Some(CppFnSig {
                    rust_link_name,
                    inputs: constructor_def
                        .inputs
                        .iter()
                        .map(|x| x.1.into_cpp(default_ns, &sanitized_crate_name))
                        .collect(),
                    output: ty.into_cpp(default_ns, &sanitized_crate_name),
                });
            }
            let discriminant = if ty_def.exhaustive {
                rust_file.add_discriminant(&ty.to_string(), &ty_def.variants)
            } else {
                None
            };
            for variant in ty_def.variants {
                let rust_name = format!("{}::{}", ty, variant.name);
                let constructor = if variant.exhaustive {
                    let rust_link_name = rust_file.add_constructor(
                        &rust_name,
                        variant.fields.iter().map(|field| (&field.name, &field.ty)),
                    );
                    Some(CppFnSig {
                        rust_link_name,
                        inputs: variant
                            .fields
                            .iter()
                            .map(|x| x.ty.into_cpp(default_ns, &sanitized_crate_name))
                            .collect(),
                        output: ty
                            .into_cpp(default_ns, &sanitized_crate_name)
                            .with_tail(variant.name.clone()),
                    })
                } else {
                    None
                };
                let match_check = rust_file.add_match_check(&rust_name);
                let mut fields = vec![];
                for field in variant.fields {
                    let mn =
                        rust_file.add_variant_field_calculations(&field, &ty_def.ty, &variant.name);
                    fields.push(ZngurFieldData {
                        name: field.name,
                        ty: field.ty,
                        offset: ZngurFieldDataOffset::AutoDynamic(mn),
                    });
                }
                variants.push(CppVariant {
                    name: variant.name,
                    constructor,
                    match_check,
                    fields,
                });
            }
            for field in ty_def.fields {
                let extern_mn = rust_file.add_field_assertions(&field, &ty_def.ty);
                let field = ZngurFieldData {
                    name: field.name,
                    ty: field.ty,
                    offset: match field.offset {
                        Some(offset) => ZngurFieldDataOffset::Offset(offset),
                        None => ZngurFieldDataOffset::Auto(
                            extern_mn.expect("auto offset did not provide extern name"),
                        ),
                    },
                };
                fields.push(field);
            }
            if let RustType::Tuple(fields) = &ty_def.ty {
                if !fields.is_empty() {
                    let rust_link_name = rust_file.add_tuple_constructor(&fields);
                    constructor = Some(CppFnSig {
                        rust_link_name,
                        inputs: fields
                            .iter()
                            .map(|x| x.into_cpp(default_ns, &sanitized_crate_name))
                            .collect(),
                        output: ty.into_cpp(default_ns, &sanitized_crate_name),
                    });
                }
            }
            let is_unsized = ty_def
                .wellknown_traits
                .contains(&ZngurWellknownTrait::Unsized);
            for wellknown_trait in ty_def.wellknown_traits {
                let data = rust_file.add_wellknown_trait(&ty, wellknown_trait, is_unsized);
                wellknown_traits.push(data);
            }
            for method_details in ty_def.methods {
                let ZngurMethodDetails {
                    data: method,
                    use_path,
                    deref,
                    cpp_name,
                } = method_details;
                let rusty_inputs = real_inputs_of_method(&method, &ty);
                let cpp_name = cpp_name.as_ref().unwrap_or(&method.name);

                let sig = rust_file.add_function(
                    cpp_name,
                    &format!(
                        "<{}>::{}::<{}>",
                        deref.as_ref().map(|x| &x.0).unwrap_or(&ty),
                        method.name,
                        method.generics.iter().join(", "),
                    ),
                    &rusty_inputs,
                    &method.output,
                    use_path,
                    deref.map(|x| x.1),
                    default_ns,
                    &sanitized_crate_name,
                );
                cpp_methods.push(CppMethod {
                    name: cpp_handle_keyword(cpp_name).to_owned(),
                    kind: method.receiver,
                    sig,
                });
            }
            let (layout_size, layout_align, is_layout_conservative) = match layout {
                LayoutPolicy::StackAllocated { size, align } => (size, align, false),
                LayoutPolicy::Conservative { size, align } => (size, align, true),
                _ => (0, 1, false),
            };
            cpp_file.type_defs.push(CppTypeDefinition {
                ty: ty.into_cpp(default_ns, &sanitized_crate_name),
                layout: rust_file.add_layout_policy_shim(&ty, layout),
                layout_size,
                layout_align,
                is_layout_conservative,
                constructor,
                variants,
                discriminant,
                fields,
                methods: cpp_methods,
                wellknown_traits,
                cpp_ref,
                cpp_stack_owned,
                check_trivially_relocatable: ty_def.trivially_relocatable
                    == TriviallyRelocatable::Checked,
                stack_owned_destructor,
                from_trait: if let RustType::Boxed(b) = &ty {
                    if let RustType::Dyn(tr, _) = b.as_ref() {
                        if let RustTrait::Fn {
                            name,
                            inputs,
                            output,
                        } = tr
                        {
                            if let Entry::Vacant(e) = cpp_file.trait_defs.entry(tr.clone()) {
                                let rust_link_name =
                                    rust_file.add_builder_for_dyn_fn(name, inputs, output);
                                e.insert(CppTraitDefinition::Fn {
                                    sig: CppFnSig {
                                        rust_link_name,
                                        inputs: inputs
                                            .iter()
                                            .map(|x| x.into_cpp(default_ns, &sanitized_crate_name))
                                            .collect(),
                                        output: output.into_cpp(default_ns, &sanitized_crate_name),
                                    },
                                });
                            }
                        }
                        Some(tr.clone())
                    } else {
                        None
                    }
                } else {
                    None
                },
                from_trait_ref: if let RustType::Dyn(tr, _) = &ty {
                    Some(tr.clone())
                } else {
                    None
                },
            });
        }
        // `c++::` types of imported modules are defined in their own crate (at
        // `::<crate>::generated::<path>`). Re-exporting them at the same relative path as local
        // `c++::` types lets this module's generated code refer to both the same way.
        for module in &zng.imported_modules {
            let Some(crate_name) = &module.crate_name else {
                continue;
            };
            for segs in &module.cpp_types {
                let Some((_, module_path)) = segs.split_last() else {
                    continue;
                };
                cpp_tree.insert(
                    module_path,
                    &format!(
                        "\n#[allow(unused_imports)]\npub use ::{}::generated::{};\n",
                        crate_name.replace('-', "_"),
                        segs.join("::")
                    ),
                );
            }
        }
        // Free C++ functions: a Rust function that calls a C++ function that calls the C++
        // function. The Rust function is defined at the top level, where the generated code's
        // paths resolve, and re-exported in the `c++::` tree, at the C++ namespace's path.
        for func in &zng.cpp_funcs {
            let rust_name = func.rust_name();
            // `__` separators, so that `a::b_c` and `a_b::c` don't collide.
            let hidden_name = format!("__zngur_cpp_fn__{}__{rust_name}", func.namespace.join("__"));
            let mangled_name =
                rust_file.add_cpp_function(&hidden_name, &func.inputs, &func.output, func.is_safe);
            let supers = "super::".repeat(func.namespace.len());
            cpp_tree.insert(
                &func.namespace,
                &format!("\npub use {supers}{hidden_name} as {rust_name};\n"),
            );
            cpp_file
                .cpp_method_bridges
                .push(bridges.function(&mangled_name, func));
        }
        rust_file.text.push_str(&cpp_tree.render());
        for func in zng.funcs {
            let sig = rust_file.add_function(
                &func.path.to_string(),
                &func.path.to_string(),
                &func.inputs,
                &func.output,
                None,
                None,
                default_ns,
                &sanitized_crate_name,
            );
            cpp_file.fn_defs.push(CppFnDefinition {
                name: CppPath::from_rust_path(&func.path.path, default_ns, &sanitized_crate_name),
                sig,
            });
        }
        for func in zng.extern_cpp_funcs {
            let rust_link_name = rust_file.add_extern_cpp_function(
                &func.name,
                &func.inputs,
                &func.output,
                func.is_safe,
            );
            cpp_file.exported_fn_defs.push(CppExportedFnDefinition {
                name: func.name.clone(),
                sig: CppFnSig {
                    rust_link_name,
                    inputs: func
                        .inputs
                        .into_iter()
                        .map(|x| x.into_cpp(default_ns, &sanitized_crate_name))
                        .collect(),
                    output: func.output.into_cpp(default_ns, &sanitized_crate_name),
                },
            });
        }
        for impl_block in zng.extern_cpp_impls {
            let rust_link_names = rust_file.add_extern_cpp_impl(
                &impl_block.ty,
                impl_block.tr.as_ref(),
                &impl_block.methods,
            );
            cpp_file.exported_impls.push(CppExportedImplDefinition {
                tr: impl_block
                    .tr
                    .map(|x| x.into_cpp(default_ns, &sanitized_crate_name)),
                ty: impl_block.ty.into_cpp(default_ns, &sanitized_crate_name),
                methods: impl_block
                    .methods
                    .iter()
                    .zip(&rust_link_names)
                    .map(|(method, link_name)| {
                        let inputs = real_inputs_of_method(method, &impl_block.ty);
                        let inputs = inputs
                            .iter()
                            .map(|ty| ty.into_cpp(default_ns, &sanitized_crate_name))
                            .collect();
                        (
                            cpp_handle_keyword(&method.name).to_owned(),
                            CppFnSig {
                                rust_link_name: link_name.clone(),
                                inputs,
                                output: method.output.into_cpp(default_ns, &sanitized_crate_name),
                            },
                        )
                    })
                    .collect(),
            });
        }
        let (h, cpp) = cpp_file.render(default_ns, &sanitized_crate_name);
        (rust_file.text, h, cpp)
    }
}

pub struct ZngHeaderGenerator {
    pub panic_to_exception: bool,
    pub cpp_namespace: String,
}

impl ZngHeaderGenerator {
    /// Renders the zngur.h header
    pub fn render(&self) -> Result<String, askama::Error> {
        let zng_h = ZngHeaderTemplate {
            panic_to_exception: self.panic_to_exception,
            cpp_namespace: self.cpp_namespace.clone(),
        };
        zng_h.render()
    }
}

fn real_inputs_of_method(method: &ZngurMethod, ty: &RustType) -> Vec<RustType> {
    let receiver_type = match method.receiver {
        ZngurMethodReceiver::Static => None,
        ZngurMethodReceiver::Ref(m) => Some(RustType::Ref(m, Box::new(ty.clone()))),
        ZngurMethodReceiver::Move => Some(ty.clone()),
    };
    let rusty_inputs = receiver_type
        .into_iter()
        .chain(method.inputs.clone())
        .collect::<Vec<_>>();
    rusty_inputs
}

#[cfg(test)]
mod tests {
    use zngur_def::*;

    use crate::ZngurGenerator;

    /// A `c++::` type that Rust holds by value.
    fn minimal_stack_owned_type(ty: RustType, size: usize, align: usize) -> ZngurType {
        ZngurType {
            ty,
            layout: Some(LayoutPolicy::StackAllocated { size, align }),
            trivially_relocatable: TriviallyRelocatable::Checked,
            wellknown_traits: vec![],
            exhaustive: true,
            methods: vec![],
            constructor: None,
            variants: vec![],
            fields: vec![],
            cpp_methods: vec![],
            cpp_constructors: vec![],
            cpp_fields: vec![],
        }
    }

    /// A `c++::` type that Rust only refers to.
    fn minimal_ref_type(ty: RustType) -> ZngurType {
        ZngurType {
            layout: None,
            trivially_relocatable: TriviallyRelocatable::No,
            ..minimal_stack_owned_type(ty, 0, 1)
        }
    }

    fn cpp(segments: &[&str]) -> RustType {
        RustType::Cpp(
            segments
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
                .into(),
        )
    }

    #[test]
    fn rust_type_named_unit_is_not_the_unit_type() {
        let ty = RustType::Adt(RustPathAndGenerics {
            path: vec!["a".to_owned(), "Unit".to_owned()],
            generics: vec![],
            named_generics: vec![],
        });
        let spec = ZngurSpec {
            types: vec![ZngurType {
                trivially_relocatable: TriviallyRelocatable::No,
                ..minimal_stack_owned_type(ty, 1, 1)
            }],
            ..Default::default()
        };
        let (_rust_code, h, _cpp) =
            ZngurGenerator::build_from_zng(spec, "test_crate".to_owned()).render(false);
        // Only `rust::Unit` is an alias of `rust::Tuple<>`.
        assert!(h.contains("struct Unit {"), "{h}");
    }

    #[test]
    fn cpp_type_gets_nested_module() {
        let spec = ZngurSpec {
            types: vec![minimal_ref_type(cpp(&["a", "b", "Name"]))],
            ..Default::default()
        };
        let (rust_code, _h, _cpp) =
            ZngurGenerator::build_from_zng(spec, "test_crate".to_owned()).render(false);
        assert!(rust_code.contains("pub mod a {"));
        assert!(rust_code.contains("pub mod b {"));
        assert!(rust_code.contains("pub struct Name(());"));
    }

    #[test]
    fn cpp_type_with_layout_and_trivially_relocatable_is_held_by_value() {
        let spec = ZngurSpec {
            types: vec![minimal_stack_owned_type(cpp(&["geo", "Point"]), 16, 8)],
            ..Default::default()
        };
        let (rust_code, h, _cpp) =
            ZngurGenerator::build_from_zng(spec, "test_crate".to_owned()).render(false);
        assert!(rust_code.contains("pub mod geo {"));
        assert!(rust_code.contains("pub struct Point {"));
        assert!(rust_code.contains("unsafe impl ::zngur_lib::ZngCppStackObject for Point {}"));
        assert!(h.contains(
            "static_assert(::rust::is_trivially_relocatable< ::geo::Point >::value, \"Type must \
             be trivially relocatable\");"
        ));
    }

    #[test]
    fn cpp_type_with_unchecked_trivially_relocatable_is_held_by_value_without_the_check() {
        let spec = ZngurSpec {
            types: vec![ZngurType {
                trivially_relocatable: TriviallyRelocatable::Unchecked,
                ..minimal_stack_owned_type(cpp(&["geo", "Point"]), 16, 8)
            }],
            ..Default::default()
        };
        let (rust_code, h, _cpp) =
            ZngurGenerator::build_from_zng(spec, "test_crate".to_owned()).render(false);
        assert!(rust_code.contains("unsafe impl ::zngur_lib::ZngCppStackObject for Point {}"));
        // The layout is still checked, but not that the type is trivially relocatable.
        assert!(h.contains("static_assert(sizeof(::geo::Point) == 16, \"Size mismatch\");"));
        assert!(
            !h.contains("is_trivially_relocatable< ::geo::Point >"),
            "{h}"
        );
    }

    #[test]
    fn cpp_type_with_layout_but_not_trivially_relocatable_is_reference_only() {
        let spec = ZngurSpec {
            types: vec![ZngurType {
                trivially_relocatable: TriviallyRelocatable::No,
                ..minimal_stack_owned_type(cpp(&["geo", "Point"]), 16, 8)
            }],
            ..Default::default()
        };
        let (rust_code, h, _cpp) =
            ZngurGenerator::build_from_zng(spec, "test_crate".to_owned()).render(false);
        // Zero-sized in Rust...
        assert!(rust_code.contains("pub struct Point(());"));
        assert!(!rust_code.contains("ZngCppStackObject"));
        // ...but the layout is still checked in C++.
        assert!(h.contains("static_assert(sizeof(::geo::Point) == 16, \"Size mismatch\");"));
        assert!(h.contains("static_assert(alignof(::geo::Point) == 8, \"Alignment mismatch\");"));
    }

    #[test]
    fn imported_cpp_types_are_reexported_from_their_crate() {
        let spec = ZngurSpec {
            imported_modules: vec![ModuleImport {
                path: "./bar.zng".into(),
                crate_name: Some("bar-bindings".to_owned()),
                cpp_types: vec![
                    vec!["ns".to_owned(), "Bar".to_owned()],
                    vec!["Top".to_owned()],
                ],
                cpp_by_value_types: vec![],
                cpp_template_homes: vec![],
            }],
            // A local `c++::` type in the same namespace shares the module.
            types: vec![minimal_ref_type(cpp(&["ns", "Local"]))],
            ..Default::default()
        };
        let (rust_code, h, _cpp) =
            ZngurGenerator::build_from_zng(spec, "test_crate".to_owned()).render(false);
        assert!(rust_code.contains("pub use ::bar_bindings::generated::Top;"));
        let ns_module = rust_code
            .split("pub mod ns {")
            .nth(1)
            .expect("no `ns` module");
        assert!(ns_module.contains("pub use ::bar_bindings::generated::ns::Bar;"));
        assert!(ns_module.contains("pub struct Local"));
        assert_eq!(rust_code.matches("pub mod ns {").count(), 1);
        assert!(h.contains("#include \"./bar.zng.h\""));
    }

    #[test]
    fn cpp_stack_owned_destructor_names_match_and_are_unique() {
        let spec = ZngurSpec {
            types: vec![
                minimal_stack_owned_type(cpp(&["a", "Point"]), 16, 8),
                minimal_stack_owned_type(cpp(&["b", "Point"]), 16, 8),
            ],
            mangling_base: "my_base".to_owned(),
            ..Default::default()
        };
        let (rust_code, _h, cpp) =
            ZngurGenerator::build_from_zng(spec, "test_crate".to_owned()).render(false);
        let cpp = cpp.expect("stack owned types need a C++ file for their destructors");
        for (name, cpp_type) in [
            ("_zngur_my_base_zngur_a__Point_destructor_", "::a::Point"),
            ("_zngur_my_base_zngur_b__Point_destructor_", "::b::Point"),
        ] {
            // Rust calls the destructor by the same (mangled) name that C++ defines it with.
            assert!(
                rust_code.contains(&format!("fn {name}(")),
                "{name} not in:\n{rust_code}"
            );
            assert!(
                cpp.contains(&format!("void {name}(")),
                "{name} not in:\n{cpp}"
            );
            assert!(cpp.contains(&format!("using __zngur_T = {cpp_type};")));
        }
    }

    #[test]
    fn cpp_types_sharing_a_prefix_merge_into_a_single_module() {
        let spec = ZngurSpec {
            types: vec![
                minimal_ref_type(cpp(&["a", "Foo"])),
                minimal_ref_type(cpp(&["a", "Bar"])),
            ],
            ..Default::default()
        };
        let (rust_code, _h, _cpp) =
            ZngurGenerator::build_from_zng(spec, "test_crate".to_owned()).render(false);
        // Both structs must land inside a single, merged `pub mod a { ... }`.
        let mod_a_count = rust_code.matches("pub mod a {").count();
        assert_eq!(
            mod_a_count, 1,
            "expected exactly one `pub mod a {{` block, found {mod_a_count} in:\n{rust_code}"
        );
        assert!(rust_code.contains("pub struct Foo"));
        assert!(rust_code.contains("pub struct Bar"));
    }

    #[test]
    fn cpp_methods_get_rust_functions_and_cpp_bridges() {
        let b = cpp(&["a", "B"]);
        let i32 = RustType::Primitive(PrimitiveRustType::Int(32));
        let mut ty = minimal_stack_owned_type(b.clone(), 8, 4);
        ty.cpp_methods = vec![
            ZngurCppMethod {
                cpp_name: "value".to_owned(),
                template_args: vec![],
                rust_name: None,
                receiver: ZngurMethodReceiver::Ref(Mutability::Not),
                inputs: vec![],
                output: i32.clone(),
                is_safe: false,
            },
            ZngurCppMethod {
                cpp_name: "set".to_owned(),
                template_args: vec![],
                rust_name: Some("set_from".to_owned()),
                receiver: ZngurMethodReceiver::Ref(Mutability::Mut),
                inputs: vec![RustType::cpp_ptr(
                    Mutability::Not,
                    CppIndirection::Reference,
                    b.clone(),
                )],
                output: RustType::UNIT,
                is_safe: false,
            },
            ZngurCppMethod {
                cpp_name: "make".to_owned(),
                template_args: vec![],
                rust_name: None,
                receiver: ZngurMethodReceiver::Static,
                inputs: vec![i32.clone(), b.clone()],
                output: b.clone(),
                is_safe: true,
            },
        ];
        let spec = ZngurSpec {
            types: vec![ty],
            ..Default::default()
        };
        let (rust_code, _h, cpp_code) =
            ZngurGenerator::build_from_zng(spec, "test_crate".to_owned()).render(false);
        let cpp_code = cpp_code.expect("C++ member functions need a C++ file");

        // Rust: an associated function per member function, taking `this` as a `CppRef`.
        assert!(rust_code.contains(
            "pub unsafe fn value(i0: ::zngur_lib::CppRef<::zngur_lib::Const<a::B>>, ) -> i32"
        ));
        assert!(rust_code.contains(
            "pub unsafe fn set_from(i0: ::zngur_lib::CppRef<a::B>, \
             i1: ::zngur_lib::CppRef<::zngur_lib::Const<a::B>>, ) -> ()"
        ));
        assert!(rust_code.contains("pub fn make(i0: i32, i1: a::B, ) -> a::B"));

        // C++: bridges calling the member functions directly.
        assert!(cpp_code.contains("->value()"), "{cpp_code}");
        assert!(cpp_code.contains("->set(*"), "{cpp_code}");
        assert!(
            cpp_code.contains("new (o) ::a::B(::a::B::make("),
            "{cpp_code}"
        );
        assert!(cpp_code.contains(".cpp())"), "{cpp_code}");
    }

    #[test]
    fn cpp_template_instantiations_share_an_alias() {
        let b = cpp(&["a", "B"]);
        let unique_ptr = |arg: RustType| {
            RustType::Cpp(CppTypePath {
                path: vec!["std".to_owned(), "unique_ptr".to_owned()],
                generics: vec![arg],
            })
        };
        let spec = ZngurSpec {
            types: vec![
                minimal_ref_type(b.clone()),
                minimal_stack_owned_type(unique_ptr(b.clone()), 8, 8),
                minimal_ref_type(unique_ptr(RustType::Primitive(PrimitiveRustType::Int(32)))),
            ],
            ..Default::default()
        };
        let (rust_code, _h, _cpp) =
            ZngurGenerator::build_from_zng(spec, "test_crate".to_owned()).render(false);

        // One trait and alias for the template, in the template's module.
        assert!(rust_code.contains("pub mod std {"), "{rust_code}");
        assert_eq!(
            rust_code
                .matches("pub trait __ZngurCppTemplate_unique_ptr {")
                .count(),
            1,
            "{rust_code}"
        );
        assert_eq!(
            rust_code
                .matches("pub type unique_ptr<T0> = <T0 as __ZngurCppTemplate_unique_ptr>::Type;")
                .count(),
            1,
            "{rust_code}"
        );
        // Each instantiation is a struct that the trait names for its arguments.
        assert!(
            rust_code.contains(
                "impl std::__ZngurCppTemplate_unique_ptr for a::B {\n    type Type = \
                 __zngur_cpp_std__unique_ptr__a__B;\n}"
            ),
            "{rust_code}"
        );
        assert!(
            rust_code.contains(
                "impl std::__ZngurCppTemplate_unique_ptr for i32 {\n    type Type = \
                 __zngur_cpp_std__unique_ptr__i32;\n}"
            ),
            "{rust_code}"
        );
        // Only by-value instantiations are `ZngCppStackObject`s.
        assert!(
            rust_code.contains(
                "unsafe impl ::zngur_lib::ZngCppStackObject for __zngur_cpp_std__unique_ptr__a__B {}"
            ),
            "{rust_code}"
        );
        assert!(
            !rust_code.contains(
                "unsafe impl ::zngur_lib::ZngCppStackObject for __zngur_cpp_std__unique_ptr__i32 {}"
            ),
            "{rust_code}"
        );
    }

    #[test]
    fn cpp_template_instantiations_use_an_imported_home() {
        let b = cpp(&["a", "B"]);
        let unique_ptr_path = vec!["std".to_owned(), "unique_ptr".to_owned()];
        let spec = ZngurSpec {
            types: vec![
                minimal_ref_type(b.clone()),
                minimal_stack_owned_type(
                    RustType::Cpp(CppTypePath {
                        path: unique_ptr_path.clone(),
                        generics: vec![b.clone()],
                    }),
                    8,
                    8,
                ),
            ],
            imported_modules: vec![ModuleImport {
                path: "./bar.zng".into(),
                crate_name: Some("bar".to_owned()),
                cpp_types: vec![],
                cpp_by_value_types: vec![],
                cpp_template_homes: vec![CppTemplateHome {
                    path: unique_ptr_path,
                    arity: 1,
                    explicit: true,
                }],
            }],
            ..Default::default()
        };
        let (rust_code, _h, _cpp) =
            ZngurGenerator::build_from_zng(spec, "test_crate".to_owned()).render(false);

        assert!(
            !rust_code.contains("pub trait __ZngurCppTemplate_unique_ptr"),
            "{rust_code}"
        );
        assert!(
            rust_code.contains("pub use ::bar::generated::std::unique_ptr;"),
            "{rust_code}"
        );
        assert!(
            rust_code.contains(
                "impl ::bar::generated::std::__ZngurCppTemplate_unique_ptr for a::B {\n    type Type = \
                 __zngur_cpp_std__unique_ptr__a__B;\n}"
            ),
            "{rust_code}"
        );
    }

    #[test]
    fn cpp_fields_get_accessors_and_cpp_bridges() {
        let b = cpp(&["a", "B"]);
        let mut ty = minimal_stack_owned_type(b, 8, 4);
        ty.cpp_fields = vec![ZngurCppField {
            name: "x".to_owned(),
            rust_name: Some("x_mut".to_owned()),
            ty: RustType::Primitive(PrimitiveRustType::Int(32)),
            mutability: Mutability::Mut,
        }];
        let spec = ZngurSpec {
            types: vec![ty],
            ..Default::default()
        };
        let (rust_code, _h, cpp_code) =
            ZngurGenerator::build_from_zng(spec, "test_crate".to_owned()).render(false);
        let cpp_code = cpp_code.expect("C++ field accessors need a C++ file");

        assert!(
            rust_code.contains(
                "pub fn x_mut(i0: ::zngur_lib::CppRef<a::B>, ) -> ::zngur_lib::CppRef<i32>"
            )
        );
        assert!(
            cpp_code.contains(
                "static_assert(::std::is_same< ::std::remove_cv< decltype(::a::B::x) >::type, "
            ),
            "{cpp_code}"
        );
        assert!(
            cpp_code.contains(
                "::rust::CppPtr< ::int32_t > __zngur_o = \
                 &(::rust::__zngur_internal_move_from_rust< ::rust::CppPtr< ::rust::cpp::a::B > \
                 >(i0))->x;"
            ),
            "{cpp_code}"
        );
    }

    #[test]
    fn const_cpp_fields_and_template_arguments() {
        let b = cpp(&["a", "B"]);
        let i32 = RustType::Primitive(PrimitiveRustType::Int(32));
        let mut ty = minimal_stack_owned_type(b, 8, 4);
        ty.cpp_fields = vec![ZngurCppField {
            name: "x".to_owned(),
            rust_name: None,
            ty: i32.clone(),
            mutability: Mutability::Not,
        }];
        let span = RustType::Cpp(CppTypePath {
            path: vec!["std".to_owned(), "span".to_owned()],
            generics: vec![RustType::CppConst(Box::new(i32))],
        });
        let spec = ZngurSpec {
            types: vec![ty, minimal_stack_owned_type(span, 16, 8)],
            ..Default::default()
        };
        let (rust_code, h, cpp_code) =
            ZngurGenerator::build_from_zng(spec, "test_crate".to_owned()).render(false);
        let cpp_code = cpp_code.expect("C++ field accessors need a C++ file");

        assert!(rust_code.contains(
            "pub fn x(i0: ::zngur_lib::CppRef<::zngur_lib::Const<a::B>>, ) -> \
             ::zngur_lib::CppRef<::zngur_lib::Const<i32>>"
        ));
        assert!(
            cpp_code.contains(
                "::rust::CppPtr< ::rust::CppConst< ::int32_t > > __zngur_o = \
                 &(::rust::__zngur_internal_move_from_rust< ::rust::CppPtr< ::rust::CppConst< \
                 ::rust::cpp::a::B > > >(i0))->x;"
            ),
            "{cpp_code}"
        );
        assert!(
            rust_code.contains("impl std::__ZngurCppTemplate_span for ::zngur_lib::Const<i32> {"),
            "{rust_code}"
        );
        assert!(
            h.contains("static_assert(sizeof(::std::span< const ::int32_t >) == 16"),
            "{h}"
        );
    }

    #[test]
    fn cpp_functions_get_rust_functions_in_the_cpp_tree_and_cpp_bridges() {
        let i32 = RustType::Primitive(PrimitiveRustType::Int(32));
        let spec = ZngurSpec {
            cpp_funcs: vec![ZngurCppFn {
                namespace: vec!["a".to_owned(), "b".to_owned()],
                cpp_name: "f".to_owned(),
                template_args: vec![],
                rust_name: Some("g".to_owned()),
                inputs: vec![i32.clone()],
                output: i32,
                is_safe: true,
            }],
            ..Default::default()
        };
        let (rust_code, _h, cpp_code) =
            ZngurGenerator::build_from_zng(spec, "test_crate".to_owned()).render(false);
        let cpp_code = cpp_code.expect("C++ functions need a C++ file");

        assert!(rust_code.contains("pub fn __zngur_cpp_fn__a__b__g(i0: i32, ) -> i32"));
        assert!(rust_code.contains("pub mod a {"));
        assert!(rust_code.contains("pub mod b {"));
        assert!(rust_code.contains("pub use super::super::__zngur_cpp_fn__a__b__g as g;"));
        assert!(
            cpp_code.contains(
                "::int32_t __zngur_o = ::a::b::f(::rust::__zngur_internal_move_from_rust< \
                 ::int32_t >(i0));"
            ),
            "{cpp_code}"
        );
    }

    #[test]
    fn cpp_constructors_get_construct_impls_and_cpp_bridges() {
        let b = cpp(&["a", "B"]);
        let i32 = RustType::Primitive(PrimitiveRustType::Int(32));
        let mut ty = minimal_stack_owned_type(b.clone(), 8, 4);
        ty.cpp_constructors = vec![
            ZngurCppConstructor {
                inputs: vec![],
                is_safe: true,
            },
            ZngurCppConstructor {
                inputs: vec![
                    i32,
                    RustType::cpp_ptr(Mutability::Not, CppIndirection::Reference, b),
                ],
                is_safe: false,
            },
        ];
        let spec = ZngurSpec {
            types: vec![ty],
            ..Default::default()
        };
        let (rust_code, _h, cpp_code) =
            ZngurGenerator::build_from_zng(spec, "test_crate".to_owned()).render(false);
        let cpp_code = cpp_code.expect("C++ constructors need a C++ file");

        assert!(rust_code.contains("unsafe impl ::zngur_lib::ZngCppConstruct<()> for a::B {"));
        assert!(rust_code.contains("type Safety = ::zngur_lib::IsSafe;"));
        assert!(rust_code.contains(
            "unsafe impl ::zngur_lib::ZngCppConstruct<(i32, \
             ::zngur_lib::CppRef<::zngur_lib::Const<a::B>>, )> for a::B {"
        ));
        assert!(rust_code.contains("type Safety = ::zngur_lib::IsUnsafe;"));

        assert!(cpp_code.contains("new (o) ::a::B();"), "{cpp_code}");
        assert!(
            cpp_code.contains(
                "new (o) ::a::B(::rust::__zngur_internal_move_from_rust< ::int32_t >(i0), *"
            ),
            "{cpp_code}"
        );
    }

    #[test]
    fn cpp_functions_move_imported_by_value_types_like_local_ones() {
        let point = cpp(&["geo", "Point"]);
        let spec = ZngurSpec {
            imported_modules: vec![ModuleImport {
                path: "./geo.zng".into(),
                crate_name: Some("geo".to_owned()),
                cpp_types: vec![vec!["geo".to_owned(), "Point".to_owned()]],
                cpp_by_value_types: vec![(point.clone(), "::geo::Point".to_owned())],
                cpp_template_homes: vec![],
            }],
            cpp_funcs: vec![ZngurCppFn {
                namespace: vec!["shapes".to_owned()],
                cpp_name: "Mirror".to_owned(),
                template_args: vec![],
                rust_name: None,
                inputs: vec![point.clone()],
                output: point,
                is_safe: true,
            }],
            ..Default::default()
        };
        let (_rust_code, _h, cpp_code) =
            ZngurGenerator::build_from_zng(spec, "test_crate".to_owned()).render(false);
        let cpp_code = cpp_code.expect("C++ functions need a C++ file");

        // Constructed in place in the Rust value, from the argument moved out of its wrapper.
        assert!(
            cpp_code.contains(
                "new (o) ::geo::Point(::shapes::Mirror(::rust::__zngur_internal_move_from_rust< \
                 ::rust::cpp::geo::Point >(i0).cpp()));"
            ),
            "{cpp_code}"
        );
    }

    #[test]
    fn cpp_function_templates_are_called_with_their_template_arguments() {
        let point = cpp(&["geo", "Point"]);
        let unique_ptr = RustType::Cpp(CppTypePath {
            path: vec!["std".to_owned(), "unique_ptr".to_owned()],
            generics: vec![point.clone()],
        });
        let i32 = RustType::Primitive(PrimitiveRustType::Int(32));
        let spec = ZngurSpec {
            types: vec![
                minimal_stack_owned_type(point.clone(), 8, 4),
                minimal_stack_owned_type(unique_ptr.clone(), 8, 8),
            ],
            cpp_funcs: vec![ZngurCppFn {
                namespace: vec!["std".to_owned()],
                cpp_name: "make_unique".to_owned(),
                template_args: vec![point],
                rust_name: Some("make_point".to_owned()),
                inputs: vec![i32.clone(), i32],
                output: unique_ptr,
                is_safe: true,
            }],
            ..Default::default()
        };
        let (rust_code, _h, cpp_code) =
            ZngurGenerator::build_from_zng(spec, "test_crate".to_owned()).render(false);
        let cpp_code = cpp_code.expect("C++ functions need a C++ file");

        assert!(rust_code.contains("as make_point;"), "{rust_code}");
        assert!(
            cpp_code.contains(
                "new (o) ::std::unique_ptr< ::geo::Point >(::std::make_unique< ::geo::Point \
                 >(::rust::__zngur_internal_move_from_rust< ::int32_t >(i0), "
            ),
            "{cpp_code}"
        );
    }
}
