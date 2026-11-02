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
use cpp_template::{CppTemplates, cpp_template, struct_name};
use indexmap::map::Entry;
use itertools::Itertools;
use rust::CoroSupport;
use rust::CppRustReceiver;
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
        let mut included_headers = std::collections::HashSet::new();
        for module in &zng.imported_modules {
            if included_headers.insert(&module.path) {
                cpp_file
                    .additional_includes
                    .push_str(&format!("\n#include \"{}.h\"", module.path.display()));
            }
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
        let cpp_heap_allocated_types: std::collections::HashSet<RustType> = zng
            .types
            .iter()
            .filter(|ty_def| ty_def.cpp_heap_allocated.is_some())
            .map(|ty_def| ty_def.ty.with_anonymous_lifetimes())
            .chain(
                zng.imported_modules
                    .iter()
                    .flat_map(|m| &m.cpp_heap_allocated_types)
                    .map(|segs| RustType::Cpp(segs.clone().into())),
            )
            .collect();
        let local_cpp_fn_paths: std::collections::HashSet<Vec<String>> = zng
            .cpp_funcs
            .iter()
            .map(|func| {
                let mut p = func.namespace.clone();
                p.push(func.rust_name().to_owned());
                p
            })
            .collect();
        let cpp_templates = CppTemplates::new(
            &zng.cpp_template_homes,
            &zng.imported_modules,
            &zng.types,
            &local_cpp_fn_paths,
            &mut rust_file,
            &mut cpp_tree,
        );
        for mut ty_def in zng.types {
            let raw_lifetimes = ty_def.lifetimes.clone();
            let raw_ty = ty_def.ty.clone();
            let raw_field_tys: Vec<RustType> = ty_def.fields.iter().map(|f| f.ty.clone()).collect();
            // The glue names types with lifetime parameters (`type<'a> crate::Parser<'a>`) with
            // `'_`, since it doesn't declare them. The same goes for the signatures of Rust
            // methods, which zngur only calls.
            anonymize_lifetimes(&mut ty_def);
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
                cpp_name.clone().map(|name| cpp::CppRef {
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
            // C++ member functions that Rust calls: a Rust function, and a C++ function that
            // calls the member function.
            if ty_def.cpp_heap_allocated.is_none()
                && let Some(owner_cpp_type) = &cpp_name
            {
                for method in &ty_def.cpp_methods {
                    let receiver = match (method.receiver, method.raw_receiver) {
                        (ZngurMethodReceiver::Ref(mutability), None) => CppRustReceiver::Ref {
                            mutability,
                            constness: method.receiver_constness,
                            lifetime: method.receiver_lifetime.clone(),
                        },
                        (ZngurMethodReceiver::Ref(_), Some(_)) => CppRustReceiver::Raw {
                            constness: method.receiver_constness,
                        },
                        (ZngurMethodReceiver::Static | ZngurMethodReceiver::Move, _) => {
                            CppRustReceiver::Static
                        }
                    };
                    let mangled_name = rust_file.add_cpp_method(
                        ty,
                        method.rust_name(),
                        &method.lifetimes,
                        receiver,
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
                // Field accessors: like member functions that return a reference to the field or
                // a copy of it.
                for field in &ty_def.cpp_fields {
                    let method = field.as_cpp_method();
                    let ZngurMethodReceiver::Ref(mutability) = method.receiver else {
                        unreachable!("field accessors have a receiver");
                    };
                    let mangled_name = rust_file.add_cpp_method(
                        ty,
                        field.rust_name(),
                        &[],
                        CppRustReceiver::Ref {
                            mutability,
                            constness: method.receiver_constness,
                            lifetime: None,
                        },
                        &[],
                        &method.output,
                        field.is_safe,
                    );
                    cpp_file.cpp_method_bridges.push(bridges.field(
                        &mangled_name,
                        ty,
                        owner_cpp_type,
                        field,
                    ));
                }
                // C++ constructors (by-value types only, checked by the parser): an associated
                // function returning a `CppCtor`, and a C++ function that runs the constructor in
                // place.
                for constructor in &ty_def.cpp_constructors {
                    let mangled_name = rust_file.add_cpp_constructor(ty, constructor);
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
            // The Rust wrapper of a C++ type is in the module of its `c++::` path, except for
            // instantiations of C++ class templates, which are structs at the root that their
            // template's alias names.
            let template = cpp_template(ty);
            let (type_name, is_cpp, module_path): (String, bool, Vec<String>) = match &ty {
                RustType::Cpp(path) if template.is_some() => (struct_name(path), true, vec![]),
                RustType::Cpp(path) => {
                    let (name, path) = path
                        .path
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
            let insert_struct = |rust_file: &mut RustFile,
                                 cpp_tree: &mut CppModuleTree,
                                 def: &str| {
                if template.is_some() {
                    rust_file.text.push_str(def);
                } else if is_cpp {
                    cpp_tree.insert(&module_path, def);
                } else {
                    rust_file.text.push_str(def);
                    cpp_tree.insert(
                        &["cpp".to_owned()],
                        &format!(
                            "\n#[allow(dead_code)]\n#[deprecated(note = \"use `{type_name}` directly instead of `cpp::{type_name}`\")]\npub type {type_name} = super::{type_name};\n"
                        ),
                    );
                }
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
    /// `{type_name}<::zngur_lib::Const>` is a `const` object (see `zngur_lib::Constness`).
    #[repr(C)]
    #[repr(align({align}))]
    #[allow(non_camel_case_types)]
    pub struct {type_name}<C: ::zngur_lib::Constness = ::zngur_lib::NonConst> {{
      pub(crate) buffer: ::core::cell::UnsafeCell<::core::mem::MaybeUninit<[u8; {buf_size}]>>,
      _no_auto_traits: ::core::marker::PhantomData<*mut ()>,
      _pinned: ::core::marker::PhantomPinned,
      _constness: ::core::marker::PhantomData<C>,
    }}

    unsafe impl<C: ::zngur_lib::Constness> ::zngur_lib::ZngCppObject for {type_name}<C> {{}}
    unsafe impl<C: ::zngur_lib::Constness> ::zngur_lib::ZngCppStackObject for {type_name}<C> {{}}

    unsafe impl<C: ::zngur_lib::Constness> ::zngur_lib::ZngCppDestruct for {type_name}<C> {{
        unsafe fn destruct(&mut self) {{
            unsafe extern "C" {{
                fn {mangled_name}(i0: *mut u8);
            }}
            unsafe {{
                {mangled_name}(self.buffer.get() as *mut _);
            }}
        }}
    }}

    impl<C: ::zngur_lib::Constness> Drop for {type_name}<C> {{
        fn drop(&mut self) {{
            use ::zngur_lib::ZngCppDestruct;
            unsafe {{
                self.destruct();
            }}
        }}
    }}

    impl {type_name} {{
        /// This object, as a `const` object.
        #[allow(dead_code)]
        pub fn as_const(&self) -> &{type_name}<::zngur_lib::Const> {{
            unsafe {{ &*(self as *const Self as *const {type_name}<::zngur_lib::Const>) }}
        }}

        /// This object, as a `const` object that nothing else can access meanwhile.
        #[allow(dead_code)]
        pub fn as_const_mut(&mut self) -> &mut {type_name}<::zngur_lib::Const> {{
            unsafe {{ &mut *(self as *mut Self as *mut {type_name}<::zngur_lib::Const>) }}
        }}

        /// Makes this object `const`.
        #[allow(dead_code)]
        pub fn into_const(self) -> {type_name}<::zngur_lib::Const> {{
            let this = ::core::mem::ManuallyDrop::new(self);
            unsafe {{ ::core::ptr::read(&*this as *const Self as *const {type_name}<::zngur_lib::Const>) }}
        }}
    }}
"#
                );
                insert_struct(&mut rust_file, &mut cpp_tree, &struct_def);
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
                insert_struct(&mut rust_file, &mut cpp_tree, &struct_def);
            }
            if cpp_ref.is_some() {
                let struct_def = format!(
                    r#"
    /// `{type_name}<::zngur_lib::Const>` is a `const` object (see `zngur_lib::Constness`).
    #[allow(non_camel_case_types)]
    pub struct {type_name}<C: ::zngur_lib::Constness = ::zngur_lib::NonConst> {{
        _constness: ::core::marker::PhantomData<C>,
        _cell: ::core::cell::UnsafeCell<()>,
        _no_auto_traits: ::core::marker::PhantomData<*mut ()>,
        _pinned: ::core::marker::PhantomPinned,
    }}

    unsafe impl<C: ::zngur_lib::Constness> ::zngur_lib::ZngCppObject for {type_name}<C> {{}}

    impl {type_name} {{
        /// This object, as a `const` object.
        #[allow(dead_code)]
        pub fn as_const(&self) -> &{type_name}<::zngur_lib::Const> {{
            unsafe {{ &*(self as *const Self as *const {type_name}<::zngur_lib::Const>) }}
        }}

        /// This object, as a `const` object that nothing else can access meanwhile.
        #[allow(dead_code)]
        pub fn as_const_mut(&mut self) -> &mut {type_name}<::zngur_lib::Const> {{
            unsafe {{ &mut *(self as *mut Self as *mut {type_name}<::zngur_lib::Const>) }}
        }}
    }}
"#
                );
                insert_struct(&mut rust_file, &mut cpp_tree, &struct_def);
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
            for (field, raw_field_ty) in ty_def.fields.into_iter().zip(&raw_field_tys) {
                let extern_mn = rust_file.add_field_assertions(
                    &field,
                    &ty_def.ty,
                    &raw_lifetimes,
                    &raw_ty,
                    raw_field_ty,
                );
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
                            lifetimes,
                            inputs,
                            output,
                        } = tr
                        {
                            if let Entry::Vacant(e) = cpp_file.trait_defs.entry(tr.clone()) {
                                let rust_link_name = rust_file
                                    .add_builder_for_dyn_fn(name, lifetimes, inputs, output);
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
        let mut reexported = std::collections::HashSet::new();
        for module in &zng.imported_modules {
            let Some(crate_name) = &module.crate_name else {
                continue;
            };
            let module_crate = crate_name.replace('-', "_");
            for segs in &module.cpp_types {
                let Some((type_name, module_path)) = segs.split_last() else {
                    continue;
                };
                if !reexported.insert(segs.clone()) {
                    continue;
                }
                let item = if local_cpp_fn_paths.contains(segs) {
                    let is_heap = module.cpp_heap_allocated_types.contains(segs);
                    if is_heap {
                        format!(
                            "\n#[allow(dead_code, non_camel_case_types)]\npub type {type_name} = \
                             ::{module_crate}::generated::{};\n",
                            segs.join("::")
                        )
                    } else {
                        format!(
                            "\n#[allow(dead_code, non_camel_case_types)]\npub type {type_name}<C = \
                             ::zngur_lib::NonConst> = ::{module_crate}::generated::{}<C>;\n",
                            segs.join("::")
                        )
                    }
                } else {
                    format!(
                        "\n#[allow(unused_imports)]\npub use ::{module_crate}::generated::{};\n",
                        segs.join("::")
                    )
                };
                cpp_tree.insert(module_path, &item);
            }
        }
        // Free C++ functions: a Rust function that calls a C++ function that calls the C++
        // function. The Rust function is defined at the top level, where the generated code's
        // paths resolve, and re-exported in the `c++::` tree, at the C++ namespace's path.
        for func in &zng.cpp_funcs {
            let rust_name = func.rust_name();
            let (hidden_name, mangled_name) = rust_file.add_cpp_function(
                &func.namespace,
                rust_name,
                &func.lifetimes,
                &func.inputs,
                &func.output,
                func.is_safe,
            );
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
            // zngur only calls Rust functions, so their lifetimes don't matter here.
            let rust_path = func.path.with_anonymous_lifetimes().to_string();
            let inputs: Vec<RustType> = func
                .inputs
                .iter()
                .map(RustType::with_anonymous_lifetimes)
                .collect();
            let sig = rust_file.add_function(
                &rust_path,
                &rust_path,
                &inputs,
                &func.output.with_anonymous_lifetimes(),
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
                &func.lifetimes,
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
                &impl_block.lifetimes,
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
                        let receiver_ty = if matches!(impl_block.ty, RustType::Cpp(_))
                            && !cpp_heap_allocated_types
                                .contains(&impl_block.ty.with_anonymous_lifetimes())
                        {
                            RustType::CppVar(Box::new(impl_block.ty.clone()))
                        } else {
                            impl_block.ty.clone()
                        };
                        let inputs = real_inputs_of_method(method, &receiver_ty);
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

/// Replaces the named lifetimes in the glue-only parts of `ty_def` (the type itself, the
/// signatures of its Rust methods, its fields and constructors) with `'_`.
fn anonymize_lifetimes(ty_def: &mut ZngurType) {
    let anon = |t: &mut RustType| *t = t.with_anonymous_lifetimes();
    anon(&mut ty_def.ty);
    for method in &mut ty_def.methods {
        method.data.lifetimes.clear();
        method.data.receiver_lifetime = None;
        method.data.inputs.iter_mut().for_each(anon);
        anon(&mut method.data.output);
        method.data.generics.iter_mut().for_each(anon);
        if let Some((deref, _)) = &mut method.deref {
            anon(deref);
        }
    }
    for field in &mut ty_def.fields {
        anon(&mut field.ty);
    }
    for variant in &mut ty_def.variants {
        for field in &mut variant.fields {
            anon(&mut field.ty);
        }
    }
    if let Some(constructor) = &mut ty_def.constructor {
        for (_, t) in &mut constructor.inputs {
            anon(t);
        }
    }
}

fn real_inputs_of_method(method: &ZngurMethod, ty: &RustType) -> Vec<RustType> {
    let receiver_type = match method.receiver {
        ZngurMethodReceiver::Static => None,
        ZngurMethodReceiver::Ref(m) => Some(RustType::Ref(m, None, Box::new(ty.clone()))),
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

    use crate::{ZngurGenerator, struct_name};

    fn minimal_heap_allocated_type(ty: RustType, cpp_path: &str) -> ZngurType {
        ZngurType {
            ty,
            lifetimes: vec![],
            layout: Some(LayoutPolicy::HeapAllocated),
            trivially_relocatable: TriviallyRelocatable::No,
            wellknown_traits: vec![],
            exhaustive: true,
            methods: vec![],
            cpp_methods: vec![],
            cpp_constructors: vec![],
            cpp_fields: vec![],
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
            lifetimes: vec![],
            layout: Some(LayoutPolicy::StackAllocated { size, align }),
            trivially_relocatable: TriviallyRelocatable::No,
            wellknown_traits: vec![],
            exhaustive: true,
            methods: vec![],
            cpp_methods: vec![],
            cpp_constructors: vec![],
            cpp_fields: vec![],
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
            lifetimes: vec![],
            layout: Some(LayoutPolicy::ZERO_SIZED_TYPE),
            trivially_relocatable: TriviallyRelocatable::No,
            wellknown_traits: vec![],
            exhaustive: true,
            methods: vec![],
            cpp_methods: vec![],
            cpp_constructors: vec![],
            cpp_fields: vec![],
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
            lifetimes: vec![],
            layout: Some(LayoutPolicy::StackAllocated { size, align }),
            trivially_relocatable: TriviallyRelocatable::Checked,
            wellknown_traits: vec![],
            exhaustive: true,
            methods: vec![],
            cpp_methods: vec![],
            cpp_constructors: vec![],
            cpp_fields: vec![],
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
            ..Default::default()
        })
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
        assert_eq!(crate::cpp::CppPath::from("other_ns::Unit").name(), "Unit");
        assert_eq!(crate::cpp::CppPath::from("rust::Unit").name(), "Unit");

        let rust_adt = |segments: &[&str], methods: Vec<ZngurMethodDetails>| ZngurType {
            ty: adt(segments),
            lifetimes: vec![],
            layout: Some(LayoutPolicy::StackAllocated { size: 1, align: 1 }),
            trivially_relocatable: TriviallyRelocatable::No,
            wellknown_traits: vec![],
            exhaustive: true,
            methods,
            cpp_methods: vec![],
            cpp_constructors: vec![],
            cpp_fields: vec![],
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
                lifetimes: vec![],
                generics: vec![],
                receiver: ZngurMethodReceiver::Ref(Mutability::Not),
                receiver_lifetime: None,
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
            lifetimes: vec![],
            layout: Some(LayoutPolicy::StackAllocated { size: 8, align: 4 }),
            trivially_relocatable: TriviallyRelocatable::No,
            wellknown_traits: vec![],
            exhaustive: true,
            methods: vec![],
            cpp_methods: vec![],
            cpp_constructors: vec![],
            cpp_fields: vec![],
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
                    ..Default::default()
                },
                lifetimes: vec![],
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
                ..Default::default()
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
                lifetimes: vec![],
                layout: Some(LayoutPolicy::Conservative { size: 20, align: 8 }),
                trivially_relocatable: TriviallyRelocatable::No,
                wellknown_traits: vec![],
                exhaustive: true,
                methods: vec![],
                cpp_methods: vec![],
                cpp_constructors: vec![],
                cpp_fields: vec![],
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
        assert!(
            rust_code
                .contains("pub struct Handle<C: ::zngur_lib::Constness = ::zngur_lib::NonConst> {")
        );
        assert!(rust_code.contains("::core::cell::UnsafeCell<()>"));
        assert!(rust_code.contains("::core::marker::PhantomData<*mut ()>"));
        assert!(rust_code.contains("::core::marker::PhantomPinned"));
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
            ..Default::default()
        });
        let mut traits = indexmap::IndexMap::new();
        traits.insert(
            my_trait.clone(),
            ZngurTrait {
                tr: my_trait.clone(),
                methods: vec![ZngurMethod {
                    name: "run".to_owned(),
                    lifetimes: vec![],
                    generics: vec![],
                    receiver: ZngurMethodReceiver::Ref(Mutability::Not),
                    receiver_lifetime: None,
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
                    lifetimes: vec![],
                    layout: Some(LayoutPolicy::StackAllocated { size: 16, align: 8 }),
                    trivially_relocatable: TriviallyRelocatable::No,
                    wellknown_traits: vec![],
                    exhaustive: true,
                    methods: vec![],
                    cpp_methods: vec![],
                    cpp_constructors: vec![],
                    cpp_fields: vec![],
                    constructor: None,
                    variants: vec![],
                    fields: vec![],
                    cpp_heap_allocated: None,
                    cpp_ref: None,
                    cpp_stack_owned: None,
                },
                ZngurType {
                    ty: RustType::Ref(
                        Mutability::Not,
                        None,
                        Box::new(RustType::Dyn(my_trait, vec![])),
                    ),
                    lifetimes: vec![],
                    layout: Some(LayoutPolicy::StackAllocated { size: 16, align: 8 }),
                    trivially_relocatable: TriviallyRelocatable::No,
                    wellknown_traits: vec![],
                    exhaustive: true,
                    methods: vec![],
                    cpp_methods: vec![],
                    cpp_constructors: vec![],
                    cpp_fields: vec![],
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
                            lifetimes: vec![],
                            inputs: vec![],
                            output: Box::new(RustType::UNIT),
                        },
                        vec![],
                    ))),
                    lifetimes: vec![],
                    layout: Some(LayoutPolicy::StackAllocated { size: 16, align: 8 }),
                    trivially_relocatable: TriviallyRelocatable::No,
                    wellknown_traits: vec![],
                    exhaustive: true,
                    methods: vec![],
                    cpp_methods: vec![],
                    cpp_constructors: vec![],
                    cpp_fields: vec![],
                    constructor: None,
                    variants: vec![],
                    fields: vec![],
                    cpp_heap_allocated: None,
                    cpp_ref: None,
                    cpp_stack_owned: None,
                },
                ZngurType {
                    ty: adt(&["crate", "MyEnum"]),
                    lifetimes: vec![],
                    layout: Some(LayoutPolicy::StackAllocated { size: 8, align: 4 }),
                    trivially_relocatable: TriviallyRelocatable::No,
                    wellknown_traits: vec![],
                    exhaustive: true,
                    methods: vec![],
                    cpp_methods: vec![],
                    cpp_constructors: vec![],
                    cpp_fields: vec![],
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
            lifetimes: vec![],
            layout: Some(LayoutPolicy::StackAllocated { size: 24, align: 8 }),
            trivially_relocatable: TriviallyRelocatable::No,
            wellknown_traits: vec![],
            exhaustive: true,
            methods: vec![],
            cpp_methods: vec![],
            cpp_constructors: vec![],
            cpp_fields: vec![],
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
        assert!(
            rust_code
                .contains("pub struct Point<C: ::zngur_lib::Constness = ::zngur_lib::NonConst> {")
        );
        assert!(rust_code.contains(
            "unsafe impl<C: ::zngur_lib::Constness> ::zngur_lib::ZngCppStackObject for Point<C> {}"
        ));
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
        assert!(rust_code.contains(
            "unsafe impl<C: ::zngur_lib::Constness> ::zngur_lib::ZngCppStackObject for Point<C> {}"
        ));
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
        assert!(
            rust_code
                .contains("pub struct Point<C: ::zngur_lib::Constness = ::zngur_lib::NonConst> {")
        );
        assert!(
            rust_code
                .contains("pub struct Box2d<C: ::zngur_lib::Constness = ::zngur_lib::NonConst> {")
        );
        assert!(!rust_code.contains("ZngCppStackObject"));
        // ...but exact (`==`) and conservative (`<=`) layouts are still checked in C++.
        assert!(h.contains("static_assert(sizeof(::geo::Point) == 16, \"Size mismatch\");"));
        assert!(h.contains("static_assert(alignof(::geo::Point) == 8, \"Alignment mismatch\");"));
        assert!(h.contains("static_assert(sizeof(::geo::Box2d) <= 20, \"Size mismatch\");"));
        assert!(h.contains("static_assert(alignof(::geo::Box2d) <= 8, \"Alignment mismatch\");"));
    }

    #[test]
    fn imported_cpp_types_are_reexported_and_deduplicated() {
        let bar_import = ModuleImport {
            path: "./bar.zng".into(),
            crate_name: Some("bar-bindings".to_owned()),
            cpp_types: vec![
                vec!["ns".to_owned(), "Bar".to_owned()],
                vec!["Top".to_owned()],
            ],
            ..Default::default()
        };
        let spec = ZngurSpec {
            // Duplicate `ModuleImport` entries (e.g. from two `merge`d files importing the same
            // module) must emit each `pub use` only once.
            imported_modules: vec![bar_import.clone(), bar_import],
            // A local `c++::` type in the same namespace shares the module.
            types: vec![minimal_opaque_cpp_type(cpp(&["ns", "Local"]))],
            ..Default::default()
        };
        let (rust_code, h, _cpp) =
            ZngurGenerator::build_from_zng(spec, "test_crate".to_owned()).render(false);
        assert_eq!(
            rust_code
                .matches("pub use ::bar_bindings::generated::Top;")
                .count(),
            1
        );
        let ns_module = rust_code
            .split("pub mod ns {")
            .nth(1)
            .expect("no `ns` module");
        assert_eq!(
            ns_module
                .matches("pub use ::bar_bindings::generated::ns::Bar;")
                .count(),
            1
        );
        assert!(ns_module.contains("pub struct Local"));
        assert_eq!(rust_code.matches("pub mod ns {").count(), 1);
        assert_eq!(h.matches("#include \"./bar.zng.h\"").count(), 1);
    }

    #[test]
    fn cpp_type_without_layout_is_reference_only_and_has_no_layout_checks() {
        let spec = ZngurSpec {
            types: vec![minimal_opaque_cpp_type(cpp(&["geo", "Point"]))],
            ..Default::default()
        };
        let (rust_code, h, _cpp) =
            ZngurGenerator::build_from_zng(spec, "test_crate".to_owned()).render(false);
        assert!(
            rust_code
                .contains("pub struct Point<C: ::zngur_lib::Constness = ::zngur_lib::NonConst> {")
        );
        assert!(!rust_code.contains("ZngCppStackObject"));
        assert!(!h.contains("sizeof(::geo::Point)"));
        assert!(h.contains("inline ::geo::Point& cpp()"));
    }

    #[test]
    fn references_to_cpp_objects_are_cpp_references_and_options_are_pointers() {
        let b = cpp(&["a", "B"]);
        let a = Some(Lifetime("a".to_owned()));
        let r = |m, constness, l: &Option<Lifetime>| {
            RustType::Ref(
                m,
                l.clone(),
                Box::new(RustType::cpp_object(constness, b.clone())),
            )
        };
        let option = |t: RustType| {
            RustType::Adt(RustPathAndGenerics {
                path: vec!["std".to_owned(), "option".to_owned(), "Option".to_owned()],
                generics: vec![t],
                ..Default::default()
            })
        };
        let spec = ZngurSpec {
            types: vec![minimal_opaque_cpp_type(b.clone())],
            // C++ functions that Rust calls.
            cpp_funcs: vec![ZngurCppFn {
                lifetimes: vec![Lifetime("a".to_owned())],
                namespace: vec!["a".to_owned()],
                cpp_name: "f".to_owned(),
                template_args: vec![],
                rust_name: None,
                inputs: vec![option(r(Mutability::Not, CppConstness::Const, &a))],
                output: option(r(Mutability::Not, CppConstness::Var, &a)),
                is_safe: true,
            }],
            // Functions that Rust calls and C++ implements.
            extern_cpp_funcs: vec![
                ZngurExternCppFn {
                    name: "g".to_owned(),
                    lifetimes: vec![Lifetime("a".to_owned())],
                    inputs: vec![
                        r(Mutability::Not, CppConstness::Var, &a),
                        option(r(Mutability::Not, CppConstness::Const, &a)),
                    ],
                    output: r(Mutability::Not, CppConstness::Const, &a),
                    is_safe: true,
                },
                ZngurExternCppFn {
                    name: "g_opt_out".to_owned(),
                    lifetimes: vec![Lifetime("a".to_owned())],
                    inputs: vec![option(r(Mutability::Not, CppConstness::Const, &a))],
                    output: option(r(Mutability::Not, CppConstness::Const, &a)),
                    is_safe: true,
                },
            ],
            // Rust functions that C++ calls.
            funcs: vec![
                ZngurFn {
                    path: RustPathAndGenerics {
                        path: vec!["crate".to_owned(), "h".to_owned()],
                        ..Default::default()
                    },
                    lifetimes: vec![],
                    inputs: vec![
                        option(r(Mutability::Mut, CppConstness::Var, &None)),
                        r(Mutability::Mut, CppConstness::Var, &None),
                    ],
                    output: r(Mutability::Not, CppConstness::Const, &None),
                    is_safe: None,
                },
                ZngurFn {
                    path: RustPathAndGenerics {
                        path: vec!["crate".to_owned(), "h_opt_out".to_owned()],
                        ..Default::default()
                    },
                    lifetimes: vec![],
                    inputs: vec![option(r(Mutability::Mut, CppConstness::Var, &None))],
                    output: option(r(Mutability::Mut, CppConstness::Var, &None)),
                    is_safe: None,
                },
            ],
            ..Default::default()
        };
        let (rust_code, h, cpp_code) =
            ZngurGenerator::build_from_zng(spec, "test_crate".to_owned()).render(false);
        let cpp_code = cpp_code.expect("C++ functions need a C++ file");
        let b = "::rust::cpp::a::B";
        let const_b = format!("::rust::CppConst< {b} >");

        // C++ functions: Rust sees `Option<&'a ...>` (with the implicit-`const` generic), and the
        // C++ function takes and returns pointers.
        assert!(
            rust_code.contains(
                "pub fn __zngur__c___a_cpp_fn_f_l9l10s11e13e20<'a, __C1: \
                 ::zngur_lib::Constness>(i0: ::std::option::Option<&'a a::B::<__C1>>, ) -> \
                 ::std::option::Option::<&'a a::B>"
            ),
            "{rust_code}"
        );
        assert!(
            cpp_code.contains(&format!(
                "::rust::CppPtr< {b} > __zngur_o = ::a::f(::rust::__zngur_internal_move_from_rust< \
                 ::rust::CppPtr< {const_b} > >(i0));"
            )),
            "{cpp_code}"
        );
        assert!(cpp_code.contains(&format!("), ::rust::CppPtr< {b} > >::value")));

        // `extern "C++"` functions: the C++ definition takes and returns C++ references and
        // pointers; inputs are generic over constness while outputs keep fixed `Const`.
        assert!(
            rust_code.contains(
                "pub fn g<'a, __C1: ::zngur_lib::Constness>(i0: &'a a::B, i1: \
                 ::std::option::Option<&'a a::B::<__C1>>, ) -> &'a a::B::<::zngur_lib::Const>"
            ),
            "{rust_code}"
        );
        assert!(
            rust_code.contains(
                "pub fn g_opt_out<'a, __C1: ::zngur_lib::Constness>(i0: \
                 ::std::option::Option<&'a a::B::<__C1>>, ) -> \
                 ::std::option::Option::<&'a a::B::<::zngur_lib::Const>>"
            ),
            "{rust_code}"
        );
        assert!(
            h.contains(&format!(
                "::rust::CppRef< {const_b} > g(\n    ::rust::CppRef< {b} >, ::rust::CppPtr< {const_b} >"
            )),
            "{h}"
        );
        assert!(
            h.contains(&format!(
                "::rust::CppPtr< {const_b} > g_opt_out(\n    ::rust::CppPtr< {const_b} >"
            )),
            "{h}"
        );
        assert!(cpp_code.contains(&format!(
            "::rust::__zngur_internal_move_to_rust< ::rust::CppRef< {const_b} > >(o, oo);"
        )));
        assert!(cpp_code.contains(&format!(
            "::rust::__zngur_internal_move_to_rust< ::rust::CppPtr< {const_b} > >(o, oo);"
        )));

        // Rust functions called from C++: C++ passes and gets back C++ references and pointers,
        // and references cross as pointers while nullable references already are pointers.
        assert!(
            h.contains(&format!(
                "inline ::rust::CppRef< {const_b} > h(::rust::CppPtr< {b} > i0, ::rust::CppRef< {b} \
                 > i1) noexcept {{"
            )),
            "{h}"
        );
        assert!(h.contains(&format!("::rust::CppPtr< {b} > __zngur_i1 = &i1;")));
        assert!(h.contains("return *o;"));
        assert!(
            h.contains(&format!(
                "inline ::rust::CppPtr< {b} > h_opt_out(::rust::CppPtr< {b} > i0) noexcept {{"
            )),
            "{h}"
        );
        assert!(h.contains(&format!("::rust::CppPtr< {b} > o{{}};")));
        assert!(h.contains("return o;"));
    }

    #[test]
    fn cpp_functions_get_rust_functions_in_the_cpp_tree_and_cpp_bridges() {
        let i32 = RustType::Primitive(PrimitiveRustType::Int(32));
        let spec = ZngurSpec {
            cpp_funcs: vec![
                ZngurCppFn {
                    lifetimes: vec![],
                    namespace: vec!["a".to_owned(), "b".to_owned()],
                    cpp_name: "f".to_owned(),
                    template_args: vec![],
                    rust_name: Some("g".to_owned()),
                    inputs: vec![i32.clone()],
                    output: i32.clone(),
                    is_safe: true,
                },
                ZngurCppFn {
                    lifetimes: vec![],
                    namespace: vec!["a".to_owned(), "b".to_owned()],
                    cpp_name: "raw_step".to_owned(),
                    template_args: vec![],
                    rust_name: None,
                    inputs: vec![RustType::cpp_ptr(
                        Mutability::Not,
                        CppIndirection::Pointer,
                        i32.clone(),
                    )],
                    output: i32,
                    is_safe: false,
                },
            ],
            ..Default::default()
        };
        let (rust_code, _h, cpp_code) =
            ZngurGenerator::build_from_zng(spec, "test_crate".to_owned()).render(false);
        let cpp_code = cpp_code.expect("C++ functions need a C++ file");

        assert!(
            rust_code
                .contains("pub fn __zngur__c___a_b_cpp_fn_g_l9l10s11s13e15e22(i0: i32, ) -> i32"),
            "{rust_code}"
        );
        assert!(
            rust_code.contains(
                "pub unsafe fn __zngur__c___a_b_cpp_fn_raw_step_l9l10s11s13e15e22<__C1: \
                 ::zngur_lib::Constness>(i0: *const ::zngur_lib::Cpp<i32, __C1>, ) -> i32"
            ),
            "{rust_code}"
        );
        assert!(rust_code.contains("pub mod a {"));
        assert!(rust_code.contains("pub mod b {"));
        assert!(
            rust_code.contains(
                "pub use super::super::__zngur__c___a_b_cpp_fn_g_l9l10s11s13e15e22 as g;"
            ),
            "{rust_code}"
        );
        assert!(
            rust_code.contains(
                "pub use super::super::__zngur__c___a_b_cpp_fn_raw_step_l9l10s11s13e15e22 as \
                 raw_step;"
            ),
            "{rust_code}"
        );
        assert!(
            cpp_code.contains(
                "::int32_t __zngur_o = ::a::b::f(::rust::__zngur_internal_move_from_rust< \
                 ::int32_t >(i0));"
            ),
            "{cpp_code}"
        );
        assert!(
            cpp_code.contains(
                "::int32_t __zngur_o = ::a::b::raw_step(::rust::__zngur_internal_move_from_rust< \
                 ::rust::CppPtr< ::rust::CppConst< ::int32_t > > >(i0));"
            ),
            "{cpp_code}"
        );
    }

    #[test]
    fn cpp_functions_with_underscores_do_not_collide() {
        let mk = |ns: &[&str], cpp_name: &str, rust_name: Option<&str>| ZngurCppFn {
            lifetimes: vec![],
            namespace: ns.iter().map(|s| (*s).to_owned()).collect(),
            cpp_name: cpp_name.to_owned(),
            template_args: vec![],
            rust_name: rust_name.map(str::to_owned),
            inputs: vec![],
            output: RustType::UNIT,
            is_safe: true,
        };
        let b_cpp_fn = cpp(&["a", "b_cpp_fn"]);
        let dyn_fn_ty = RustType::Boxed(Box::new(RustType::Dyn(
            RustTrait::Fn {
                name: "Fn".to_owned(),
                lifetimes: vec![],
                inputs: vec![],
                output: Box::new(b_cpp_fn.clone()),
            },
            vec![],
        )));
        let spec = ZngurSpec {
            types: vec![
                minimal_by_value_cpp_type(b_cpp_fn, 8, 4),
                ZngurType {
                    ty: dyn_fn_ty,
                    lifetimes: vec![],
                    layout: Some(LayoutPolicy::StackAllocated { size: 16, align: 8 }),
                    trivially_relocatable: TriviallyRelocatable::No,
                    wellknown_traits: vec![],
                    exhaustive: true,
                    methods: vec![],
                    cpp_methods: vec![],
                    cpp_constructors: vec![],
                    cpp_fields: vec![],
                    constructor: None,
                    variants: vec![],
                    fields: vec![],
                    cpp_heap_allocated: None,
                    cpp_ref: None,
                    cpp_stack_owned: None,
                },
            ],
            cpp_funcs: vec![
                mk(&["a"], "b", None),
                mk(&["a"], "f", Some("b__c")),
                mk(&["a", "b"], "c", None),
                mk(&["a_"], "_b", None),
                mk(&["a", "_"], "b", None),
            ],
            ..Default::default()
        };
        let (rust_code, _h, cpp_code) =
            ZngurGenerator::build_from_zng(spec, "test_crate".to_owned()).render(false);
        let cpp_code = cpp_code.expect("C++ functions need a C++ file");

        let mut extern_fns = std::collections::HashSet::new();
        let mut hidden_fns = std::collections::HashSet::new();
        for line in rust_code.lines() {
            if let Some(rest) = line
                .strip_prefix("unsafe extern \"C\" { fn ")
                .or_else(|| line.strip_prefix("pub extern \"C\" fn "))
            {
                let name = rest.split('(').next().unwrap();
                assert!(extern_fns.insert(name), "duplicate extern symbol `{name}`");
            }
            if let Some(rest) = line.strip_prefix("pub fn ") {
                let name = rest.split('(').next().unwrap();
                assert!(hidden_fns.insert(name), "duplicate hidden fn `{name}`");
            }
        }
        assert_eq!(extern_fns.len(), 6);
        assert_eq!(hidden_fns.len(), 5);
        assert!(extern_fns.is_disjoint(&hidden_fns));
        assert!(cpp_code.contains("::a::b()"));
        assert!(cpp_code.contains("::a::f()"));
        assert!(cpp_code.contains("::a::b::c()"));
        assert!(cpp_code.contains("::a_::_b()"));
        assert!(cpp_code.contains("::a::_::b()"));
    }

    #[test]
    fn cpp_function_sharing_name_with_cpp_type_avoids_value_namespace_collision() {
        let local_foo = cpp(&["ns", "Foo"]);
        let imported_bar = cpp(&["ns", "Bar"]);
        let imported_baz_path = CppTypePath {
            path: vec!["ns".to_owned(), "Baz".to_owned()],
            generics: vec![RustType::Primitive(PrimitiveRustType::Int(32))],
        };
        let imported_baz = RustType::Cpp(imported_baz_path.clone());
        let spec = ZngurSpec {
            types: vec![minimal_opaque_cpp_type(local_foo.clone())],
            imported_modules: vec![ModuleImport {
                path: "./base.zng".into(),
                crate_name: Some("base".to_owned()),
                cpp_types: vec![vec!["ns".to_owned(), "Bar".to_owned()]],
                cpp_templates: vec![vec!["ns".to_owned(), "Baz".to_owned()]],
                cpp_template_instantiations: vec![imported_baz_path],
                ..Default::default()
            }],
            cpp_funcs: vec![
                ZngurCppFn {
                    lifetimes: vec![Lifetime("a".to_owned())],
                    namespace: vec!["ns".to_owned()],
                    cpp_name: "Foo".to_owned(),
                    template_args: vec![],
                    rust_name: None,
                    inputs: vec![RustType::Ref(
                        Mutability::Not,
                        Some(Lifetime("a".to_owned())),
                        Box::new(RustType::CppConst(Box::new(local_foo))),
                    )],
                    output: RustType::UNIT,
                    is_safe: true,
                },
                ZngurCppFn {
                    lifetimes: vec![Lifetime("a".to_owned())],
                    namespace: vec!["ns".to_owned()],
                    cpp_name: "Bar".to_owned(),
                    template_args: vec![],
                    rust_name: None,
                    inputs: vec![RustType::Ref(
                        Mutability::Not,
                        Some(Lifetime("a".to_owned())),
                        Box::new(RustType::CppConst(Box::new(imported_bar))),
                    )],
                    output: RustType::UNIT,
                    is_safe: true,
                },
                ZngurCppFn {
                    lifetimes: vec![Lifetime("a".to_owned())],
                    namespace: vec!["ns".to_owned()],
                    cpp_name: "Baz".to_owned(),
                    template_args: vec![],
                    rust_name: None,
                    inputs: vec![RustType::Ref(
                        Mutability::Not,
                        Some(Lifetime("a".to_owned())),
                        Box::new(RustType::CppConst(Box::new(imported_baz))),
                    )],
                    output: RustType::UNIT,
                    is_safe: true,
                },
            ],
            ..Default::default()
        };
        let (rust_code, _h, _cpp_code) =
            ZngurGenerator::build_from_zng(spec, "test_crate".to_owned()).render(false);
        assert!(
            rust_code
                .contains("pub struct Foo<C: ::zngur_lib::Constness = ::zngur_lib::NonConst> {"),
            "{rust_code}"
        );
        assert!(
            rust_code.contains(
                "pub type Bar<C = ::zngur_lib::NonConst> = ::base::generated::ns::Bar<C>;"
            ),
            "{rust_code}"
        );
        assert!(!rust_code.contains("pub use ::base::generated::ns::Bar;"));
        assert!(
            rust_code.contains(
                "pub type Baz<T0, C = ::zngur_lib::NonConst> = \
                 ::base::generated::ns::Baz<T0, C>;"
            ),
            "{rust_code}"
        );
        assert!(!rust_code.contains("pub use ::base::generated::ns::Baz;"));
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
                ..Default::default()
            }],
            cpp_funcs: vec![ZngurCppFn {
                lifetimes: vec![],
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
                "::new (static_cast<void*>(o)) \
                 ::geo::Point(::shapes::Mirror(::rust::__zngur_internal_move_from_rust< \
                 ::rust::cpp::geo::Point >(i0).cpp()));"
            ),
            "{cpp_code}"
        );
    }

    #[test]
    fn extern_cpp_impl_const_inputs_are_generic_for_inherent_and_fixed_for_trait() {
        let b = cpp(&["a", "B"]);
        let lt_a = Lifetime("a".to_owned());
        let const_b_ref = RustType::Ref(
            Mutability::Not,
            Some(lt_a.clone()),
            Box::new(RustType::cpp_object(CppConstness::Const, b.clone())),
        );
        let method = |name: &str| ZngurMethod {
            name: name.to_owned(),
            lifetimes: vec![lt_a.clone()],
            receiver: ZngurMethodReceiver::Ref(Mutability::Not),
            receiver_lifetime: None,
            generics: vec![],
            inputs: vec![const_b_ref.clone()],
            output: RustType::UNIT,
            is_safe: Some(true),
        };
        let spec = ZngurSpec {
            types: vec![minimal_opaque_cpp_type(b.clone())],
            extern_cpp_impls: vec![
                ZngurExternCppImpl {
                    tr: None,
                    ty: b.clone(),
                    lifetimes: vec![],
                    methods: vec![method("inherent_cmp")],
                },
                ZngurExternCppImpl {
                    tr: Some(RustTrait::Normal(RustPathAndGenerics {
                        path: vec!["crate".to_owned(), "MyTrait".to_owned()],
                        ..Default::default()
                    })),
                    ty: b,
                    lifetimes: vec![],
                    methods: vec![method("trait_cmp")],
                },
            ],
            ..Default::default()
        };
        let (rust_code, _, _) =
            ZngurGenerator::build_from_zng(spec, "test_crate".to_owned()).render(false);
        assert!(
            rust_code.contains(
                "impl a::B {pub fn inherent_cmp<'a, __C1: ::zngur_lib::Constness>(&self, i1: &'a a::B::<__C1>, ) -> ()"
            ),
            "{rust_code}"
        );
        assert!(
            rust_code.contains(
                "impl crate::MyTrait for a::B {fn trait_cmp<'a>(&self, i1: &'a a::B::<::zngur_lib::Const>, ) -> ()"
            ),
            "{rust_code}"
        );
    }

    #[test]
    fn lifetimes_are_rendered_in_rust_glue_and_erased_in_cpp() {
        let lt_a = Lifetime("a".to_owned());
        let lt_b = Lifetime("b".to_owned());
        let str_ty = RustType::Primitive(PrimitiveRustType::Str);
        let ref_a_str = RustType::Ref(
            Mutability::Not,
            Some(lt_a.clone()),
            Box::new(str_ty.clone()),
        );
        let ref_b_str = RustType::Ref(
            Mutability::Not,
            Some(lt_b.clone()),
            Box::new(str_ty.clone()),
        );
        let parser_ty = RustType::Adt(RustPathAndGenerics {
            path: vec!["crate".to_owned(), "Parser".to_owned()],
            lifetimes: vec![lt_a.clone()],
            ..Default::default()
        });
        let pair_ty = RustType::Adt(RustPathAndGenerics {
            path: vec!["crate".to_owned(), "Pair".to_owned()],
            lifetimes: vec![lt_a.clone(), lt_b.clone()],
            ..Default::default()
        });
        let chooser_tr = RustTrait::Normal(RustPathAndGenerics {
            path: vec!["crate".to_owned(), "Chooser".to_owned()],
            ..Default::default()
        });
        let higher_ranked_fn_tr = RustTrait::Fn {
            name: "Fn".to_owned(),
            lifetimes: vec![lt_a.clone()],
            inputs: vec![ref_a_str.clone()],
            output: Box::new(ref_a_str.clone()),
        };
        let ref_anon_i32 = RustType::Ref(
            Mutability::Not,
            Some(Lifetime("_".to_owned())),
            Box::new(RustType::Primitive(PrimitiveRustType::Int(32))),
        );
        let anon_fn_tr = RustTrait::Fn {
            name: "Fn".to_owned(),
            lifetimes: vec![],
            inputs: vec![ref_anon_i32.clone()],
            output: Box::new(ref_anon_i32),
        };
        let value_fn_tr = RustTrait::Fn {
            name: "Fn".to_owned(),
            lifetimes: vec![],
            inputs: vec![RustType::Primitive(PrimitiveRustType::Int(32))],
            output: Box::new(RustType::Primitive(PrimitiveRustType::Int(32))),
        };
        let borrowed_future_ty = RustType::Boxed(Box::new(RustType::Dyn(
            RustTrait::Normal(RustPathAndGenerics {
                path: vec!["std".to_owned(), "future".to_owned(), "Future".to_owned()],
                named_generics: vec![("Output".to_owned(), ref_a_str.clone())],
                ..Default::default()
            }),
            vec![],
        )));

        let mut traits = indexmap::IndexMap::new();
        traits.insert(
            chooser_tr.clone(),
            ZngurTrait {
                tr: chooser_tr.clone(),
                methods: vec![ZngurMethod {
                    name: "choose".to_owned(),
                    lifetimes: vec![lt_a.clone()],
                    generics: vec![],
                    receiver: ZngurMethodReceiver::Ref(Mutability::Not),
                    receiver_lifetime: Some(lt_a.clone()),
                    inputs: vec![ref_a_str.clone()],
                    output: ref_a_str.clone(),
                    is_safe: Some(true),
                }],
            },
        );

        let boxed_type = |tr: RustTrait| ZngurType {
            ty: RustType::Boxed(Box::new(RustType::Dyn(tr, vec![]))),
            lifetimes: vec![],
            layout: Some(LayoutPolicy::StackAllocated { size: 16, align: 8 }),
            trivially_relocatable: TriviallyRelocatable::No,
            wellknown_traits: vec![ZngurWellknownTrait::Drop],
            exhaustive: true,
            methods: vec![],
            cpp_methods: vec![],
            cpp_constructors: vec![],
            cpp_fields: vec![],
            constructor: None,
            variants: vec![],
            fields: vec![],
            cpp_heap_allocated: None,
            cpp_ref: None,
            cpp_stack_owned: None,
        };

        let spec = ZngurSpec {
            types: vec![
                ZngurType {
                    ty: str_ty,
                    lifetimes: vec![],
                    layout: Some(LayoutPolicy::OnlyByRef),
                    trivially_relocatable: TriviallyRelocatable::No,
                    wellknown_traits: vec![ZngurWellknownTrait::Unsized],
                    exhaustive: true,
                    methods: vec![],
                    cpp_methods: vec![],
                    cpp_constructors: vec![],
                    cpp_fields: vec![],
                    constructor: None,
                    variants: vec![],
                    fields: vec![],
                    cpp_heap_allocated: None,
                    cpp_ref: None,
                    cpp_stack_owned: None,
                },
                ZngurType {
                    ty: pair_ty,
                    lifetimes: vec![lt_a.clone(), lt_b.clone()],
                    layout: Some(LayoutPolicy::StackAllocated { size: 32, align: 8 }),
                    trivially_relocatable: TriviallyRelocatable::No,
                    wellknown_traits: vec![ZngurWellknownTrait::Drop],
                    exhaustive: true,
                    methods: vec![],
                    cpp_methods: vec![],
                    cpp_constructors: vec![],
                    cpp_fields: vec![],
                    constructor: Some(ZngurConstructor {
                        inputs: vec![
                            ("first".to_owned(), ref_a_str.clone()),
                            ("second".to_owned(), ref_b_str.clone()),
                        ],
                    }),
                    variants: vec![],
                    fields: vec![
                        ZngurField {
                            name: "first".to_owned(),
                            ty: ref_a_str.clone(),
                            offset: Some(0),
                        },
                        ZngurField {
                            name: "second".to_owned(),
                            ty: ref_b_str,
                            offset: Some(16),
                        },
                    ],
                    cpp_heap_allocated: None,
                    cpp_ref: None,
                    cpp_stack_owned: None,
                },
                boxed_type(chooser_tr),
                boxed_type(higher_ranked_fn_tr),
                boxed_type(anon_fn_tr),
                boxed_type(value_fn_tr),
                ZngurType {
                    ty: borrowed_future_ty,
                    lifetimes: vec![lt_a.clone()],
                    layout: Some(LayoutPolicy::StackAllocated { size: 16, align: 8 }),
                    trivially_relocatable: TriviallyRelocatable::No,
                    wellknown_traits: vec![ZngurWellknownTrait::Drop],
                    exhaustive: true,
                    methods: vec![],
                    cpp_methods: vec![],
                    cpp_constructors: vec![],
                    cpp_fields: vec![],
                    constructor: None,
                    variants: vec![],
                    fields: vec![],
                    cpp_heap_allocated: None,
                    cpp_ref: None,
                    cpp_stack_owned: None,
                },
            ],
            traits,
            funcs: vec![ZngurFn {
                path: RustPathAndGenerics {
                    path: vec!["crate".to_owned(), "identity".to_owned()],
                    generics: vec![ref_a_str.clone()],
                    ..Default::default()
                },
                lifetimes: vec![lt_a.clone()],
                inputs: vec![ref_a_str.clone()],
                output: ref_a_str.clone(),
                is_safe: None,
            }],
            extern_cpp_funcs: vec![ZngurExternCppFn {
                name: "shortest".to_owned(),
                lifetimes: vec![lt_a.clone()],
                inputs: vec![ref_a_str.clone(), ref_a_str.clone()],
                output: ref_a_str.clone(),
                is_safe: true,
            }],
            extern_cpp_impls: vec![
                ZngurExternCppImpl {
                    lifetimes: vec![lt_a.clone()],
                    tr: None,
                    ty: parser_ty.clone(),
                    methods: vec![ZngurMethod {
                        name: "skip_and_rest".to_owned(),
                        lifetimes: vec![],
                        generics: vec![],
                        receiver: ZngurMethodReceiver::Ref(Mutability::Mut),
                        receiver_lifetime: Some(lt_a.clone()),
                        inputs: vec![],
                        output: ref_a_str.clone(),
                        is_safe: Some(true),
                    }],
                },
                ZngurExternCppImpl {
                    lifetimes: vec![lt_a.clone()],
                    tr: Some(RustTrait::Normal(RustPathAndGenerics {
                        path: vec!["crate".to_owned(), "MyTrait".to_owned()],
                        lifetimes: vec![lt_a.clone()],
                        ..Default::default()
                    })),
                    ty: parser_ty,
                    methods: vec![ZngurMethod {
                        name: "inspect".to_owned(),
                        lifetimes: vec![lt_b.clone()],
                        generics: vec![],
                        receiver: ZngurMethodReceiver::Ref(Mutability::Not),
                        receiver_lifetime: Some(lt_b),
                        inputs: vec![],
                        output: ref_a_str,
                        is_safe: Some(true),
                    }],
                },
            ],
            ..Default::default()
        };

        let (rust_code, h, _cpp) =
            ZngurGenerator::build_from_zng(spec, "crate".to_owned()).render(false);

        // `extern "C++"` functions and `impl` blocks preserve lifetimes in Rust, erase in C++.
        assert!(
            rust_code.contains("pub fn shortest<'a>(i0: &'a str, i1: &'a str, ) -> &'a str"),
            "{rust_code}"
        );
        assert!(
            rust_code.contains(
                "impl<'a> crate::Parser::<'a> {pub fn skip_and_rest(&'a mut self, ) -> &'a str"
            ),
            "{rust_code}"
        );
        assert!(
            rust_code.contains(
                "impl<'a> crate::MyTrait::<'a> for crate::Parser::<'a> {fn inspect<'b>(&'b self, ) -> &'a str"
            ),
            "{rust_code}"
        );
        assert!(
            h.contains("class Impl< ::rust::crate::Parser, ::rust::crate::MyTrait >"),
            "{h}"
        );
        assert!(
            h.contains(
                "::rust::Ref< ::rust::Str > shortest(\n    ::rust::Ref< ::rust::Str >, ::rust::Ref< ::rust::Str >\n);"
            ),
            "{h}"
        );

        // Trait wrapper methods emit `<'a>` and `&'a self`.
        assert!(
            rust_code.contains("fn choose<'a>(&'a self, i0: &'a str) -> &'a str"),
            "{rust_code}"
        );

        // Both higher-ranked (`for<'a> Fn`) and elided/`'_` (`Fn(&'_ i32) -> &'_ i32`) emit
        // `__zngur_coerce`, while `Fn(i32) -> i32` uses a typed closure parameter without it.
        assert!(
            rust_code
                .contains("fn __zngur_coerce<F: for<'a> Fn(&'a str) -> &'a str>(f: F) -> F { f }"),
            "{rust_code}"
        );
        assert!(
            rust_code.contains("fn __zngur_coerce<F: Fn(&'_ i32) -> &'_ i32>(f: F) -> F { f }"),
            "{rust_code}"
        );
        assert!(
            rust_code.contains("Box::new(move |i0: i32| unsafe {"),
            "{rust_code}"
        );

        // Free function generic type arguments anonymize `'a` to `'_`.
        assert!(
            rust_code.contains("crate::identity::<&'_ str>("),
            "{rust_code}"
        );
        assert!(
            !rust_code.contains("crate::identity::<&'a str>"),
            "{rust_code}"
        );

        // Multi-lifetime field assertions keep un-anonymized `'a, 'b` on `__zngur_assert_has_field!`
        // while using `'_` for layout and offset assertions.
        assert!(
            rust_code.contains(
                "__zngur_assert_has_field!(['a, 'b], crate::Pair::<'a, 'b>, &'a str, first);"
            ),
            "{rust_code}"
        );
        assert!(
            rust_code.contains(
                "__zngur_assert_has_field!(['a, 'b], crate::Pair::<'a, 'b>, &'b str, second);"
            ),
            "{rust_code}"
        );
        assert!(
            rust_code.contains("__zngur_assert_field_offset!(crate::Pair::<'_, '_>, 0, first);"),
            "{rust_code}"
        );
        assert!(
            rust_code.contains(
                "crate::Pair::<'_, '_> { first: ::std::ptr::read(f_first as *mut &'_ str), second: ::std::ptr::read(f_second as *mut &'_ str)"
            ),
            "{rust_code}"
        );

        // Borrowed `Future<Output = &'a str>` skips C++20 coroutine shim generation.
        assert!(!rust_code.contains("struct CoroState"), "{rust_code}");
        assert!(!h.contains("ZNGUR_CORO_SHIMS_"), "{h}");
    }

    #[test]
    fn cpp_indirection_reference_emits_cpp_ref_in_cpp_and_raw_ptr_in_rust() {
        let b = cpp(&["a", "B"]);
        let const_b_ref = RustType::cpp_ptr(Mutability::Not, CppIndirection::Reference, b.clone());
        let i32_ref = RustType::cpp_ptr(
            Mutability::Mut,
            CppIndirection::Reference,
            RustType::Primitive(PrimitiveRustType::Int(32)),
        );
        let spec = ZngurSpec {
            types: vec![minimal_opaque_cpp_type(b)],
            extern_cpp_funcs: vec![ZngurExternCppFn {
                name: "take_and_return_cpp_refs".to_owned(),
                lifetimes: vec![],
                inputs: vec![const_b_ref.clone(), i32_ref.clone()],
                output: const_b_ref.clone(),
                is_safe: false,
            }],
            funcs: vec![ZngurFn {
                path: RustPathAndGenerics {
                    path: vec!["crate".to_owned(), "rust_takes_cpp_refs".to_owned()],
                    ..Default::default()
                },
                lifetimes: vec![],
                inputs: vec![const_b_ref, i32_ref.clone()],
                output: i32_ref,
                is_safe: None,
            }],
            ..Default::default()
        };
        let (rust_code, h, cpp_code) =
            ZngurGenerator::build_from_zng(spec, "test_crate".to_owned()).render(false);
        let cpp_code = cpp_code.expect("extern C++ functions need a C++ file");
        let const_b = "::rust::CppConst< ::rust::cpp::a::B >";

        assert!(
            rust_code.contains(
                "pub unsafe fn take_and_return_cpp_refs<__C1: ::zngur_lib::Constness>(i0: *const a::B::<__C1>, i1: *mut ::zngur_lib::Cpp<i32>, ) -> *const a::B::<::zngur_lib::Const>"
            ),
            "{rust_code}"
        );
        assert!(
            h.contains(&format!(
                "::rust::CppRef< {const_b} > take_and_return_cpp_refs(\n    ::rust::CppRef< {const_b} >, ::rust::CppRef< ::int32_t >"
            )),
            "{h}"
        );
        assert!(
            cpp_code.contains(&format!(
                "::rust::__zngur_internal_move_to_rust< ::rust::CppRef< {const_b} > >(o, oo);"
            )),
            "{cpp_code}"
        );
        assert!(
            h.contains(&format!(
                "inline ::rust::CppRef< ::int32_t > rust_takes_cpp_refs(::rust::CppRef< {const_b} > i0, ::rust::CppRef< ::int32_t > i1) noexcept {{"
            )),
            "{h}"
        );
    }

    #[test]
    fn imported_cpp_heap_allocated_type_uses_rust_value_ref() {
        let heap_foo = cpp(&["ns", "HeapFoo"]);
        let spec = ZngurSpec {
            imported_modules: vec![ModuleImport {
                path: "./bar.zng".into(),
                crate_name: Some("bar_bindings".to_owned()),
                cpp_types: vec![vec!["ns".to_owned(), "HeapFoo".to_owned()]],
                cpp_heap_allocated_types: vec![vec!["ns".to_owned(), "HeapFoo".to_owned()]],
                ..Default::default()
            }],
            extern_cpp_impls: vec![ZngurExternCppImpl {
                lifetimes: vec![],
                tr: None,
                ty: heap_foo,
                methods: vec![ZngurMethod {
                    name: "inspect".to_owned(),
                    lifetimes: vec![],
                    generics: vec![],
                    receiver: ZngurMethodReceiver::Ref(Mutability::Not),
                    receiver_lifetime: None,
                    inputs: vec![],
                    output: RustType::UNIT,
                    is_safe: Some(true),
                }],
            }],
            ..Default::default()
        };
        let (rust_code, h, _cpp) =
            ZngurGenerator::build_from_zng(spec, "test_crate".to_owned()).render(false);
        assert!(
            rust_code.contains("impl ns::HeapFoo {pub fn inspect(&self, ) -> ()"),
            "{rust_code}"
        );
        assert!(
            h.contains("static ::rust::Unit inspect(\n        ::rust::Ref< ::rust::cpp::ns::HeapFoo >\n    );"),
            "{h}"
        );
        assert!(!h.contains("CppRef< ::rust::cpp::ns::HeapFoo >"), "{h}");
    }

    #[test]
    fn rust_value_references_to_cpp_pointers_emit_ref_to_ptr() {
        let b = cpp(&["a", "B"]);
        let lt_a = Some(Lifetime("a".to_owned()));
        let ref_i32_ptr = RustType::Ref(
            Mutability::Not,
            lt_a.clone(),
            Box::new(RustType::cpp_ptr(
                Mutability::Mut,
                CppIndirection::Pointer,
                RustType::Primitive(PrimitiveRustType::Int(32)),
            )),
        );
        let ref_mut_b_ptr = RustType::Ref(
            Mutability::Mut,
            lt_a,
            Box::new(RustType::cpp_ptr(
                Mutability::Mut,
                CppIndirection::Pointer,
                b.clone(),
            )),
        );
        let spec = ZngurSpec {
            types: vec![minimal_opaque_cpp_type(b)],
            extern_cpp_funcs: vec![ZngurExternCppFn {
                name: "copy_ptr".to_owned(),
                lifetimes: vec![Lifetime("a".to_owned())],
                inputs: vec![ref_i32_ptr.clone(), ref_mut_b_ptr],
                output: ref_i32_ptr,
                is_safe: false,
            }],
            ..Default::default()
        };
        let (_rust_code, h, _cpp) =
            ZngurGenerator::build_from_zng(spec, "test_crate".to_owned()).render(true);
        assert!(h.contains("struct Ref< T* >"), "{h}");
        assert!(h.contains("struct RefMut< T* >"), "{h}");
        assert!(
            h.contains(
                "::rust::Ref< ::rust::CppPtr< ::int32_t > > copy_ptr(\n    ::rust::Ref< ::rust::CppPtr< ::int32_t > >, ::rust::RefMut< ::rust::CppPtr< ::rust::cpp::a::B > >"
            ),
            "{h}"
        );
    }

    #[test]
    fn nested_cpp_namespace_supports_cpp_refs_without_struct_cpp_ref_forward_decl() {
        let b = cpp(&["a", "B"]);
        let lt_a = Some(Lifetime("a".to_owned()));
        let const_b_ref = RustType::Ref(
            Mutability::Not,
            lt_a,
            Box::new(RustType::CppConst(Box::new(b.clone()))),
        );
        let spec = ZngurSpec {
            cpp_namespace: Some("foo::bar".to_owned()),
            mangling_base: "foo::bar".to_owned(),
            types: vec![minimal_opaque_cpp_type(b)],
            funcs: vec![ZngurFn {
                path: RustPathAndGenerics {
                    path: vec!["crate".to_owned(), "roundtrip_ref".to_owned()],
                    ..Default::default()
                },
                lifetimes: vec![Lifetime("a".to_owned())],
                inputs: vec![const_b_ref.clone()],
                output: const_b_ref.clone(),
                is_safe: None,
            }],
            extern_cpp_funcs: vec![ZngurExternCppFn {
                name: "cpp_takes_ref".to_owned(),
                lifetimes: vec![Lifetime("a".to_owned())],
                inputs: vec![const_b_ref.clone()],
                output: const_b_ref,
                is_safe: true,
            }],
            extern_cpp_impls: vec![ZngurExternCppImpl {
                lifetimes: vec![],
                tr: None,
                ty: RustType::Boxed(Box::new(RustType::cpp_ptr(
                    Mutability::Not,
                    CppIndirection::Pointer,
                    cpp(&["a", "B"]),
                ))),
                methods: vec![],
            }],
            ..Default::default()
        };
        let (_rust_code, h, cpp_code) =
            ZngurGenerator::build_from_zng(spec, "test_crate".to_owned()).render(false);
        let cpp_code = cpp_code.expect("extern C++ functions need a C++ file");

        assert!(!h.contains("struct CppRef;"), "{h}");
        assert!(!h.contains("struct CppConst;"), "{h}");
        assert!(!h.contains("struct CppPtr;"), "{h}");
        assert!(
            h.contains("::foo::bar::CppPtr< ::foo::bar::CppConst< ::foo::bar::cpp::a::B > > o{};"),
            "{h}"
        );
        assert!(h.contains("return *o;"), "{h}");
        assert!(
            cpp_code.contains("::foo::bar::exported_functions::cpp_takes_ref"),
            "{cpp_code}"
        );
    }

    #[test]
    fn user_type_named_cpp_ref_is_not_treated_as_zngur_cpp_ref_alias() {
        let rust_cpp_ref_ty = adt(&["crate", "CppRef"]);
        let cpp_cpp_ref_ty = cpp(&["CppRef"]);
        let mut rust_cpp_ref = minimal_opaque_cpp_type(rust_cpp_ref_ty.clone());
        rust_cpp_ref.layout = Some(LayoutPolicy::StackAllocated { size: 8, align: 4 });
        rust_cpp_ref.constructor = Some(ZngurConstructor {
            inputs: vec![("0".to_owned(), cpp_cpp_ref_ty.clone())],
        });
        rust_cpp_ref.methods = vec![ZngurMethodDetails {
            data: ZngurMethod {
                name: "take".to_owned(),
                lifetimes: vec![],
                generics: vec![],
                receiver: ZngurMethodReceiver::Ref(Mutability::Not),
                receiver_lifetime: None,
                inputs: vec![rust_cpp_ref_ty.clone()],
                output: rust_cpp_ref_ty.clone(),
                is_safe: None,
            },
            use_path: None,
            deref: None,
            cpp_name: None,
        }];
        let cpp_cpp_ref = minimal_by_value_cpp_type(cpp_cpp_ref_ty.clone(), 8, 4);
        let spec = ZngurSpec {
            types: vec![rust_cpp_ref, cpp_cpp_ref],
            funcs: vec![ZngurFn {
                path: RustPathAndGenerics {
                    path: vec!["crate".to_owned(), "roundtrip_cpp_ref".to_owned()],
                    ..Default::default()
                },
                lifetimes: vec![],
                inputs: vec![rust_cpp_ref_ty, cpp_cpp_ref_ty.clone()],
                output: cpp_cpp_ref_ty,
                is_safe: None,
            }],
            ..Default::default()
        };
        let (_rust_code, h, _cpp_code) =
            ZngurGenerator::build_from_zng(spec, "test_crate".to_owned()).render(false);

        assert!(!h.contains("CppPtr"), "{h}");
        assert!(
            h.contains("::rust::cpp::CppRef o{};"),
            "expected by-value storage for c++::CppRef, got:\n{h}"
        );
        assert!(
            h.contains("::rust::test_crate::CppRef o{};"),
            "expected by-value storage for crate::CppRef, got:\n{h}"
        );
        assert!(
            h.contains("::rust::__zngur_internal_assume_deinit(i0);"),
            "{h}"
        );
        assert!(
            h.contains("::rust::__zngur_internal_assume_deinit(i1);"),
            "{h}"
        );
        assert!(
            h.contains("::rust::__zngur_internal_assume_init(o);"),
            "{h}"
        );
        assert!(h.contains("::std::move(i0)"), "{h}");
        assert!(!h.contains("return *o;"), "{h}");
    }

    #[test]
    fn cpp_fields_get_accessors_and_cpp_bridges() {
        let b = cpp(&["a", "B"]);
        let i32 = RustType::Primitive(PrimitiveRustType::Int(32));
        let bool_ty = RustType::Primitive(PrimitiveRustType::Bool);
        let const_bool_ptr =
            RustType::cpp_ptr(Mutability::Not, CppIndirection::Pointer, bool_ty.clone());
        let b_ptr = RustType::cpp_ptr(Mutability::Mut, CppIndirection::Pointer, b.clone());
        let mut ty = minimal_by_value_cpp_type(b.clone(), 8, 4);
        let field = |rust_name: &str, ty: &RustType, access, is_safe| ZngurCppField {
            name: "x".to_owned(),
            rust_name: Some(rust_name.to_owned()),
            ty: ty.clone(),
            access,
            is_safe,
        };
        ty.cpp_fields = vec![
            field(
                "x_mut",
                &i32,
                CppFieldAccess::Ref {
                    unique: true,
                    constness: CppConstness::Var,
                },
                true,
            ),
            field(
                "x_unique_const",
                &i32,
                CppFieldAccess::Ref {
                    unique: true,
                    constness: CppConstness::Const,
                },
                true,
            ),
            field(
                "x_var",
                &i32,
                CppFieldAccess::Ref {
                    unique: false,
                    constness: CppConstness::Var,
                },
                true,
            ),
            field(
                "x_copy",
                &i32,
                CppFieldAccess::Copy {
                    constness: CppConstness::Var,
                },
                true,
            ),
            field(
                "b_copy",
                &b,
                CppFieldAccess::Copy {
                    constness: CppConstness::Const,
                },
                true,
            ),
            field(
                "flag_ref",
                &bool_ty,
                CppFieldAccess::Ref {
                    unique: false,
                    constness: CppConstness::Const,
                },
                true,
            ),
            field(
                "flag_ptr",
                &const_bool_ptr,
                CppFieldAccess::Ref {
                    unique: false,
                    constness: CppConstness::Const,
                },
                true,
            ),
            field(
                "next_var",
                &b_ptr,
                CppFieldAccess::Ref {
                    unique: false,
                    constness: CppConstness::Var,
                },
                false,
            ),
        ];
        let spec = ZngurSpec {
            types: vec![ty],
            ..Default::default()
        };
        let (rust_code, _h, cpp_code) =
            ZngurGenerator::build_from_zng(spec, "test_crate".to_owned()).render(false);
        let cpp_code = cpp_code.expect("C++ field accessors need a C++ file");

        assert!(
            rust_code.contains(
                "impl a::B {\n#[allow(non_snake_case, clippy::too_many_arguments)]\npub fn \
                 x_mut(&mut self, ) -> &mut ::zngur_lib::Cpp<i32>"
            ),
            "{rust_code}"
        );
        assert!(
            rust_code.contains(
                "impl a::B {\n#[allow(non_snake_case, clippy::too_many_arguments)]\npub fn \
                 x_unique_const(&mut self, ) -> &mut ::zngur_lib::Cpp<i32, ::zngur_lib::Const>"
            ),
            "{rust_code}"
        );
        assert!(
            rust_code.contains(
                "impl a::B {\n#[allow(non_snake_case, clippy::too_many_arguments)]\npub fn \
                 x_var(&self, ) -> &::zngur_lib::Cpp<i32>"
            ),
            "{rust_code}"
        );
        assert!(
            rust_code.contains(
                "impl<__C0: ::zngur_lib::Constness> a::B<__C0> {\n#[allow(non_snake_case, \
                 clippy::too_many_arguments)]\npub fn x_copy(&self, ) -> i32"
            ),
            "{rust_code}"
        );
        assert!(
            rust_code.contains("pub fn b_copy(&self, ) -> a::B::<::zngur_lib::Const>"),
            "{rust_code}"
        );
        assert!(
            rust_code.contains(
                "pub fn flag_ref(&self, ) -> &::zngur_lib::Cpp<bool, ::zngur_lib::Const>"
            ),
            "{rust_code}"
        );
        assert!(
            rust_code.contains(
                "pub fn flag_ptr(&self, ) -> &::zngur_lib::Cpp<*const ::zngur_lib::Cpp<bool, \
                 ::zngur_lib::Const>, ::zngur_lib::Const>"
            ),
            "{rust_code}"
        );
        assert!(
            rust_code.contains(
                "impl a::B {\n#[allow(non_snake_case, clippy::too_many_arguments)]\npub unsafe fn \
                 next_var(&self, ) -> &::zngur_lib::Cpp<*mut a::B>"
            ),
            "{rust_code}"
        );
        assert!(
            cpp_code.contains(
                "static_assert(!::std::is_union< ::a::B >::value, \
                 \"field accessors on C++ unions aren't supported\");"
            ),
            "{cpp_code}"
        );
        assert!(
            cpp_code.contains(
                "static_assert(::rust::__zngur_non_union_member_ptr< decltype(&::a::B::x) >::value,"
            ),
            "{cpp_code}"
        );
        assert!(
            cpp_code.contains(
                "static_assert(::std::is_same< ::std::remove_const< decltype(::a::B::x) >::type, "
            ),
            "{cpp_code}"
        );
        assert!(
            cpp_code.contains(
                "static_assert(::rust::__zngur_same_repr< ::std::remove_const< \
                 decltype(::a::B::x) >::type, "
            ),
            "{cpp_code}"
        );
        assert!(
            cpp_code.contains("static_assert(!::std::is_const< decltype(::a::B::x) >::value,"),
            "{cpp_code}"
        );
        assert!(
            cpp_code.contains("::rust::CppTypeOf< ::rust::Bool >::type"),
            "{cpp_code}"
        );
        assert!(
            cpp_code.contains(
                "::rust::CppTypeOf< ::rust::CppPtr< ::rust::CppConst< ::rust::Bool > > >::type"
            ),
            "{cpp_code}"
        );
        assert!(
            cpp_code.contains(
                "::rust::CppPtr< ::int32_t > __zngur_o = \
                 ::std::addressof((::rust::__zngur_internal_move_from_rust< ::rust::CppPtr< \
                 ::rust::cpp::a::B > >(i0))->x);"
            ),
            "{cpp_code}"
        );
        // Copies are constructed in place, through a `const` `this`.
        assert!(
            cpp_code.contains(
                "::new (static_cast<void*>(o)) ::rust::CppTypeOf< ::int32_t \
                 >::type(static_cast< ::rust::CppTypeOf< ::int32_t >::type \
                 >((::rust::__zngur_internal_move_from_rust< ::rust::CppPtr< \
                 ::rust::CppConst< ::rust::cpp::a::B > > >(i0))->x));"
            ),
            "{cpp_code}"
        );
        assert!(
            cpp_code.contains(
                "::new (static_cast<void*>(o)) ::a::B((::rust::__zngur_internal_move_from_rust< \
                 ::rust::CppPtr< ::rust::CppConst< ::rust::cpp::a::B > > >(i0))->x);"
            ),
            "{cpp_code}"
        );
    }

    #[test]
    fn cpp_methods_get_rust_functions_and_cpp_bridges() {
        let b = cpp(&["a", "B"]);
        let i32 = RustType::Primitive(PrimitiveRustType::Int(32));
        let mut ty = minimal_by_value_cpp_type(b.clone(), 8, 4);
        ty.cpp_methods = vec![
            ZngurCppMethod {
                cpp_name: "value".to_owned(),
                lifetimes: vec![],
                template_args: vec![],
                rust_name: None,
                receiver: ZngurMethodReceiver::Ref(Mutability::Not),
                receiver_constness: CppConstness::Const,
                receiver_lifetime: None,
                raw_receiver: None,
                inputs: vec![],
                output: i32.clone(),
                is_safe: false,
            },
            ZngurCppMethod {
                cpp_name: "set".to_owned(),
                lifetimes: vec![],
                template_args: vec![],
                rust_name: Some("set_from".to_owned()),
                receiver: ZngurMethodReceiver::Ref(Mutability::Mut),
                receiver_constness: CppConstness::Var,
                receiver_lifetime: None,
                raw_receiver: None,
                inputs: vec![RustType::cpp_ptr(
                    Mutability::Not,
                    CppIndirection::Reference,
                    b.clone(),
                )],
                output: RustType::UNIT,
                is_safe: false,
            },
            // A non-`const` member function called through a shared reference.
            ZngurCppMethod {
                cpp_name: "bump".to_owned(),
                lifetimes: vec![],
                template_args: vec![],
                rust_name: None,
                receiver: ZngurMethodReceiver::Ref(Mutability::Not),
                receiver_constness: CppConstness::Var,
                receiver_lifetime: None,
                raw_receiver: None,
                inputs: vec![],
                output: RustType::UNIT,
                is_safe: true,
            },
            // A `const` member function called through a unique reference.
            ZngurCppMethod {
                cpp_name: "peek".to_owned(),
                lifetimes: vec![],
                template_args: vec![],
                rust_name: None,
                receiver: ZngurMethodReceiver::Ref(Mutability::Mut),
                receiver_constness: CppConstness::Const,
                receiver_lifetime: None,
                raw_receiver: None,
                inputs: vec![],
                output: i32.clone(),
                is_safe: true,
            },
            // Raw receivers (`const self*`, `self&`) become associated functions taking
            // `*const Self` (on `impl<C> B<C>`) or `*mut Self` (on `impl B`).
            ZngurCppMethod {
                cpp_name: "raw_value".to_owned(),
                lifetimes: vec![],
                template_args: vec![],
                rust_name: None,
                receiver: ZngurMethodReceiver::Ref(Mutability::Not),
                receiver_constness: CppConstness::Const,
                receiver_lifetime: None,
                raw_receiver: Some(CppIndirection::Pointer),
                inputs: vec![],
                output: i32.clone(),
                is_safe: false,
            },
            ZngurCppMethod {
                cpp_name: "raw_bump".to_owned(),
                lifetimes: vec![],
                template_args: vec![],
                rust_name: None,
                receiver: ZngurMethodReceiver::Ref(Mutability::Mut),
                receiver_constness: CppConstness::Var,
                receiver_lifetime: None,
                raw_receiver: Some(CppIndirection::Reference),
                inputs: vec![i32.clone()],
                output: RustType::UNIT,
                is_safe: false,
            },
            ZngurCppMethod {
                cpp_name: "make".to_owned(),
                lifetimes: vec![],
                template_args: vec![],
                rust_name: None,
                receiver: ZngurMethodReceiver::Static,
                receiver_constness: CppConstness::Var,
                receiver_lifetime: None,
                raw_receiver: None,
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

        // Rust: methods, in an `impl<__C0>` block for `const` member functions and an `impl a::B`
        // (non-`const`) block for the others.
        assert!(
            rust_code.contains(
                "impl<__C0: ::zngur_lib::Constness> a::B<__C0> {\n#[allow(non_snake_case, \
                 clippy::too_many_arguments)]\npub unsafe fn value(&self, ) -> i32"
            ),
            "{rust_code}"
        );
        // Inputs that are `const` C++ objects are generic over their constness.
        assert!(
            rust_code.contains(
                "impl a::B {\n#[allow(non_snake_case, clippy::too_many_arguments)]\npub unsafe fn \
                 set_from<__C1: ::zngur_lib::Constness>(&mut self, i1: *const a::B::<__C1>, ) -> ()"
            ),
            "{rust_code}"
        );
        assert!(
            rust_code.contains(
                "impl a::B {\n#[allow(non_snake_case, clippy::too_many_arguments)]\npub fn \
                 bump(&self, ) -> ()"
            ),
            "{rust_code}"
        );
        assert!(
            rust_code.contains(
                "impl<__C0: ::zngur_lib::Constness> a::B<__C0> {\n#[allow(non_snake_case, \
                 clippy::too_many_arguments)]\npub fn peek(&mut self, ) -> i32"
            ),
            "{rust_code}"
        );
        assert!(
            rust_code.contains(
                "impl<__C0: ::zngur_lib::Constness> a::B<__C0> {\n#[allow(non_snake_case, \
                 clippy::too_many_arguments)]\npub unsafe fn raw_value(i0: *const Self, ) -> i32"
            ),
            "{rust_code}"
        );
        assert!(
            rust_code.contains(
                "impl a::B {\n#[allow(non_snake_case, clippy::too_many_arguments)]\npub unsafe fn \
                 raw_bump(i0: *mut Self, i1: i32, ) -> ()"
            ),
            "{rust_code}"
        );
        assert!(rust_code.contains("pub fn make(i0: i32, i1: a::B, ) -> a::B"));

        // C++: bridges calling the member functions directly, through a `const` `this` for
        // `const` member functions.
        assert!(
            cpp_code.contains(
                "(::rust::__zngur_internal_move_from_rust< ::rust::CppPtr< ::rust::CppConst< \
                 ::rust::cpp::a::B > > >(i0))->value()"
            ),
            "{cpp_code}"
        );
        assert!(cpp_code.contains("->set(*"), "{cpp_code}");
        assert!(
            cpp_code.contains(
                "(::rust::__zngur_internal_move_from_rust< ::rust::CppPtr< ::rust::cpp::a::B > \
                 >(i0))->bump()"
            ),
            "{cpp_code}"
        );
        assert!(
            cpp_code.contains("::rust::CppConst< ::rust::cpp::a::B > > >(i0))->peek()"),
            "{cpp_code}"
        );
        assert!(
            cpp_code.contains(
                "(::rust::__zngur_internal_move_from_rust< ::rust::CppPtr< ::rust::CppConst< \
                 ::rust::cpp::a::B > > >(i0))->raw_value()"
            ),
            "{cpp_code}"
        );
        assert!(
            cpp_code.contains(
                "(::rust::__zngur_internal_move_from_rust< ::rust::CppPtr< ::rust::cpp::a::B > \
                 >(i0))->raw_bump(::rust::__zngur_internal_move_from_rust< ::int32_t >(i1))"
            ),
            "{cpp_code}"
        );
        assert!(
            cpp_code.contains("::new (static_cast<void*>(o)) ::a::B(::a::B::make("),
            "{cpp_code}"
        );
        assert!(cpp_code.contains(".cpp())"), "{cpp_code}");
    }

    #[test]
    fn cpp_methods_handle_bool_by_value_and_rust_value_references() {
        let b = cpp(&["a", "B"]);
        let lt_a = Lifetime("a".to_owned());
        let bool_ty = RustType::Primitive(PrimitiveRustType::Bool);
        let i32_ty = RustType::Primitive(PrimitiveRustType::Int(32));
        let foo_ty = adt(&["crate", "Foo"]);
        let ref_foo_ty = RustType::Ref(Mutability::Not, Some(lt_a.clone()), Box::new(foo_ty));
        let ref_i32_ty = RustType::Ref(
            Mutability::Not,
            Some(lt_a.clone()),
            Box::new(i32_ty.clone()),
        );
        let mut_ref_i32_ty = RustType::Ref(Mutability::Mut, Some(lt_a.clone()), Box::new(i32_ty));
        let opt_const_b_ty = RustType::Adt(RustPathAndGenerics {
            path: vec!["std".to_owned(), "option".to_owned(), "Option".to_owned()],
            lifetimes: vec![],
            generics: vec![RustType::Ref(
                Mutability::Not,
                Some(lt_a.clone()),
                Box::new(RustType::CppConst(Box::new(b.clone()))),
            )],
            named_generics: vec![],
        });
        let str_ty = RustType::Primitive(PrimitiveRustType::Str);
        let mut ty = minimal_opaque_cpp_type(b);
        ty.cpp_methods = vec![
            ZngurCppMethod {
                cpp_name: "check".to_owned(),
                lifetimes: vec![lt_a.clone()],
                template_args: vec![],
                rust_name: None,
                receiver: ZngurMethodReceiver::Ref(Mutability::Not),
                receiver_constness: CppConstness::Const,
                receiver_lifetime: Some(lt_a.clone()),
                raw_receiver: None,
                inputs: vec![
                    bool_ty.clone(),
                    ref_foo_ty.clone(),
                    RustType::Ref(Mutability::Not, Some(lt_a.clone()), Box::new(str_ty)),
                ],
                output: bool_ty,
                is_safe: true,
            },
            ZngurCppMethod {
                cpp_name: "borrow_foo".to_owned(),
                lifetimes: vec![lt_a.clone()],
                template_args: vec![],
                rust_name: None,
                receiver: ZngurMethodReceiver::Ref(Mutability::Not),
                receiver_constness: CppConstness::Const,
                receiver_lifetime: Some(lt_a.clone()),
                raw_receiver: None,
                inputs: vec![],
                output: ref_foo_ty,
                is_safe: true,
            },
            ZngurCppMethod {
                cpp_name: "nullable_and_builtin_refs".to_owned(),
                lifetimes: vec![lt_a.clone()],
                template_args: vec![],
                rust_name: None,
                receiver: ZngurMethodReceiver::Ref(Mutability::Not),
                receiver_constness: CppConstness::Const,
                receiver_lifetime: Some(lt_a),
                raw_receiver: None,
                inputs: vec![opt_const_b_ty.clone(), ref_i32_ty, mut_ref_i32_ty.clone()],
                output: opt_const_b_ty,
                is_safe: true,
            },
            ZngurCppMethod {
                cpp_name: "mut_int".to_owned(),
                lifetimes: vec![Lifetime("a".to_owned())],
                template_args: vec![],
                rust_name: None,
                receiver: ZngurMethodReceiver::Ref(Mutability::Mut),
                receiver_constness: CppConstness::Var,
                receiver_lifetime: Some(Lifetime("a".to_owned())),
                raw_receiver: None,
                inputs: vec![],
                output: mut_ref_i32_ty,
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
        assert!(
            rust_code.contains(
                "pub fn nullable_and_builtin_refs<'a, __C1: ::zngur_lib::Constness>(&'a self, \
                 i1: ::std::option::Option<&'a a::B::<__C1>>, i2: &'a i32, i3: &'a mut i32, ) \
                 -> ::std::option::Option::<&'a a::B::<::zngur_lib::Const>>"
            ),
            "expected generic-constness nullable C++ ref and Rust builtin refs in:\n{rust_code}"
        );
        assert!(
            cpp_code.contains("static_cast<bool>(*i1)"),
            "expected by-value bool cast in:\n{cpp_code}"
        );
        assert!(
            cpp_code.contains("bool __zngur_o = "),
            "expected direct bool return handling in:\n{cpp_code}"
        );
        assert!(
            cpp_code.contains("::rust::Ref< ::rust::test_crate::Foo >"),
            "expected `::rust::Ref< ::rust::test_crate::Foo >` in:\n{cpp_code}"
        );
        assert!(
            cpp_code.contains(
                "static_assert(::rust::__zngur_same_repr< decltype((::rust::__zngur_internal_move_from_rust< \
                 ::rust::CppPtr< ::rust::CppConst< ::rust::cpp::a::B > > >(i0))->borrow_foo()), \
                 ::rust::Ref< ::rust::test_crate::Foo > >::value,"
            ),
            "expected return-type static_assert for `&'a crate::Foo` in:\n{cpp_code}"
        );
        assert!(
            cpp_code.contains(
                "::rust::__zngur_internal_move_from_rust< ::rust::CppPtr< ::rust::CppConst< ::rust::cpp::a::B > > >(i1)"
            ) && cpp_code.contains(
                "::rust::CppPtr< ::rust::CppConst< ::rust::cpp::a::B > > __zngur_o = "
            ),
            "expected `CppPtr<CppConst<...>>` lowering for `Option<&'a const c++::a::B>` in:\n{cpp_code}"
        );
        assert!(
            cpp_code.contains(
                "*::rust::__zngur_internal_move_from_rust< ::rust::CppPtr< ::rust::CppConst< ::int32_t > > >(i2)"
            ) && cpp_code.contains(
                "*::rust::__zngur_internal_move_from_rust< ::rust::CppPtr< ::int32_t > >(i3)"
            ) && cpp_code.contains(
                "::rust::CppTypeOf< ::int32_t >::type&"
            ) && cpp_code.contains(
                "::rust::CppPtr< ::int32_t > __zngur_o = &("
            ),
            "expected C++ reference lowering for `&i32` / `&mut i32` in:\n{cpp_code}"
        );
        assert!(
            cpp_code.contains("::rust::Ref< ::rust::Str >"),
            "expected `::rust::Ref< ::rust::Str >` in:\n{cpp_code}"
        );
        assert!(
            !cpp_code.contains("CppPtr< ::rust::test_crate::Foo >"),
            "unexpected `CppPtr` on Rust ADT in:\n{cpp_code}"
        );
        assert!(
            !cpp_code.contains("CppPtr< ::rust::Str >"),
            "unexpected `CppPtr` on `str` in:\n{cpp_code}"
        );
    }

    #[test]
    fn cpp_methods_move_imported_by_value_types_like_local_ones() {
        let imported_bar = cpp(&["ns", "Bar"]);
        let mut local_ty = minimal_opaque_cpp_type(cpp(&["ns", "Local"]));
        local_ty.cpp_methods = vec![ZngurCppMethod {
            cpp_name: "round_trip".to_owned(),
            lifetimes: vec![],
            template_args: vec![],
            rust_name: None,
            receiver: ZngurMethodReceiver::Static,
            receiver_constness: CppConstness::Var,
            receiver_lifetime: None,
            raw_receiver: None,
            inputs: vec![imported_bar.clone()],
            output: imported_bar.clone(),
            is_safe: true,
        }];
        let spec = ZngurSpec {
            imported_modules: vec![ModuleImport {
                path: "./bar.zng".into(),
                crate_name: Some("bar_bindings".to_owned()),
                cpp_types: vec![vec!["ns".to_owned(), "Bar".to_owned()]],
                cpp_by_value_types: vec![(imported_bar, "::ns::Bar".to_owned())],
                ..Default::default()
            }],
            types: vec![local_ty],
            ..Default::default()
        };
        let (_rust_code, _h, cpp_code) =
            ZngurGenerator::build_from_zng(spec, "test_crate".to_owned()).render(false);
        let cpp_code = cpp_code.expect("C++ member functions need a C++ file");
        assert!(
            cpp_code.contains(
                "::rust::__zngur_internal_move_from_rust< ::rust::cpp::ns::Bar >(i0).cpp()"
            ),
            "{cpp_code}"
        );
        assert!(
            cpp_code.contains("::new (static_cast<void*>(o)) ::ns::Bar(::ns::Local::round_trip("),
            "{cpp_code}"
        );
    }

    #[test]
    fn cpp_method_bridge_symbols_do_not_collide_across_underscore_boundaries_or_extern_cpp() {
        let mut foo_cpp_method = minimal_opaque_cpp_type(cpp(&["Foo_cpp_method"]));
        foo_cpp_method.cpp_methods = vec![ZngurCppMethod {
            cpp_name: "bar".to_owned(),
            lifetimes: vec![],
            template_args: vec![],
            rust_name: None,
            receiver: ZngurMethodReceiver::Ref(Mutability::Not),
            receiver_constness: CppConstness::Const,
            receiver_lifetime: None,
            raw_receiver: None,
            inputs: vec![],
            output: RustType::UNIT,
            is_safe: true,
        }];
        let mut foo = minimal_opaque_cpp_type(cpp(&["Foo"]));
        foo.cpp_methods = vec![
            ZngurCppMethod {
                cpp_name: "cpp_method_bar".to_owned(),
                lifetimes: vec![],
                template_args: vec![],
                rust_name: None,
                receiver: ZngurMethodReceiver::Ref(Mutability::Not),
                receiver_constness: CppConstness::Const,
                receiver_lifetime: None,
                raw_receiver: None,
                inputs: vec![],
                output: RustType::UNIT,
                is_safe: true,
            },
            ZngurCppMethod {
                cpp_name: "bar".to_owned(),
                lifetimes: vec![],
                template_args: vec![],
                rust_name: None,
                receiver: ZngurMethodReceiver::Ref(Mutability::Not),
                receiver_constness: CppConstness::Const,
                receiver_lifetime: None,
                raw_receiver: None,
                inputs: vec![],
                output: RustType::UNIT,
                is_safe: true,
            },
        ];
        let spec = ZngurSpec {
            types: vec![foo_cpp_method, foo],
            extern_cpp_funcs: vec![ZngurExternCppFn {
                name: "Foo_cpp_method_bar".to_owned(),
                lifetimes: vec![],
                inputs: vec![],
                output: RustType::UNIT,
                is_safe: true,
            }],
            ..Default::default()
        };
        let (rust_code, _h, _cpp) =
            ZngurGenerator::build_from_zng(spec, "test_crate".to_owned()).render(false);
        let extern_fns: Vec<&str> = rust_code
            .lines()
            .filter_map(|line| {
                line.strip_prefix("unsafe extern \"C\" { fn ")
                    .and_then(|rest| rest.split('(').next())
            })
            .collect();
        assert_eq!(
            extern_fns.len(),
            4,
            "expected 4 extern C declarations:\n{rust_code}"
        );
        let unique: std::collections::HashSet<&str> = extern_fns.iter().copied().collect();
        assert_eq!(
            unique.len(),
            extern_fns.len(),
            "colliding extern C bridge symbols: {extern_fns:?}"
        );
    }

    #[test]
    fn const_cpp_method_does_not_shadow_cpp_type_or_namespace_named_c() {
        let mut top_c = minimal_opaque_cpp_type(cpp(&["C"]));
        let mut nested_c_foo = minimal_opaque_cpp_type(cpp(&["ns", "C", "Foo"]));
        let lt_a = Lifetime("a".to_owned());
        top_c.cpp_methods = vec![ZngurCppMethod {
            cpp_name: "id".to_owned(),
            lifetimes: vec![lt_a.clone()],
            template_args: vec![],
            rust_name: None,
            receiver: ZngurMethodReceiver::Ref(Mutability::Not),
            receiver_constness: CppConstness::Const,
            receiver_lifetime: Some(lt_a.clone()),
            raw_receiver: None,
            inputs: vec![RustType::Ref(
                Mutability::Not,
                Some(lt_a.clone()),
                Box::new(RustType::CppConst(Box::new(cpp(&["C"])))),
            )],
            output: RustType::Ref(
                Mutability::Not,
                Some(lt_a.clone()),
                Box::new(RustType::CppConst(Box::new(cpp(&["C"])))),
            ),
            is_safe: true,
        }];
        nested_c_foo.cpp_methods = vec![ZngurCppMethod {
            cpp_name: "get".to_owned(),
            lifetimes: vec![lt_a.clone()],
            template_args: vec![],
            rust_name: None,
            receiver: ZngurMethodReceiver::Ref(Mutability::Not),
            receiver_constness: CppConstness::Const,
            receiver_lifetime: Some(lt_a),
            raw_receiver: None,
            inputs: vec![],
            output: RustType::Primitive(PrimitiveRustType::Int(32)),
            is_safe: true,
        }];
        let spec = ZngurSpec {
            types: vec![top_c, nested_c_foo],
            ..Default::default()
        };
        let (rust_code, _h, _cpp) =
            ZngurGenerator::build_from_zng(spec, "test_crate".to_owned()).render(false);
        assert!(
            rust_code.contains(
                "impl<__C0: ::zngur_lib::Constness> C<__C0> {\n#[allow(non_snake_case, \
                 clippy::too_many_arguments)]\npub fn id<'a, __C1: ::zngur_lib::Constness>(&'a \
                 self, i1: &'a C::<__C1>, ) -> &'a C::<::zngur_lib::Const>"
            ),
            "{rust_code}"
        );
        assert!(
            rust_code.contains("impl<__C0: ::zngur_lib::Constness> ns::C::Foo<__C0>"),
            "{rust_code}"
        );
    }

    #[test]
    fn cpp_constructors_get_associated_functions_and_cpp_bridges() {
        let b = cpp(&["a", "B"]);
        let i32 = RustType::Primitive(PrimitiveRustType::Int(32));
        let dyn_trait = RustType::Boxed(Box::new(RustType::Dyn(
            RustTrait::Normal(RustPathAndGenerics {
                path: vec!["crate".to_owned(), "MyTrait".to_owned()],
                lifetimes: vec![Lifetime("_".to_owned())],
                generics: vec![RustType::Ref(Mutability::Not, None, Box::new(i32.clone()))],
                ..Default::default()
            }),
            vec![],
        )));
        let dyn_fn = RustType::Boxed(Box::new(RustType::Dyn(
            RustTrait::Fn {
                name: "Fn".to_owned(),
                lifetimes: vec![],
                inputs: vec![RustType::Ref(Mutability::Not, None, Box::new(i32.clone()))],
                output: Box::new(RustType::UNIT),
            },
            vec![],
        )));
        let higher_ranked_fn = RustType::Boxed(Box::new(RustType::Dyn(
            RustTrait::Fn {
                name: "Fn".to_owned(),
                lifetimes: vec![Lifetime("__l1".to_owned())],
                inputs: vec![RustType::Ref(
                    Mutability::Not,
                    Some(Lifetime("__l1".to_owned())),
                    Box::new(i32.clone()),
                )],
                output: Box::new(RustType::UNIT),
            },
            vec![],
        )));
        let mut ty = minimal_by_value_cpp_type(b.clone(), 8, 4);
        ty.cpp_constructors = vec![
            ZngurCppConstructor {
                rust_name: "new".to_owned(),
                lifetimes: vec![],
                inputs: vec![],
                is_safe: true,
            },
            ZngurCppConstructor {
                rust_name: "shifted".to_owned(),
                lifetimes: vec![],
                inputs: vec![
                    i32,
                    RustType::cpp_ptr(Mutability::Not, CppIndirection::Reference, b.clone()),
                ],
                is_safe: false,
            },
            ZngurCppConstructor {
                rust_name: "copy".to_owned(),
                lifetimes: vec![Lifetime("a".to_owned())],
                inputs: vec![RustType::Ref(
                    Mutability::Not,
                    Some(Lifetime("a".to_owned())),
                    Box::new(RustType::CppConst(Box::new(b.clone()))),
                )],
                is_safe: true,
            },
            // With `#allow(elided_lifetimes)`.
            ZngurCppConstructor {
                rust_name: "copy_elided".to_owned(),
                lifetimes: vec![],
                inputs: vec![RustType::Ref(
                    Mutability::Not,
                    None,
                    Box::new(RustType::CppConst(Box::new(b.clone()))),
                )],
                is_safe: true,
            },
            // Normal `dyn Trait<'_, &T>` elided lifetimes are free function-input lifetimes (not
            // higher-ranked like `dyn Fn(&T)`) and must be named and captured in `use<...>`.
            ZngurCppConstructor {
                rust_name: "from_traits".to_owned(),
                lifetimes: vec![],
                inputs: vec![
                    RustType::Ref(
                        Mutability::Not,
                        None,
                        Box::new(RustType::CppConst(Box::new(b.clone()))),
                    ),
                    dyn_trait,
                    dyn_fn,
                ],
                is_safe: true,
            },
            // Synthesized `'__l{n}` names skip user-declared lifetimes of the same name.
            ZngurCppConstructor {
                rust_name: "from_two".to_owned(),
                lifetimes: vec![Lifetime("__l1".to_owned())],
                inputs: vec![
                    RustType::Ref(
                        Mutability::Not,
                        Some(Lifetime("__l1".to_owned())),
                        Box::new(RustType::CppConst(Box::new(b.clone()))),
                    ),
                    RustType::Ref(
                        Mutability::Not,
                        Some(Lifetime("_".to_owned())),
                        Box::new(RustType::CppConst(Box::new(b.clone()))),
                    ),
                ],
                is_safe: true,
            },
            // Synthesized `'__l{n}` names also skip higher-ranked `for<...>` binders in inputs.
            ZngurCppConstructor {
                rust_name: "from_higher_ranked".to_owned(),
                lifetimes: vec![],
                inputs: vec![
                    higher_ranked_fn,
                    RustType::Ref(
                        Mutability::Not,
                        None,
                        Box::new(RustType::CppConst(Box::new(b))),
                    ),
                ],
                is_safe: true,
            },
        ];
        let spec = ZngurSpec {
            types: vec![ty],
            ..Default::default()
        };
        let (rust_code, _h, cpp_code) =
            ZngurGenerator::build_from_zng(spec, "test_crate".to_owned()).render(false);
        let cpp_code = cpp_code.expect("C++ constructors need a C++ file");

        assert!(
            rust_code.contains("pub fn new() -> impl ::zngur_lib::CppCtor<Output = Self> {"),
            "{rust_code}"
        );
        // `unsafe` constructors are `unsafe` functions, and inputs that are `const` C++ objects
        // are generic over their constness. The returned constructor call captures the generic
        // parameters, borrowed inputs' lifetimes included.
        assert!(
            rust_code.contains(
                "pub unsafe fn shifted<__C1: ::zngur_lib::Constness>(i0: i32, i1: *const \
                 a::B::<__C1>, ) -> impl ::zngur_lib::CppCtor<Output = Self> + use<__C1> {"
            ),
            "{rust_code}"
        );
        assert!(
            rust_code.contains(
                "pub fn copy<'a, __C1: ::zngur_lib::Constness>(i0: &'a a::B::<__C1>, ) -> impl \
                 ::zngur_lib::CppCtor<Output = Self> + use<'a, __C1> {"
            ),
            "{rust_code}"
        );
        // Elided lifetimes get names, so that the constructor call can capture them.
        assert!(
            rust_code.contains(
                "pub fn copy_elided<'__l1, __C1: ::zngur_lib::Constness>(i0: &'__l1 a::B::<__C1>, \
                 ) -> impl ::zngur_lib::CppCtor<Output = Self> + use<'__l1, __C1> {"
            ),
            "{rust_code}"
        );
        assert!(
            rust_code.contains(
                "pub fn from_traits<'__l1, '__l2, '__l3, __C1: ::zngur_lib::Constness>(i0: &'__l1 \
                 a::B::<__C1>, i1: Box<dyn crate::MyTrait::<'__l2, &'__l3 i32>>, i2: \
                 Box<dyn Fn(&i32)>, ) -> impl ::zngur_lib::CppCtor<Output = \
                 Self> + use<'__l1, '__l2, '__l3, __C1> {"
            ),
            "{rust_code}"
        );
        assert!(
            rust_code.contains(
                "pub fn from_two<'__l1, '__l2, __C1: ::zngur_lib::Constness, __C2: \
                 ::zngur_lib::Constness>(i0: &'__l1 a::B::<__C1>, i1: &'__l2 a::B::<__C2>, ) -> \
                 impl ::zngur_lib::CppCtor<Output = Self> + use<'__l1, '__l2, __C1, __C2> {"
            ),
            "{rust_code}"
        );
        assert!(
            rust_code.contains(
                "pub fn from_higher_ranked<'__l2, __C1: ::zngur_lib::Constness>(i0: Box<dyn \
                 for<'__l1> Fn(&'__l1 i32)>, i1: &'__l2 a::B::<__C1>, ) -> impl \
                 ::zngur_lib::CppCtor<Output = Self> + use<'__l2, __C1> {"
            ),
            "{rust_code}"
        );

        assert!(
            cpp_code.contains("::new (static_cast<void*>(o)) ::a::B();"),
            "{cpp_code}"
        );
        assert!(
            cpp_code.contains(
                "::new (static_cast<void*>(o)) \
                 ::a::B(::rust::__zngur_internal_move_from_rust< ::int32_t >(i0), *"
            ),
            "{cpp_code}"
        );
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
        let up_b = unique_ptr(b.clone());
        let up_i32 = unique_ptr(RustType::Primitive(PrimitiveRustType::Int(32)));
        let RustType::Cpp(up_b_path) = &up_b else {
            unreachable!()
        };
        let RustType::Cpp(up_i32_path) = &up_i32 else {
            unreachable!()
        };
        let up_b_struct = struct_name(up_b_path);
        let up_i32_struct = struct_name(up_i32_path);
        let spec = ZngurSpec {
            types: vec![
                minimal_opaque_cpp_type(b.clone()),
                minimal_by_value_cpp_type(up_b.clone(), 8, 8),
                minimal_opaque_cpp_type(up_i32.clone()),
            ],
            ..Default::default()
        };
        let (rust_code, _h, _cpp) =
            ZngurGenerator::build_from_zng(spec, "test_crate".to_owned()).render(false);

        // One trait and alias for the template, in the template's module.
        assert!(rust_code.contains("pub mod std {"), "{rust_code}");
        assert_eq!(
            rust_code
                .matches(
                    "pub trait __ZngurCppTemplate_unique_ptr<C: ::zngur_lib::Constness = \
                     ::zngur_lib::NonConst> {"
                )
                .count(),
            1,
            "{rust_code}"
        );
        assert_eq!(
            rust_code
                .matches(
                    "pub type unique_ptr<T0, C = ::zngur_lib::NonConst> = <T0 as \
                     __ZngurCppTemplate_unique_ptr<C>>::Type;"
                )
                .count(),
            1,
            "{rust_code}"
        );
        // Each instantiation is a struct that the trait names for its arguments.
        assert!(
            rust_code.contains(&format!(
                "impl<C: ::zngur_lib::Constness> std::__ZngurCppTemplate_unique_ptr<C> for a::B {{\n    \
                 type Type = {up_b_struct}<C>;\n}}"
            )),
            "{rust_code}"
        );
        assert!(
            rust_code.contains(&format!(
                "impl<C: ::zngur_lib::Constness> std::__ZngurCppTemplate_unique_ptr<C> for i32 {{\n    \
                 type Type = {up_i32_struct}<C>;\n}}"
            )),
            "{rust_code}"
        );
        // Only by-value instantiations are `ZngCppStackObject`s.
        assert!(
            rust_code.contains(&format!(
                "unsafe impl<C: ::zngur_lib::Constness> ::zngur_lib::ZngCppStackObject for \
                 {up_b_struct}<C> {{}}"
            )),
            "{rust_code}"
        );
        assert!(
            !rust_code.contains(&format!(
                "::zngur_lib::ZngCppStackObject for {up_i32_struct}<C>"
            )),
            "{rust_code}"
        );
    }

    #[test]
    fn cpp_template_instantiations_use_an_imported_home() {
        let b = cpp(&["a", "B"]);
        let i32_ty = RustType::Primitive(PrimitiveRustType::Int(32));
        let unique_ptr_path = vec!["std".to_owned(), "unique_ptr".to_owned()];
        let map_path = vec!["std".to_owned(), "map".to_owned()];
        let span_path = vec!["std".to_owned(), "span".to_owned()];
        let vec_path = vec!["std".to_owned(), "vector".to_owned()];

        let up_b_path = CppTypePath {
            path: unique_ptr_path.clone(),
            generics: vec![b.clone()],
        };
        let map_i32_b_path = CppTypePath {
            path: map_path.clone(),
            generics: vec![i32_ty, b.clone()],
        };
        let span_const_b_path = CppTypePath {
            path: span_path.clone(),
            generics: vec![RustType::CppConst(Box::new(b.clone()))],
        };
        let up_b_struct = struct_name(&up_b_path);
        let map_struct = struct_name(&map_i32_b_path);
        let span_struct = struct_name(&span_const_b_path);

        let bar_import = ModuleImport {
            path: "./bar.zng".into(),
            crate_name: Some("bar".to_owned()),
            cpp_template_homes: vec![
                CppTemplateHome {
                    path: unique_ptr_path.clone(),
                    arity: 1,
                    explicit: true,
                },
                CppTemplateHome {
                    path: map_path.clone(),
                    arity: 2,
                    explicit: true,
                },
                CppTemplateHome {
                    path: span_path.clone(),
                    arity: 1,
                    explicit: true,
                },
            ],
            cpp_templates: vec![unique_ptr_path, map_path, span_path],
            ..Default::default()
        };
        // `mid` instantiates `std::vector<mid::Item>` whose home isn't directly imported here;
        // `cpp_templates` still re-exports `std::vector` so signatures using it compile.
        let mid_import = ModuleImport {
            path: "./mid.zng".into(),
            crate_name: Some("mid".to_owned()),
            cpp_types: vec![vec!["mid".to_owned(), "Item".to_owned()]],
            cpp_templates: vec![vec_path],
            ..Default::default()
        };
        let spec = ZngurSpec {
            types: vec![
                minimal_opaque_cpp_type(b),
                minimal_by_value_cpp_type(RustType::Cpp(up_b_path), 8, 8),
                minimal_opaque_cpp_type(RustType::Cpp(map_i32_b_path)),
                minimal_by_value_cpp_type(RustType::Cpp(span_const_b_path), 16, 8),
            ],
            // Duplicate `bar_import` also verifies `pub use` deduplication.
            imported_modules: vec![bar_import.clone(), bar_import, mid_import],
            ..Default::default()
        };
        let (rust_code, _h, _cpp) =
            ZngurGenerator::build_from_zng(spec, "test_crate".to_owned()).render(false);

        assert!(
            !rust_code.contains("pub trait __ZngurCppTemplate_unique_ptr"),
            "{rust_code}"
        );
        assert_eq!(
            rust_code
                .matches("pub use ::bar::generated::std::unique_ptr;")
                .count(),
            1,
            "{rust_code}"
        );
        assert_eq!(
            rust_code
                .matches("pub use ::bar::generated::std::map;")
                .count(),
            1,
            "{rust_code}"
        );
        assert_eq!(
            rust_code
                .matches("pub use ::bar::generated::std::span;")
                .count(),
            1,
            "{rust_code}"
        );
        assert_eq!(
            rust_code
                .matches("pub use ::mid::generated::std::vector;")
                .count(),
            1,
            "{rust_code}"
        );
        assert!(
            rust_code.contains(&format!(
                "impl<C: ::zngur_lib::Constness> ::bar::generated::std::__ZngurCppTemplate_unique_ptr<C> \
                 for a::B {{\n    type Type = {up_b_struct}<C>;\n}}"
            )),
            "{rust_code}"
        );
        assert!(
            rust_code.contains(&format!(
                "impl<C: ::zngur_lib::Constness> ::bar::generated::std::__ZngurCppTemplate_map<a::B, C> \
                 for i32 {{\n    type Type = {map_struct}<C>;\n}}"
            )),
            "{rust_code}"
        );
        assert!(
            rust_code.contains(&format!(
                "impl<C: ::zngur_lib::Constness> ::bar::generated::std::__ZngurCppTemplate_span<C> \
                 for a::B::<::zngur_lib::Const> {{\n    type Type = {span_struct}<C>;\n}}"
            )),
            "{rust_code}"
        );

        // When the importing module also declares a free C++ function with the same Rust name in
        // that namespace, re-exporting uses a `pub type` alias instead of `pub use` so the value
        // namespace does not collide.
        let collide_spec = ZngurSpec {
            imported_modules: vec![
                ModuleImport {
                    path: "./bar.zng".into(),
                    crate_name: Some("bar".to_owned()),
                    cpp_template_homes: vec![CppTemplateHome {
                        path: vec!["ns".to_owned(), "Box".to_owned()],
                        arity: 1,
                        explicit: true,
                    }],
                    cpp_templates: vec![vec!["ns".to_owned(), "Box".to_owned()]],
                    ..Default::default()
                },
                ModuleImport {
                    path: "./mid.zng".into(),
                    crate_name: Some("mid".to_owned()),
                    cpp_template_instantiations: vec![CppTypePath {
                        path: vec!["ns".to_owned(), "Pair".to_owned()],
                        generics: vec![
                            RustType::Primitive(PrimitiveRustType::Int(32)),
                            RustType::Primitive(PrimitiveRustType::Int(64)),
                        ],
                    }],
                    cpp_templates: vec![vec!["ns".to_owned(), "Pair".to_owned()]],
                    ..Default::default()
                },
            ],
            cpp_funcs: vec![
                zngur_def::ZngurCppFn {
                    namespace: vec!["ns".to_owned()],
                    cpp_name: "Box".to_owned(),
                    rust_name: None,
                    lifetimes: vec![],
                    template_args: vec![],
                    inputs: vec![],
                    output: RustType::Tuple(vec![]),
                    is_safe: true,
                },
                zngur_def::ZngurCppFn {
                    namespace: vec!["ns".to_owned()],
                    cpp_name: "Pair".to_owned(),
                    rust_name: None,
                    lifetimes: vec![],
                    template_args: vec![],
                    inputs: vec![],
                    output: RustType::Tuple(vec![]),
                    is_safe: true,
                },
            ],
            ..Default::default()
        };
        let (collide_rust, _, _) =
            ZngurGenerator::build_from_zng(collide_spec, "test_crate".to_owned()).render(false);
        assert!(
            collide_rust.contains(
                "pub type Box<T0, C = ::zngur_lib::NonConst> = ::bar::generated::ns::Box<T0, C>;"
            ),
            "{collide_rust}"
        );
        assert!(
            collide_rust.contains(
                "pub type Pair<T0, T1, C = ::zngur_lib::NonConst> = ::mid::generated::ns::Pair<T0, T1, C>;"
            ),
            "{collide_rust}"
        );
        assert!(
            !collide_rust.contains("pub use ::bar::generated::ns::Box;"),
            "{collide_rust}"
        );
        assert!(
            !collide_rust.contains("pub use ::mid::generated::ns::Pair;"),
            "{collide_rust}"
        );
    }

    #[test]
    fn const_cpp_fields_and_template_arguments() {
        let b = cpp(&["a", "B"]);
        let i32 = RustType::Primitive(PrimitiveRustType::Int(32));
        let mut ty = minimal_by_value_cpp_type(b, 8, 4);
        ty.cpp_fields = vec![ZngurCppField {
            name: "x".to_owned(),
            rust_name: None,
            ty: i32.clone(),
            access: CppFieldAccess::Ref {
                unique: false,
                constness: CppConstness::Const,
            },
            is_safe: true,
        }];
        let span = RustType::Cpp(CppTypePath {
            path: vec!["std".to_owned(), "span".to_owned()],
            generics: vec![RustType::CppConst(Box::new(i32))],
        });
        let spec = ZngurSpec {
            types: vec![ty, minimal_by_value_cpp_type(span, 16, 8)],
            ..Default::default()
        };
        let (rust_code, h, cpp_code) =
            ZngurGenerator::build_from_zng(spec, "test_crate".to_owned()).render(false);
        let cpp_code = cpp_code.expect("C++ field accessors need a C++ file");

        assert!(
            rust_code.contains("pub fn x(&self, ) -> &::zngur_lib::Cpp<i32, ::zngur_lib::Const>"),
            "{rust_code}"
        );
        assert!(
            cpp_code.contains(
                "::rust::CppPtr< ::rust::CppConst< ::int32_t > > __zngur_o = \
                 ::std::addressof((::rust::__zngur_internal_move_from_rust< ::rust::CppPtr< \
                 ::rust::CppConst< ::rust::cpp::a::B > > >(i0))->x);"
            ),
            "{cpp_code}"
        );
        assert!(
            rust_code.contains(
                "impl<C: ::zngur_lib::Constness> std::__ZngurCppTemplate_span<C> for \
                 ::zngur_lib::Cpp<i32, ::zngur_lib::Const> {"
            ),
            "{rust_code}"
        );
        assert!(
            h.contains("static_assert(sizeof(::std::span< const ::int32_t >) == 16"),
            "{h}"
        );
    }

    #[test]
    fn cpp_template_pointer_args_and_nested_const_names() {
        // Distinct nested `const` and `::` vs `__` paths produce distinct struct names.
        let node = cpp(&["Node"]);
        let span_of = |arg: RustType| CppTypePath {
            path: vec!["std".to_owned(), "span".to_owned()],
            generics: vec![arg],
        };
        let unique_ptr_of = |arg: RustType| {
            RustType::Cpp(CppTypePath {
                path: vec!["std".to_owned(), "unique_ptr".to_owned()],
                generics: vec![arg],
            })
        };
        let span_const_up = span_of(RustType::CppConst(Box::new(unique_ptr_of(node.clone()))));
        let span_up_const = span_of(unique_ptr_of(RustType::CppConst(Box::new(node))));
        assert_ne!(struct_name(&span_const_up), struct_name(&span_up_const));

        let foo_a_b = CppTypePath {
            path: vec!["Foo".to_owned()],
            generics: vec![cpp(&["a", "b"])],
        };
        let foo_a_double_b = CppTypePath {
            path: vec!["Foo".to_owned()],
            generics: vec![cpp(&["a__b"])],
        };
        assert_ne!(struct_name(&foo_a_b), struct_name(&foo_a_double_b));

        // Declaring `map<i32, const a::B*>` BEFORE `a::B` still emits `CppTypeOf<a::B>` before
        // `CppTypeOf<map<...>>` and never emits unmangled `*` in `extern "C"` identifiers.
        let b = cpp(&["a", "B"]);
        let map_ty = RustType::Cpp(CppTypePath {
            path: vec!["std".to_owned(), "map".to_owned()],
            generics: vec![
                RustType::Primitive(PrimitiveRustType::Int(32)),
                RustType::cpp_ptr(Mutability::Not, CppIndirection::Pointer, b.clone()),
            ],
        });
        let mut map_def = minimal_by_value_cpp_type(map_ty, 48, 8);
        map_def.cpp_methods = vec![ZngurCppMethod {
            cpp_name: "size".to_owned(),
            lifetimes: vec![],
            template_args: vec![],
            rust_name: None,
            receiver: ZngurMethodReceiver::Ref(Mutability::Not),
            receiver_lifetime: None,
            receiver_constness: CppConstness::Const,
            raw_receiver: None,
            inputs: vec![],
            output: RustType::Primitive(PrimitiveRustType::Usize),
            is_safe: true,
        }];
        let spec = ZngurSpec {
            types: vec![map_def, minimal_opaque_cpp_type(b)],
            ..Default::default()
        };
        let (rust_code, h, cpp_code) =
            ZngurGenerator::build_from_zng(spec, "test_crate".to_owned()).render(false);
        let cpp_code = cpp_code.expect("C++ methods need a C++ file");

        assert!(
            rust_code.contains(
                "pub trait __ZngurCppTemplate_map<T1, C: ::zngur_lib::Constness = \
                 ::zngur_lib::NonConst> {"
            ),
            "{rust_code}"
        );
        assert!(
            rust_code.contains(
                "pub type map<T0, T1, C = ::zngur_lib::NonConst> = <T0 as \
                 __ZngurCppTemplate_map<T1, C>>::Type;"
            ),
            "{rust_code}"
        );
        assert!(
            rust_code.contains(
                "impl<C: ::zngur_lib::Constness> std::__ZngurCppTemplate_map<*const \
                 a::B::<::zngur_lib::Const>, C> for i32 {"
            ),
            "{rust_code}"
        );
        for line in rust_code.lines().chain(h.lines()).chain(cpp_code.lines()) {
            if let Some(after) = line.split("fn _zngur_").nth(1) {
                let ident = after.split('(').next().unwrap_or("");
                assert!(
                    !ident.contains('*'),
                    "unmangled `*` in extern symbol `{ident}`: {line}"
                );
            }
        }
        let pos_b = h
            .find("struct CppTypeOf< ::rust::cpp::a::B >")
            .expect("missing CppTypeOf for a::B");
        let pos_map = h
            .find("struct CppTypeOf< ::rust::cpp::std::map<")
            .expect("missing CppTypeOf for std::map");
        assert!(
            pos_b < pos_map,
            "CppTypeOf<a::B> must precede CppTypeOf<std::map<..., const a::B*>>:\n{h}"
        );

        // `CppTypeOf` specializations must also precede `zngur_is_unsized` specializations so
        // unsized slice/dyn types mentioning `CppPtr<...>` (e.g. `[c++::View<const c++::Node*>]`)
        // do not instantiate `CppTypeOf` before its explicit specialization.
        let node = cpp(&["Node"]);
        let view_ty = RustType::Cpp(CppTypePath {
            path: vec!["View".to_owned()],
            generics: vec![RustType::cpp_ptr(
                Mutability::Not,
                CppIndirection::Pointer,
                node.clone(),
            )],
        });
        let slice_def = ZngurType {
            ty: RustType::Slice(Box::new(view_ty.clone())),
            lifetimes: vec![],
            layout: Some(LayoutPolicy::OnlyByRef),
            trivially_relocatable: TriviallyRelocatable::No,
            wellknown_traits: vec![ZngurWellknownTrait::Unsized],
            exhaustive: true,
            methods: vec![],
            cpp_methods: vec![],
            cpp_constructors: vec![],
            cpp_fields: vec![],
            constructor: None,
            variants: vec![],
            fields: vec![],
            cpp_heap_allocated: None,
            cpp_ref: None,
            cpp_stack_owned: None,
        };
        let (_, unsized_h, _) = ZngurGenerator::build_from_zng(
            ZngurSpec {
                types: vec![
                    slice_def,
                    minimal_opaque_cpp_type(view_ty),
                    minimal_opaque_cpp_type(node),
                ],
                ..Default::default()
            },
            "test_crate".to_owned(),
        )
        .render(false);
        let pos_node_cpp_type = unsized_h
            .find("struct CppTypeOf< ::rust::cpp::Node >")
            .expect("missing CppTypeOf for Node");
        let pos_view_cpp_type = unsized_h
            .find("struct CppTypeOf< ::rust::cpp::View<")
            .expect("missing CppTypeOf for View");
        let pos_unsized = unsized_h
            .find("struct zngur_is_unsized<")
            .expect("missing zngur_is_unsized");
        assert!(
            pos_node_cpp_type < pos_view_cpp_type && pos_view_cpp_type < pos_unsized,
            "CppTypeOf specializations must precede zngur_is_unsized:\n{unsized_h}"
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
        let mut point_ty = minimal_by_value_cpp_type(point.clone(), 8, 4);
        point_ty.cpp_methods = vec![
            ZngurCppMethod {
                cpp_name: "cast".to_owned(),
                lifetimes: vec![],
                template_args: vec![RustType::CppConst(Box::new(i32.clone()))],
                rust_name: Some("cast_i32".to_owned()),
                receiver: ZngurMethodReceiver::Ref(Mutability::Not),
                receiver_constness: CppConstness::Const,
                receiver_lifetime: None,
                raw_receiver: None,
                inputs: vec![],
                output: i32.clone(),
                is_safe: true,
            },
            ZngurCppMethod {
                cpp_name: "from".to_owned(),
                lifetimes: vec![],
                template_args: vec![i32.clone(), i32.clone()],
                rust_name: Some("from_i32_pair".to_owned()),
                receiver: ZngurMethodReceiver::Static,
                receiver_constness: CppConstness::Var,
                receiver_lifetime: None,
                raw_receiver: None,
                inputs: vec![i32.clone(), i32.clone()],
                output: point.clone(),
                is_safe: true,
            },
        ];
        let spec = ZngurSpec {
            types: vec![
                point_ty,
                minimal_by_value_cpp_type(unique_ptr.clone(), 8, 8),
            ],
            cpp_funcs: vec![ZngurCppFn {
                lifetimes: vec![],
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
            rust_code.contains("pub fn cast_i32(&self, ) -> i32"),
            "{rust_code}"
        );
        assert!(
            rust_code.contains("pub fn from_i32_pair(i0: i32, i1: i32, ) -> geo::Point"),
            "{rust_code}"
        );
        assert!(
            cpp_code.contains(
                "::new (static_cast<void*>(o)) ::std::unique_ptr< ::geo::Point \
                 >(::std::make_unique< ::geo::Point \
                 >(::rust::__zngur_internal_move_from_rust< ::int32_t >(i0), "
            ),
            "{cpp_code}"
        );
        assert!(
            cpp_code.contains(
                "(::rust::__zngur_internal_move_from_rust< ::rust::CppPtr< ::rust::CppConst< \
                 ::rust::cpp::geo::Point > > >(i0))->cast< const ::int32_t >()"
            ),
            "{cpp_code}"
        );
        assert!(
            cpp_code.contains(
                "::new (static_cast<void*>(o)) ::geo::Point(::geo::Point::from< ::int32_t, \
                 ::int32_t >(::rust::__zngur_internal_move_from_rust< ::int32_t >(i0), "
            ),
            "{cpp_code}"
        );
    }
}
