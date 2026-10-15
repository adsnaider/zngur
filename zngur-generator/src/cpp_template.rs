//! The Rust side of C++ class templates: all instantiations of a `c++::` template (e.g.
//! `c++::std::unique_ptr<c++::a::B>` and `c++::std::unique_ptr<i32>`) are one generic Rust struct
//! (`std::unique_ptr<T0>`), so that Rust code names them the way C++ code does.
//!
//! Each instantiation has its own layout and destructor, which a generic struct can't spell out.
//! So the struct gets them from a per-template trait, which each instantiation implements for the
//! tuple of its template arguments:
//!
//! ```ignore
//! pub unsafe trait __ZngurCppTemplate_unique_ptr { type Storage; unsafe fn __zngur_destruct(p: *mut u8); }
//! pub struct unique_ptr<T0> where (T0,): __ZngurCppTemplate_unique_ptr { buffer: ..Storage.. }
//! unsafe impl std::__ZngurCppTemplate_unique_ptr for (a::B,) { type Storage = ...; ... }
//! ```
//!
//! Rust doesn't allow `Drop` (or other trait impls that the struct needs everywhere) for just one
//! instantiation, so those are generic and dispatch through the trait. Everything else (methods,
//! constructors, fields, ...) is implemented on the concrete instantiation, which Rust allows.

use std::collections::HashSet;

use zngur_def::CppTypePath;

use crate::CppModuleTree;
use crate::rust::RustFile;

/// The template path and arguments of `ty`, if it's an instantiation of a C++ class template.
pub fn cpp_template(ty: &zngur_def::RustType) -> Option<&CppTypePath> {
    match ty {
        zngur_def::RustType::Cpp(path) if !path.generics.is_empty() => Some(path),
        _ => None,
    }
}

/// The C++ class templates whose generic Rust struct has been emitted.
#[derive(Default)]
pub struct CppTemplates {
    emitted: HashSet<Vec<String>>,
}

impl CppTemplates {
    /// Adds the instantiation `ty` (and its template's generic struct, the first time). `stack` is
    /// `Some((size, align, destructor))` for instantiations that Rust holds by value, where
    /// `destructor` is the (mangled) name of the C++ function that destroys one, and `None` for
    /// reference-only ones.
    pub fn add_instantiation(
        &mut self,
        rust_file: &mut RustFile,
        cpp_tree: &mut CppModuleTree,
        ty: &CppTypePath,
        stack: Option<(usize, usize, &str)>,
    ) {
        let (name, module_path) = ty
            .path
            .split_last()
            .expect("Cpp path must have at least one segment");
        let arity = ty.generics.len();
        let params = (0..arity).map(|n| format!("T{n}")).collect::<Vec<_>>();
        let params_tuple = format!("({},)", params.join(", "));
        let params = params.join(", ");
        let trait_name = format!("__ZngurCppTemplate_{name}");
        if self.emitted.insert(ty.path.clone()) {
            cpp_tree.insert(
                module_path,
                &format!(
                    r#"
    /// The layout and destructor of each instantiation of the C++ class template `{name}`.
    #[doc(hidden)]
    #[allow(non_camel_case_types)]
    pub unsafe trait {trait_name} {{
        type Storage;
        unsafe fn __zngur_destruct(this: *mut u8);
    }}

    #[repr(C)]
    #[allow(non_camel_case_types)]
    pub struct {name}<{params}> where {params_tuple}: {trait_name} {{
        pub(crate) buffer: ::core::cell::UnsafeCell<::core::mem::MaybeUninit<<{params_tuple} as {trait_name}>::Storage>>,
        _no_auto_traits: ::core::marker::PhantomData<*mut ()>,
        _pinned: ::core::marker::PhantomPinned,
        _args: ::core::marker::PhantomData<{params_tuple}>,
    }}

    unsafe impl<{params}> ::zngur_lib::ZngCppObject for {name}<{params}> where {params_tuple}: {trait_name} {{}}

    impl<{params}> Drop for {name}<{params}> where {params_tuple}: {trait_name} {{
        fn drop(&mut self) {{
            unsafe {{ <{params_tuple} as {trait_name}>::__zngur_destruct(self.buffer.get() as *mut u8) }}
        }}
    }}
"#
                ),
            );
        }

        // Relative to the generated code's root, where the per-instantiation items go (that's
        // where the paths of the template arguments resolve).
        let trait_path = module_path
            .iter()
            .chain([&trait_name])
            .cloned()
            .collect::<Vec<_>>()
            .join("::");
        let args_tuple = format!(
            "({},)",
            ty.generics
                .iter()
                .map(|g| g.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        );
        let rust_ty = zngur_def::RustType::Cpp(ty.clone());
        let storage = rust_file.mangle_name(&format!("{rust_ty}_storage"));
        let (storage_def, destruct, stack_impls) = match stack {
            Some((size, align, destructor)) => (
                format!("#[repr(C, align({align}))]\npub struct {storage}([u8; {size}]);"),
                format!(
                    r#"unsafe extern "C" {{ fn {destructor}(i0: *mut u8, o: *mut u8); }}
        let mut dummy = ();
        unsafe {{ {destructor}(this, &mut dummy as *mut () as *mut u8) }}"#
                ),
                format!(
                    r#"
unsafe impl ::zngur_lib::ZngCppStackObject for {rust_ty} {{}}

unsafe impl ::zngur_lib::ZngCppDestruct for {rust_ty} {{
    unsafe fn destruct(&mut self) {{
        unsafe {{ <{args_tuple} as {trait_path}>::__zngur_destruct(self.buffer.get() as *mut u8) }}
    }}
}}

impl {rust_ty} {{
    /// A C++ pointer (`const T*`) to this object.
    #[allow(dead_code, clippy::should_implement_trait)]
    pub fn as_ref(&self) -> ::zngur_lib::CppPtr<Self, ::zngur_lib::Const> {{
        ::zngur_lib::CppPtr::from(self)
    }}

    /// A C++ pointer (`T*`) to this object.
    #[allow(dead_code, clippy::should_implement_trait)]
    pub fn as_mut(&mut self) -> ::zngur_lib::CppPtr<Self, ::zngur_lib::Mut> {{
        ::zngur_lib::CppPtr::from(self)
    }}
}}
"#
                ),
            ),
            // Reference-only instantiations are never owned by Rust, so never destroyed by it.
            None => (
                format!("pub struct {storage}([u8; 0]);"),
                "let _ = this;".to_owned(),
                String::new(),
            ),
        };
        rust_file.text.push_str(&format!(
            r#"
#[doc(hidden)]
#[allow(non_camel_case_types)]
{storage_def}

unsafe impl {trait_path} for {args_tuple} {{
    type Storage = {storage};
    unsafe fn __zngur_destruct(this: *mut u8) {{
        {destruct}
    }}
}}
{stack_impls}"#
        ));
    }
}
