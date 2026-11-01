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
use indexmap::map::Entry;
use itertools::Itertools;
use rust::CoroSupport;
use rust::IntoCpp;

pub mod cpp;
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
        for ty_def in zng.types {
            let ty = &ty_def.ty;
            // What Rust can do with a C++ (`c++::`) type: hold it by value if it has a layout and
            // is trivially relocatable, and otherwise only refer to it.
            let cpp_name = ty_def.cpp_name();
            let cpp_by_value_type = if ty_def.cpp_by_value_layout().is_some() {
                cpp_name.clone()
            } else {
                None
            };
            let cpp_ref = if cpp_by_value_type.is_none() && ty_def.cpp_heap_allocated.is_none() {
                cpp_name.map(|name| cpp::CppRef {
                    name,
                    layout: if ty_def.cpp_ref.is_some() {
                        None
                    } else {
                        ty_def.layout
                    },
                })
            } else {
                None
            };
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
            let is_cpp_stack_owned = cpp_by_value_type.is_some();
            match layout {
                LayoutPolicy::StackAllocated { size, align } => {
                    rust_file.add_static_size_assert(&ty, size);
                    rust_file.add_static_align_assert(&ty, align);
                }
                LayoutPolicy::Conservative { size, align } => {
                    let expected_rust_size = if is_cpp_stack_owned {
                        size.next_multiple_of(align)
                    } else {
                        size
                    };
                    rust_file.add_static_size_upper_bound_assert(&ty, expected_rust_size);
                    rust_file.add_static_align_upper_bound_assert(&ty, align);
                }
                LayoutPolicy::HeapAllocated => (),
                LayoutPolicy::OnlyByRef => (),
            }
            if is_copy {
                rust_file.add_static_is_copy_assert(&ty);
            }
            let (type_name, is_cpp, module_path): (String, bool, Vec<String>) = match &ty {
                RustType::Cpp(segs) => {
                    let (name, path) = segs
                        .split_last()
                        .expect("Cpp path must have at least one segment");
                    (name.clone(), true, path.to_vec())
                }
                _ => (
                    ty.to_string().split("::").last().unwrap().to_string(),
                    false,
                    vec![],
                ),
            };
            let mut cpp_stack_owned_data = None;
            if let Some(cpp_type) = cpp_by_value_type {
                // Derive the destructor symbol via `RustFile::mangle_name` so it includes the
                // mangling base and position-encodes `c++::` / module boundaries / generics
                // without colliding across modules or emitting a public Rust wrapper.
                let destructor_key = match &ty {
                    RustType::Cpp(_) => format!("_c++::{ty}=cpp_stack_owned_destructor"),
                    _ => format!("_{ty}=cpp_stack_owned_destructor"),
                };
                let mangled_name = rust_file.mangle_name(&destructor_key);
                cpp_stack_owned_data = Some(CppStackOwnedData {
                    destructor_fn: mangled_name.clone(),
                    cpp_type,
                });
                let (buf_size, align) = match layout {
                    LayoutPolicy::StackAllocated { size, align } => (size, align),
                    LayoutPolicy::Conservative { size, align } => {
                        (size.next_multiple_of(align), align)
                    }
                    _ => unreachable!("C++ types held by value have a size and alignment"),
                };
                let struct_def = format!(
                    r#"
    #[repr(C)]
    #[repr(align({align}))]
    pub struct {type_name} {{
      pub(crate) buffer: ::core::cell::UnsafeCell<::core::mem::MaybeUninit<[u8; {buf_size}]>>,
      _no_auto_traits: ::core::marker::PhantomData<*mut ()>,
      _pinned: ::core::marker::PhantomPinned,
    }}

    unsafe impl ::zngur_lib::ZngCppObject for {type_name} {{}}
    unsafe impl ::zngur_lib::ZngCppStackObject for {type_name} {{}}

    unsafe impl ::zngur_lib::ZngCppDestruct for {type_name} {{
        unsafe fn destruct(&mut self) {{
            unsafe extern "C" {{
                fn {mangled_name}(i0: *mut u8);
            }}
            unsafe {{
                {mangled_name}(self.buffer.get() as *mut _);
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
"#
                );
                if is_cpp {
                    cpp_tree.insert(&module_path, &struct_def);
                } else {
                    rust_file.text.push_str(&struct_def);
                    cpp_tree.insert(
                        &["cpp".to_owned()],
                        &format!(
                            "\n#[allow(dead_code)]\n#[deprecated(note = \"use `{type_name}` directly instead of `cpp::{type_name}`\")]\npub type {type_name} = super::{type_name};\n"
                        ),
                    );
                }
            }
            if ty_def.cpp_heap_allocated.is_some() {
                let struct_def = format!(
                    r#"
    #[repr(C)]
    pub struct {type_name} {{
        pub(crate) data: *mut u8,
        pub(crate) destructor: extern "C" fn(*mut u8),
    }}

    impl Drop for {type_name} {{
        fn drop(&mut self) {{
            (self.destructor)(self.data)
        }}
    }}
"#
                );
                if is_cpp {
                    cpp_tree.insert(&module_path, &struct_def);
                } else {
                    rust_file.text.push_str(&struct_def);
                    cpp_tree.insert(
                        &["cpp".to_owned()],
                        &format!(
                            "\n#[allow(dead_code)]\n#[deprecated(note = \"use `{type_name}` directly instead of `cpp::{type_name}`\")]\npub type {type_name} = super::{type_name};\n"
                        ),
                    );
                }
            }
            if cpp_ref.is_some() {
                let struct_def = format!(
                    r#"
    pub struct {type_name}(());
"#
                );
                if is_cpp {
                    cpp_tree.insert(&module_path, &struct_def);
                } else {
                    rust_file.text.push_str(&struct_def);
                    cpp_tree.insert(
                        &["cpp".to_owned()],
                        &format!(
                            "\n#[allow(dead_code)]\n#[deprecated(note = \"use `{type_name}` directly instead of `cpp::{type_name}`\")]\npub type {type_name} = super::{type_name};\n"
                        ),
                    );
                }
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
                            .with_tail(cpp_handle_keyword(&variant.name)),
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
                    name: cpp_handle_keyword(&variant.name),
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
                    method.is_safe,
                    use_path,
                    deref,
                    default_ns,
                    &sanitized_crate_name,
                );
                cpp_methods.push(CppMethod {
                    name: cpp_handle_keyword(cpp_name),
                    kind: method.receiver,
                    sig,
                });
            }
            cpp_file.type_defs.push(CppTypeDefinition {
                ty: ty.into_cpp(default_ns, &sanitized_crate_name),
                layout: rust_file.add_layout_policy_shim(&ty, layout, is_cpp_stack_owned),
                constructor,
                variants,
                discriminant,
                fields,
                methods: cpp_methods,
                wellknown_traits,
                cpp_heap_allocated: ty_def.cpp_heap_allocated.map(|cpp_heap_allocated| {
                    CppHeapAllocatedData {
                        bridge_fn: rust_file.add_cpp_heap_allocated_bridge(&ty),
                        cpp_type: cpp_heap_allocated.0,
                    }
                }),
                cpp_ref,
                cpp_stack_owned: cpp_stack_owned_data,
                check_trivially_relocatable: ty_def.cpp_stack_owned.is_some()
                    || ty_def.trivially_relocatable == TriviallyRelocatable::Checked,
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
        rust_file.text.push_str(&cpp_tree.render());
        for func in zng.funcs {
            let sig = rust_file.add_function(
                &func.path.to_string(),
                &func.path.to_string(),
                &func.inputs,
                &func.output,
                func.is_safe,
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
                name: cpp_handle_keyword(&func.name),
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
                            cpp_handle_keyword(&method.name),
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

    fn minimal_heap_allocated_type(ty: RustType, cpp_path: &str) -> ZngurType {
        ZngurType {
            ty,
            layout: Some(LayoutPolicy::HeapAllocated),
            trivially_relocatable: TriviallyRelocatable::No,
            wellknown_traits: vec![],
            exhaustive: true,
            methods: vec![],
            constructor: None,
            variants: vec![],
            fields: vec![],
            cpp_heap_allocated: Some(CppHeapAllocated(cpp_path.to_owned())),
            cpp_ref: None,
            cpp_stack_owned: None,
        }
    }

    fn minimal_stack_owned_type(
        ty: RustType,
        cpp_type: &str,
        size: usize,
        align: usize,
    ) -> ZngurType {
        ZngurType {
            ty,
            layout: Some(LayoutPolicy::StackAllocated { size, align }),
            trivially_relocatable: TriviallyRelocatable::No,
            wellknown_traits: vec![],
            exhaustive: true,
            methods: vec![],
            constructor: None,
            variants: vec![],
            fields: vec![],
            cpp_heap_allocated: None,
            cpp_ref: None,
            cpp_stack_owned: Some(CppStackOwned(cpp_type.to_owned())),
        }
    }

    fn minimal_ref_type(ty: RustType, cpp_type: &str) -> ZngurType {
        ZngurType {
            ty,
            layout: Some(LayoutPolicy::ZERO_SIZED_TYPE),
            trivially_relocatable: TriviallyRelocatable::No,
            wellknown_traits: vec![],
            exhaustive: true,
            methods: vec![],
            constructor: None,
            variants: vec![],
            fields: vec![],
            cpp_heap_allocated: None,
            cpp_ref: Some(CppRef(cpp_type.to_owned())),
            cpp_stack_owned: None,
        }
    }

    /// A `c++::` type that Rust holds by value via `#layout` + `#trivially_relocatable`.
    fn minimal_by_value_cpp_type(ty: RustType, size: usize, align: usize) -> ZngurType {
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
            cpp_heap_allocated: None,
            cpp_ref: None,
            cpp_stack_owned: None,
        }
    }

    /// A `c++::` type without legacy directives that Rust only refers to.
    fn minimal_opaque_cpp_type(ty: RustType) -> ZngurType {
        ZngurType {
            layout: None,
            trivially_relocatable: TriviallyRelocatable::No,
            ..minimal_by_value_cpp_type(ty, 0, 1)
        }
    }

    fn adt(segments: &[&str]) -> RustType {
        RustType::Adt(RustPathAndGenerics {
            path: segments.iter().map(|s| s.to_string()).collect(),
            generics: vec![],
            named_generics: vec![],
        })
    }

    fn cpp(segments: &[&str]) -> RustType {
        RustType::Cpp(segments.iter().map(|s| s.to_string()).collect())
    }

    #[test]
    fn rust_type_named_unit_is_not_the_unit_type() {
        assert_eq!(crate::cpp::CppPath::from("other_ns::Unit").name(), "Unit");
        assert_eq!(crate::cpp::CppPath::from("rust::Unit").name(), "Unit");

        let rust_adt = |segments: &[&str], methods: Vec<ZngurMethodDetails>| ZngurType {
            ty: adt(segments),
            layout: Some(LayoutPolicy::StackAllocated { size: 1, align: 1 }),
            trivially_relocatable: TriviallyRelocatable::No,
            wellknown_traits: vec![],
            exhaustive: true,
            methods,
            constructor: None,
            variants: vec![],
            fields: vec![],
            cpp_heap_allocated: None,
            cpp_ref: None,
            cpp_stack_owned: None,
        };
        let noop_method = ZngurMethodDetails {
            data: ZngurMethod {
                name: "noop".to_owned(),
                generics: vec![],
                receiver: ZngurMethodReceiver::Ref(Mutability::Not),
                inputs: vec![],
                output: RustType::UNIT,
                is_safe: None,
            },
            use_path: None,
            deref: None,
            cpp_name: None,
        };
        let mut unit_ty = rust_adt(&["crate", "Unit"], vec![noop_method]);
        unit_ty.constructor = Some(ZngurConstructor {
            inputs: vec![(
                "0".to_owned(),
                RustType::Primitive(PrimitiveRustType::Int(32)),
            )],
        });
        let real_tuple_ty = ZngurType {
            ty: RustType::Tuple(vec![
                RustType::Primitive(PrimitiveRustType::Int(32)),
                RustType::Primitive(PrimitiveRustType::Int(32)),
            ]),
            layout: Some(LayoutPolicy::StackAllocated { size: 8, align: 4 }),
            trivially_relocatable: TriviallyRelocatable::No,
            wellknown_traits: vec![],
            exhaustive: true,
            methods: vec![],
            constructor: None,
            variants: vec![],
            fields: vec![],
            cpp_heap_allocated: None,
            cpp_ref: None,
            cpp_stack_owned: None,
        };
        let spec = ZngurSpec {
            types: vec![
                unit_ty,
                rust_adt(&["my_crate", "Unit"], vec![]),
                rust_adt(&["crate", "Tuple"], vec![]),
                real_tuple_ty,
            ],
            ..Default::default()
        };
        let (_rust_code, h, _cpp) =
            ZngurGenerator::build_from_zng(spec, "test_crate".to_owned()).render(false);
        assert!(
            h.contains("namespace test_crate {\nstruct Unit {"),
            "missing `rust::test_crate::Unit` definition in:\n{h}"
        );
        assert!(
            h.contains("Unit(\n        ::int32_t\n    ) noexcept ;"),
            "missing `Unit(::int32_t)` constructor declaration in:\n{h}"
        );
        assert!(
            h.contains("inline rust::test_crate::Unit::Unit(::int32_t i0) noexcept"),
            "missing `rust::test_crate::Unit::Unit` constructor definition in:\n{h}"
        );
        assert!(
            h.contains("inline ::rust::Unit rust::test_crate::Unit::noop("),
            "missing `::rust::Unit` return type on `crate::Unit::noop` in:\n{h}"
        );
        assert!(
            h.contains("namespace my_crate {\nstruct Unit {"),
            "missing `rust::my_crate::Unit` definition in:\n{h}"
        );
        assert!(
            h.contains("namespace test_crate {\nstruct Tuple {"),
            "missing non-template `rust::test_crate::Tuple` definition in:\n{h}"
        );
        assert!(
            h.contains("template<> struct Tuple< ::int32_t, ::int32_t > {"),
            "missing `template<> struct Tuple< ::int32_t, ::int32_t >` specialization in:\n{h}"
        );
        assert!(
            !h.contains("template<> struct Tuple<  >"),
            "unexpected empty `template<> struct Tuple<  >` specialization in:\n{h}"
        );

        let fn_spec = ZngurSpec {
            funcs: vec![ZngurFn {
                path: RustPathAndGenerics {
                    path: vec!["crate".to_owned(), "Unit".to_owned()],
                    generics: vec![],
                    named_generics: vec![],
                },
                inputs: vec![],
                output: RustType::UNIT,
                is_safe: None,
            }],
            ..Default::default()
        };
        let (fn_rust_code, fn_h, _cpp) =
            ZngurGenerator::build_from_zng(fn_spec, "test_crate".to_owned()).render(false);
        assert!(
            fn_h.contains("namespace test_crate {\ninline ::rust::Unit Unit() noexcept {"),
            "module-scoped free `fn Unit()` was renamed in:\n{fn_h}"
        );
        assert!(
            fn_rust_code.contains("crate::Unit("),
            "missing `crate::Unit(...)` call in:\n{fn_rust_code}"
        );

        let mk_trait = |name: &str| {
            let tr = RustTrait::Normal(RustPathAndGenerics {
                path: vec!["crate".to_owned(), name.to_owned()],
                generics: vec![],
                named_generics: vec![],
            });
            (
                tr.clone(),
                ZngurTrait {
                    tr,
                    methods: vec![],
                },
            )
        };
        let mut traits = indexmap::IndexMap::new();
        let (u_tr, u_def) = mk_trait("Unit");
        let (t_tr, t_def) = mk_trait("Tuple");
        traits.insert(u_tr, u_def);
        traits.insert(t_tr, t_def);
        let trait_spec = ZngurSpec {
            traits,
            ..Default::default()
        };
        let (_trait_rust, trait_h, _trait_cpp) =
            ZngurGenerator::build_from_zng(trait_spec, "test_crate".to_owned()).render(false);
        assert!(
            trait_h.contains(
                "namespace test_crate {\nstruct Unit {\n  public:\n    virtual ~Unit() {};"
            ),
            "missing `trait crate::Unit` class definition in:\n{trait_h}"
        );
        assert!(
            trait_h.contains(
                "namespace test_crate {\nstruct Tuple {\n  public:\n    virtual ~Tuple() {};"
            ),
            "missing `trait crate::Tuple` class definition in:\n{trait_h}"
        );
        assert!(
            !trait_h.contains("template<> struct Tuple"),
            "unexpected `template<> struct Tuple` specialization in:\n{trait_h}"
        );
    }

    #[test]
    fn cpp_stack_owned_conservative_layout_rounds_buffer_to_alignment() {
        let spec = ZngurSpec {
            types: vec![ZngurType {
                ty: adt(&["crate", "Way"]),
                layout: Some(LayoutPolicy::Conservative { size: 20, align: 8 }),
                trivially_relocatable: TriviallyRelocatable::No,
                wellknown_traits: vec![],
                exhaustive: true,
                methods: vec![],
                constructor: None,
                variants: vec![],
                fields: vec![],
                cpp_heap_allocated: None,
                cpp_ref: None,
                cpp_stack_owned: Some(CppStackOwned("::osmium::Way".to_owned())),
            }],
            ..Default::default()
        };
        let (rust_code, h, _cpp) =
            ZngurGenerator::build_from_zng(spec, "test_crate".to_owned()).render(false);
        assert!(rust_code.contains("MaybeUninit<[u8; 24]>"));
        assert!(rust_code.contains("__zngur_assert_size_conservative!(crate::Way, 24);"));
        assert!(rust_code.contains("__zngur_assert_align_conservative!(crate::Way, 8);"));
        assert!(h.contains("::std::array< ::uint8_t, 24> __zngur_data;"));
        assert!(h.contains(
            "static_assert(sizeof(::osmium::Way) <= 20, \"Size exceeds conservative layout\");"
        ));
        assert!(h.contains(
            "static_assert(alignof(::osmium::Way) <= 8, \"Alignment exceeds conservative layout\");"
        ));
    }

    #[test]
    fn old_style_type_gets_top_level_struct_and_deprecated_shim() {
        let spec = ZngurSpec {
            types: vec![minimal_heap_allocated_type(
                adt(&["crate", "Way"]),
                "::osmium::Way",
            )],
            ..Default::default()
        };
        let (rust_code, _h, _cpp) =
            ZngurGenerator::build_from_zng(spec, "test_crate".to_owned()).render(false);
        // Primary struct at top level, not nested under `mod cpp`:
        assert!(rust_code.contains("pub struct Way"));
        let expected_shim = r#"
pub mod cpp {

#[allow(dead_code)]
#[deprecated(note = "use `Way` directly instead of `cpp::Way`")]
pub type Way = super::Way;

}
"#;
        assert!(rust_code.contains(expected_shim));
        // The heap-allocated bridge function must reference the bare
        // top-level name, not the old `cpp::Way` path (which would now
        // resolve to the deprecated shim and self-trigger a deprecation
        // warning under `-D warnings`).
        assert!(rust_code.contains("*mut Way"));
        assert!(!rust_code.contains("*mut cpp::Way"));
    }

    #[test]
    fn cpp_type_gets_nested_module_and_no_cpp_shim() {
        let spec = ZngurSpec {
            types: vec![minimal_heap_allocated_type(
                cpp(&["a", "b", "Name"]),
                "::x::Name",
            )],
            ..Default::default()
        };
        let (rust_code, _h, _cpp) =
            ZngurGenerator::build_from_zng(spec, "test_crate".to_owned()).render(false);
        assert!(rust_code.contains("pub mod a {"));
        assert!(rust_code.contains("pub mod b {"));
        assert!(rust_code.contains("pub struct Name"));
        // No `cpp` compatibility module should be emitted when there are no old-style types:
        assert!(!rust_code.contains("pub mod cpp {"));
        // The heap-allocated bridge function must reference the correct
        // nested path where the struct actually lives, not the old
        // `cpp::Name` path (which doesn't exist for a `RustType::Cpp` type
        // at all).
        assert!(rust_code.contains("*mut a::b::Name"));
        assert!(!rust_code.contains("*mut cpp::"));
    }

    #[test]
    fn cpp_heap_allocated_bridge_references_correct_wrapper_path_for_cpp_type() {
        // Regression test: add_cpp_heap_allocated_bridge (in rust.rs) used to
        // hardcode `cpp::{type_name}` for the bridge function's return/cast
        // type, which was only correct back when the wrapper struct
        // physically lived inside `mod cpp { ... }`. For `RustType::Cpp`
        // types there never was a `cpp::` home at all, so this was a
        // straight compile error waiting to happen once such a type used
        // #cpp_heap_allocated.
        let spec = ZngurSpec {
            types: vec![minimal_heap_allocated_type(
                cpp(&["a", "Name"]),
                "::x::Name",
            )],
            ..Default::default()
        };
        let (rust_code, _h, _cpp) =
            ZngurGenerator::build_from_zng(spec, "test_crate".to_owned()).render(false);
        assert!(rust_code.contains("*mut a::Name"));
        assert!(!rust_code.contains("*mut cpp::Name"));
        assert!(!rust_code.contains("*mut cpp::a::Name"));
    }

    #[test]
    fn mixed_old_and_new_style_types_both_placed_correctly() {
        let spec = ZngurSpec {
            types: vec![
                minimal_heap_allocated_type(adt(&["crate", "Way"]), "::osmium::Way"),
                minimal_heap_allocated_type(cpp(&["a", "Name"]), "::x::Name"),
            ],
            ..Default::default()
        };
        let (rust_code, _h, _cpp) =
            ZngurGenerator::build_from_zng(spec, "test_crate".to_owned()).render(false);
        assert!(rust_code.contains("pub struct Way"));
        assert!(rust_code.contains("pub mod cpp {"));
        assert!(rust_code.contains("pub type Way = super::Way;"));
        assert!(rust_code.contains("pub mod a {"));
        assert!(rust_code.contains("pub struct Name"));
        // The `RustType::Cpp` type must NOT get a deprecated alias:
        assert!(!rust_code.contains("pub type Name = super::Name;"));
    }

    #[test]
    fn cpp_stack_owned_old_and_new_style_placed_correctly() {
        let spec = ZngurSpec {
            types: vec![
                minimal_stack_owned_type(adt(&["crate", "Box2d"]), "::geos::Box2d", 16, 8),
                minimal_stack_owned_type(cpp(&["geo", "Point"]), "::geos::Point", 16, 8),
            ],
            ..Default::default()
        };
        let (rust_code, _h, _cpp) =
            ZngurGenerator::build_from_zng(spec, "test_crate".to_owned()).render(false);
        // Old-style: top-level struct + deprecated shim.
        assert!(rust_code.contains("pub struct Box2d"));
        let expected_shim = r#"
pub mod cpp {

#[allow(dead_code)]
#[deprecated(note = "use `Box2d` directly instead of `cpp::Box2d`")]
pub type Box2d = super::Box2d;

}
"#;
        assert!(rust_code.contains(expected_shim));
        // New-style: nested module, no shim.
        assert!(rust_code.contains("pub mod geo {"));
        assert!(rust_code.contains("pub struct Point"));
        assert!(!rust_code.contains("pub type Point = super::Point;"));
    }

    #[test]
    fn cpp_stack_owned_destructor_names_match_and_are_unique() {
        let cases = [
            (cpp(&["a", "Point"]), "::a::Point"),
            (cpp(&["b", "Point"]), "::b::Point<int>"),
            (cpp(&["a", "b", "Point"]), "::a::b::Point"),
            (cpp(&["a__b", "Point"]), "::a__b::Point"),
            (adt(&["crate", "Point"]), "::crate_ns::Point"),
            (cpp(&["crate__Point"]), "::std::pair<int, int>"),
        ];
        let spec = ZngurSpec {
            types: cases
                .iter()
                .map(|(ty, cpp_ty)| minimal_stack_owned_type(ty.clone(), cpp_ty, 16, 8))
                .collect(),
            mangling_base: "my_base".to_owned(),
            ..Default::default()
        };
        let rust_ref = crate::RustFile::new("my_base");
        let expected: Vec<(String, &str)> = cases
            .iter()
            .map(|(ty, cpp_ty)| {
                let key = match ty {
                    RustType::Cpp(_) => format!("_c++::{ty}=cpp_stack_owned_destructor"),
                    _ => format!("_{ty}=cpp_stack_owned_destructor"),
                };
                (rust_ref.mangle_name(&key), *cpp_ty)
            })
            .collect();
        let unique_names: std::collections::HashSet<&str> =
            expected.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            unique_names.len(),
            expected.len(),
            "destructor symbols collided: {expected:?}"
        );

        let (rust_code, _h, cpp) =
            ZngurGenerator::build_from_zng(spec, "test_crate".to_owned()).render(false);
        let cpp = cpp.expect("stack owned types need a C++ file for their destructors");
        assert!(
            !rust_code.contains("pub fn _zngur_"),
            "unexpected safe wrapper emitted in:\n{rust_code}"
        );
        for (name, cpp_type) in &expected {
            assert!(
                rust_code.contains(&format!("fn {name}(i0: *mut u8);")),
                "{name} extern decl not in:\n{rust_code}"
            );
            assert!(
                rust_code.contains(&format!("{name}(self.buffer.get() as *mut _);")),
                "{name} destruct call not in:\n{rust_code}"
            );
            assert!(
                cpp.contains(&format!("void {name}(uint8_t* i0)")),
                "{name} C++ definition not in:\n{cpp}"
            );
            assert!(
                cpp.contains(&format!("using __zngur_T = {cpp_type};")),
                "type alias for {cpp_type} not in:\n{cpp}"
            );
        }
    }

    #[test]
    fn cpp_ref_old_and_new_style_placed_correctly() {
        let spec = ZngurSpec {
            types: vec![
                minimal_ref_type(adt(&["crate", "Handle"]), "::osmium::Handle"),
                minimal_ref_type(cpp(&["h", "Handle"]), "::x::Handle"),
            ],
            ..Default::default()
        };
        let (rust_code, _h, _cpp) =
            ZngurGenerator::build_from_zng(spec, "test_crate".to_owned()).render(false);
        // Old-style: top-level struct + deprecated shim.
        assert!(rust_code.contains("pub struct Handle(());"));
        let expected_shim = r#"
pub mod cpp {

#[allow(dead_code)]
#[deprecated(note = "use `Handle` directly instead of `cpp::Handle`")]
pub type Handle = super::Handle;

}
"#;
        assert!(rust_code.contains(expected_shim));
        // New-style: nested module, and only a single deprecated shim exists
        // overall (i.e. the `RustType::Cpp` variant did not also get one).
        assert!(rust_code.contains("pub mod h {"));
        assert_eq!(rust_code.matches("#[deprecated").count(), 1);
    }

    #[test]
    fn cpp_types_sharing_a_prefix_merge_into_a_single_module() {
        let spec = ZngurSpec {
            types: vec![
                minimal_heap_allocated_type(cpp(&["a", "Foo"]), "::x::Foo"),
                minimal_heap_allocated_type(cpp(&["a", "Bar"]), "::x::Bar"),
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
        assert!(!rust_code.contains("pub mod cpp {"));
    }

    #[test]
    fn old_style_type_and_cpp_module_named_cpp_merge_into_single_mod_cpp() {
        let spec = ZngurSpec {
            types: vec![
                minimal_heap_allocated_type(adt(&["crate", "Old"]), "::x::Old"),
                minimal_heap_allocated_type(cpp(&["cpp", "New"]), "::x::New"),
            ],
            ..Default::default()
        };
        let (rust_code, _h, _cpp) =
            ZngurGenerator::build_from_zng(spec, "test_crate".to_owned()).render(false);
        assert_eq!(
            rust_code.matches("pub mod cpp {").count(),
            1,
            "expected a single merged `pub mod cpp {{` block in:\n{rust_code}"
        );
        assert!(rust_code.contains("pub struct Old"));
        assert!(rust_code.contains("pub type Old = super::Old;"));
        assert!(rust_code.contains("pub struct New"));
    }

    #[test]
    fn cpp_std_and_core_modules_do_not_shadow_rust_prelude_paths() {
        let my_trait = RustTrait::Normal(RustPathAndGenerics {
            path: vec!["crate".to_owned(), "MyTrait".to_owned()],
            generics: vec![],
            named_generics: vec![],
        });
        let mut traits = indexmap::IndexMap::new();
        traits.insert(
            my_trait.clone(),
            ZngurTrait {
                tr: my_trait.clone(),
                methods: vec![ZngurMethod {
                    name: "run".to_owned(),
                    generics: vec![],
                    receiver: ZngurMethodReceiver::Ref(Mutability::Not),
                    inputs: vec![],
                    output: RustType::UNIT,
                    is_safe: None,
                }],
            },
        );
        let spec = ZngurSpec {
            types: vec![
                minimal_stack_owned_type(cpp(&["std", "String"]), "::std::string", 16, 8),
                minimal_stack_owned_type(cpp(&["core", "Widget"]), "::core::Widget", 16, 8),
                ZngurType {
                    ty: RustType::Boxed(Box::new(RustType::Dyn(my_trait.clone(), vec![]))),
                    layout: Some(LayoutPolicy::StackAllocated { size: 16, align: 8 }),
                    trivially_relocatable: TriviallyRelocatable::No,
                    wellknown_traits: vec![],
                    exhaustive: true,
                    methods: vec![],
                    constructor: None,
                    variants: vec![],
                    fields: vec![],
                    cpp_heap_allocated: None,
                    cpp_ref: None,
                    cpp_stack_owned: None,
                },
                ZngurType {
                    ty: RustType::Ref(Mutability::Not, Box::new(RustType::Dyn(my_trait, vec![]))),
                    layout: Some(LayoutPolicy::StackAllocated { size: 16, align: 8 }),
                    trivially_relocatable: TriviallyRelocatable::No,
                    wellknown_traits: vec![],
                    exhaustive: true,
                    methods: vec![],
                    constructor: None,
                    variants: vec![],
                    fields: vec![],
                    cpp_heap_allocated: None,
                    cpp_ref: None,
                    cpp_stack_owned: None,
                },
                ZngurType {
                    ty: RustType::Boxed(Box::new(RustType::Dyn(
                        RustTrait::Fn {
                            name: "Fn".to_owned(),
                            inputs: vec![],
                            output: Box::new(RustType::UNIT),
                        },
                        vec![],
                    ))),
                    layout: Some(LayoutPolicy::StackAllocated { size: 16, align: 8 }),
                    trivially_relocatable: TriviallyRelocatable::No,
                    wellknown_traits: vec![],
                    exhaustive: true,
                    methods: vec![],
                    constructor: None,
                    variants: vec![],
                    fields: vec![],
                    cpp_heap_allocated: None,
                    cpp_ref: None,
                    cpp_stack_owned: None,
                },
                ZngurType {
                    ty: adt(&["crate", "MyEnum"]),
                    layout: Some(LayoutPolicy::StackAllocated { size: 8, align: 4 }),
                    trivially_relocatable: TriviallyRelocatable::No,
                    wellknown_traits: vec![],
                    exhaustive: true,
                    methods: vec![],
                    constructor: None,
                    variants: vec![ZngurVariant {
                        name: "Val".to_owned(),
                        fields: vec![ZngurField {
                            name: "x".to_owned(),
                            ty: RustType::Primitive(PrimitiveRustType::Int(32)),
                            offset: None,
                        }],
                        exhaustive: true,
                    }],
                    fields: vec![],
                    cpp_heap_allocated: None,
                    cpp_ref: None,
                    cpp_stack_owned: None,
                },
            ],
            traits,
            ..Default::default()
        };
        let (rust_code, _h, _cpp) =
            ZngurGenerator::build_from_zng(spec, "test_crate".to_owned()).render(false);
        assert!(rust_code.contains("pub mod std {"));
        assert!(rust_code.contains("pub mod core {"));
        assert!(rust_code.contains("::core::cell::UnsafeCell<::core::mem::MaybeUninit<"));
        assert!(rust_code.contains("::core::marker::PhantomData<*mut ()>"));
        assert!(rust_code.contains("::core::marker::PhantomPinned"));
        assert_eq!(
            rust_code
                .matches("::std::ptr::write(o as *mut _, r)")
                .count(),
            3
        );
        assert!(rust_code.contains("::std::hint::unreachable_unchecked()"));
        let stripped = rust_code
            .replace("std::String", "")
            .replace("core::Widget", "")
            .replace("::std::", "")
            .replace("::core::", "");
        assert!(
            !stripped.contains("std::") && !stripped.contains("core::"),
            "found unqualified `std::` or `core::` in:\n{rust_code}"
        );
    }

    #[test]
    fn cpp_type_wrappers_live_in_rust_cpp_namespace_and_do_not_collide_with_rust_std() {
        let rust_string = ZngurType {
            ty: adt(&["std", "string", "String"]),
            layout: Some(LayoutPolicy::StackAllocated { size: 24, align: 8 }),
            trivially_relocatable: TriviallyRelocatable::No,
            wellknown_traits: vec![],
            exhaustive: true,
            methods: vec![],
            constructor: None,
            variants: vec![],
            fields: vec![],
            cpp_heap_allocated: None,
            cpp_ref: None,
            cpp_stack_owned: None,
        };
        let spec = ZngurSpec {
            types: vec![
                rust_string,
                minimal_ref_type(cpp(&["std", "string"]), "::std::string"),
            ],
            ..Default::default()
        };
        let (_rust_code, h, _cpp) =
            ZngurGenerator::build_from_zng(spec, "test_crate".to_owned()).render(false);
        assert!(
            h.contains("namespace rust {\nnamespace cpp {\nnamespace std {\nstruct string {"),
            "missing `rust::cpp::std::string` wrapper in:\n{h}"
        );
        assert!(
            h.contains("namespace rust {\nnamespace std {\nnamespace string {\nstruct String {"),
            "missing `rust::std::string::String` wrapper in:\n{h}"
        );
    }

    #[test]
    fn cpp_type_with_layout_and_trivially_relocatable_is_held_by_value() {
        let spec = ZngurSpec {
            types: vec![minimal_by_value_cpp_type(cpp(&["geo", "Point"]), 16, 8)],
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
                ..minimal_by_value_cpp_type(cpp(&["geo", "Point"]), 16, 8)
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
            types: vec![
                ZngurType {
                    trivially_relocatable: TriviallyRelocatable::No,
                    ..minimal_by_value_cpp_type(cpp(&["geo", "Point"]), 16, 8)
                },
                ZngurType {
                    layout: Some(LayoutPolicy::Conservative { size: 20, align: 8 }),
                    trivially_relocatable: TriviallyRelocatable::No,
                    ..minimal_by_value_cpp_type(cpp(&["geo", "Box2d"]), 0, 1)
                },
            ],
            ..Default::default()
        };
        let (rust_code, h, _cpp) =
            ZngurGenerator::build_from_zng(spec, "test_crate".to_owned()).render(false);
        // Zero-sized in Rust...
        assert!(rust_code.contains("pub struct Point(());"));
        assert!(rust_code.contains("pub struct Box2d(());"));
        assert!(!rust_code.contains("ZngCppStackObject"));
        // ...but exact (`==`) and conservative (`<=`) layouts are still checked in C++.
        assert!(h.contains("static_assert(sizeof(::geo::Point) == 16, \"Size mismatch\");"));
        assert!(h.contains("static_assert(alignof(::geo::Point) == 8, \"Alignment mismatch\");"));
        assert!(h.contains("static_assert(sizeof(::geo::Box2d) <= 20, \"Size mismatch\");"));
        assert!(h.contains("static_assert(alignof(::geo::Box2d) <= 8, \"Alignment mismatch\");"));
    }

    #[test]
    fn cpp_type_without_layout_is_reference_only_and_has_no_layout_checks() {
        let spec = ZngurSpec {
            types: vec![minimal_opaque_cpp_type(cpp(&["geo", "Point"]))],
            ..Default::default()
        };
        let (rust_code, h, _cpp) =
            ZngurGenerator::build_from_zng(spec, "test_crate".to_owned()).render(false);
        assert!(rust_code.contains("pub struct Point(());"));
        assert!(!rust_code.contains("ZngCppStackObject"));
        assert!(!h.contains("sizeof(::geo::Point)"));
        assert!(h.contains("inline ::geo::Point& cpp()"));
    }
}
