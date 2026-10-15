use std::fmt::Display;

use indexmap::IndexMap;
use itertools::Itertools;

#[cfg(feature = "printing")]
pub mod printing;

mod merge;
pub use merge::{ConflictSource, Merge, MergeFailure, MergeResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mutability {
    Mut,
    Not,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ZngurMethodReceiver {
    Static,
    Ref(Mutability),
    Move,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ZngurMethod {
    pub name: String,
    pub generics: Vec<RustType>,
    pub receiver: ZngurMethodReceiver,
    pub inputs: Vec<RustType>,
    pub output: RustType,
    pub is_safe: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ZngurFn {
    pub path: RustPathAndGenerics,
    pub inputs: Vec<RustType>,
    pub output: RustType,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ZngurExternCppFn {
    pub name: String,
    pub inputs: Vec<RustType>,
    pub output: RustType,
    pub is_safe: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ZngurExternCppImpl {
    pub tr: Option<RustTrait>,
    pub ty: RustType,
    pub methods: Vec<ZngurMethod>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ZngurConstructor {
    pub inputs: Vec<(String, RustType)>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ZngurVariant {
    pub name: String,
    pub fields: Vec<ZngurField>,
    pub exhaustive: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ZngurField {
    pub name: String,
    pub ty: RustType,
    pub offset: Option<usize>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ZngurFieldData {
    pub name: String,
    pub ty: RustType,
    pub offset: ZngurFieldDataOffset,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ZngurFieldDataOffset {
    Offset(usize),
    Auto(String),
    AutoDynamic(String),
}

impl ZngurFieldDataOffset {
    pub fn is_offset(&self) -> bool {
        matches!(self, ZngurFieldDataOffset::Offset(_))
    }

    pub fn as_offset(&self) -> Option<usize> {
        match self {
            ZngurFieldDataOffset::Offset(o) => Some(*o),
            _ => None,
        }
    }

    pub fn as_auto(&self) -> Option<&str> {
        match self {
            ZngurFieldDataOffset::Auto(s) => Some(s),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ZngurWellknownTrait {
    Debug,
    Drop,
    Unsized,
    Copy,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ZngurWellknownTraitData {
    Debug {
        pretty_print: String,
        debug_print: String,
    },
    Drop {
        drop_in_place: String,
    },
    Unsized,
    Copy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutPolicy {
    StackAllocated { size: usize, align: usize },
    Conservative { size: usize, align: usize },
    HeapAllocated,
    OnlyByRef,
}

impl LayoutPolicy {
    pub const ZERO_SIZED_TYPE: Self = LayoutPolicy::StackAllocated { size: 0, align: 1 };
}

#[derive(Debug, PartialEq, Eq)]
pub struct ZngurMethodDetails {
    pub data: ZngurMethod,
    pub use_path: Option<String>,
    pub deref: Option<(RustType, Mutability)>,
    pub cpp_name: Option<String>,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct CppHeapAllocated(pub String);

#[derive(Debug)]
pub struct CppHeapAllocatedData {
    pub bridge_fn: String,
    pub cpp_type: String,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct CppRef(pub String);

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct CppStackOwned(pub String);

impl Display for CppStackOwned {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Display for CppRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, PartialEq)]
pub struct ZngurType {
    pub ty: RustType,
    pub layout: Option<LayoutPolicy>,
    pub wellknown_traits: Vec<ZngurWellknownTrait>,
    pub exhaustive: bool,
    pub methods: Vec<ZngurMethodDetails>,
    /// The C++ methods of a C++ type (`#cpp_stack_owned` or `#cpp_ref`), which Rust can call.
    pub cpp_methods: Vec<ZngurCppMethod>,
    /// The C++ constructors of a `#cpp_stack_owned` type, which Rust can call.
    pub cpp_constructors: Vec<ZngurCppConstructor>,
    /// Accessors for the fields (data members) of a C++ type (`#cpp_stack_owned` or `#cpp_ref`).
    pub cpp_fields: Vec<ZngurCppField>,
    pub constructor: Option<ZngurConstructor>,
    pub variants: Vec<ZngurVariant>,
    pub fields: Vec<ZngurField>,
    pub cpp_heap_allocated: Option<CppHeapAllocated>,
    pub cpp_ref: Option<CppRef>,
    pub cpp_stack_owned: Option<CppStackOwned>,
}

/// A C++ member function of a C++ type, declared C++-style in the type's block, e.g.
/// `unsafe fn value() const -> i32;` or `safe static fn make(i32) -> c++::ns::Bar;`. zngur
/// generates a Rust function for it that calls the C++ member function directly.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct ZngurCppMethod {
    /// The name of the C++ member function.
    pub cpp_name: String,
    /// The name of the Rust function (from `as`), if it isn't `cpp_name`.
    pub rust_name: Option<String>,
    /// `Static` for static member functions, `Ref(Not)` for `const` member functions, and
    /// `Ref(Mut)` for the others. The Rust function takes the object as its first argument, as a
    /// `zngur_lib::CppPtr<Self, Const or Mut>`.
    pub receiver: ZngurMethodReceiver,
    pub inputs: Vec<RustType>,
    pub output: RustType,
    pub is_safe: bool,
}

impl ZngurCppMethod {
    /// The name of the Rust function.
    pub fn rust_name(&self) -> &str {
        self.rust_name.as_deref().unwrap_or(&self.cpp_name)
    }

    /// The Rust function's signature, as a method of the Rust wrapper (for diagnostics).
    pub fn as_rust_method(&self) -> ZngurMethod {
        ZngurMethod {
            name: self.rust_name().to_owned(),
            generics: vec![],
            receiver: self.receiver,
            inputs: self.inputs.clone(),
            output: self.output.clone(),
            is_safe: self.is_safe,
        }
    }
}

/// A C++ constructor of a `#cpp_stack_owned` type, declared like `safe constructor(i32);`. zngur
/// implements `zngur_lib::ZngCppConstruct<(inputs...,)>` for it, which runs it in place.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct ZngurCppConstructor {
    pub inputs: Vec<RustType>,
    pub is_safe: bool,
}

/// An accessor for a field (data member) of a C++ type, declared like `field const x (type = i32);`
/// or `field x (type = i32) as x_mut;`. zngur generates an `unsafe` Rust function that takes `this`
/// as a `zngur_lib::CppPtr<Self, M>` and returns a `CppPtr<ty, M>` to the field, where `M` is
/// `Const` for `const` accessors and `Mut` otherwise.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct ZngurCppField {
    /// The name of the C++ field.
    pub name: String,
    /// The name of the Rust function (from `as`), if it isn't `name`.
    pub rust_name: Option<String>,
    /// The type of the field.
    pub ty: RustType,
    /// `Not` for `const` accessors.
    pub mutability: Mutability,
}

impl ZngurCppField {
    /// The name of the Rust function.
    pub fn rust_name(&self) -> &str {
        self.rust_name.as_deref().unwrap_or(&self.name)
    }

    /// The accessor as a C++ member function, which returns a pointer to the field.
    pub fn as_cpp_method(&self) -> ZngurCppMethod {
        ZngurCppMethod {
            cpp_name: self.name.clone(),
            rust_name: self.rust_name.clone(),
            receiver: ZngurMethodReceiver::Ref(self.mutability),
            inputs: vec![],
            output: RustType::CppPtr(
                self.mutability,
                CppIndirection::Pointer,
                Box::new(self.ty.clone()),
            ),
            is_safe: false,
        }
    }
}

/// A free C++ function, declared in a `c++::` scope like
/// `mod c++::ns { unsafe fn f(const c++::ns::Bar&) -> i32; }`. zngur generates a Rust function
/// for it, at the same path in the `c++::` tree as the C++ function's namespace, which calls it.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct ZngurCppFn {
    /// The C++ namespace (`["ns"]` for `::ns::f`). Never empty.
    pub namespace: Vec<String>,
    /// The name of the C++ function.
    pub cpp_name: String,
    /// The name of the Rust function (from `as`), if it isn't `cpp_name`.
    pub rust_name: Option<String>,
    pub inputs: Vec<RustType>,
    pub output: RustType,
    pub is_safe: bool,
}

impl ZngurCppFn {
    /// The name of the Rust function.
    pub fn rust_name(&self) -> &str {
        self.rust_name.as_deref().unwrap_or(&self.cpp_name)
    }

    /// The qualified C++ name, e.g. `::ns::f`.
    pub fn cpp_path(&self) -> String {
        self.namespace
            .iter()
            .chain([&self.cpp_name])
            .map(|s| format!("::{s}"))
            .collect()
    }
}

#[derive(Debug, PartialEq)]
pub struct ZngurTrait {
    pub tr: RustTrait,
    pub methods: Vec<ZngurMethod>,
}

#[derive(Debug, Default)]
pub struct AdditionalIncludes(pub String);

#[derive(Debug, Default)]
pub struct ConvertPanicToException(pub bool);

#[derive(Clone, Debug, Default)]
pub struct Import(pub std::path::PathBuf);

#[derive(Debug, Clone)]
pub struct ModuleImport {
    pub path: std::path::PathBuf,
    /// The `#crate` of the imported module, if it has one.
    pub crate_name: Option<String>,
    /// The `c++::` types that the imported module declares (their paths, without `c++::`). Their
    /// Rust wrappers live at `::<crate_name>::generated::<path>`.
    pub cpp_types: Vec<Vec<String>>,
}

#[derive(Debug, Default)]
pub struct ZngurSpec {
    pub imported_modules: Vec<ModuleImport>,
    pub types: Vec<ZngurType>,
    pub traits: IndexMap<RustTrait, ZngurTrait>,
    pub funcs: Vec<ZngurFn>,
    /// Free C++ functions that Rust can call, declared in `c++::` scopes.
    pub cpp_funcs: Vec<ZngurCppFn>,
    pub extern_cpp_funcs: Vec<ZngurExternCppFn>,
    pub extern_cpp_impls: Vec<ZngurExternCppImpl>,
    pub additional_includes: AdditionalIncludes,
    pub convert_panic_to_exception: ConvertPanicToException,
    pub cpp_include_header_name: String,
    pub mangling_base: String,
    pub cpp_namespace: Option<String>,
    /// The Rust crate that the generated Rust code is part of, from `#crate <name>`. Other modules
    /// that import this one find its `c++::` types at `::<crate_name>::generated::<path>`.
    pub crate_name: Option<String>,
    pub rust_cfg: Vec<(String, Option<String>)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RustTrait {
    Normal(RustPathAndGenerics),
    Fn {
        name: String,
        inputs: Vec<RustType>,
        output: Box<RustType>,
    },
}

impl RustTrait {
    pub fn take_assocs(mut self) -> (Self, Vec<(String, RustType)>) {
        let assocs = match &mut self {
            RustTrait::Normal(p) => std::mem::take(&mut p.named_generics),
            RustTrait::Fn { .. } => vec![],
        };
        (self, assocs)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PrimitiveRustType {
    Uint(u32),
    Int(u32),
    Float(u32),
    Usize,
    Bool,
    Char,
    Str,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TypeVar(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RustPathAndGenerics {
    pub path: Vec<String>,
    pub generics: Vec<RustType>,
    pub named_generics: Vec<(String, RustType)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RustType {
    Primitive(PrimitiveRustType),
    Ref(Mutability, Box<RustType>),
    Raw(Mutability, Box<RustType>),
    Boxed(Box<RustType>),
    Slice(Box<RustType>),
    Dyn(RustTrait, Vec<String>),
    Impl(RustTrait, Vec<String>),
    Tuple(Vec<RustType>),
    Adt(RustPathAndGenerics),
    TypeVar(TypeVar),
    Cpp(Vec<String>),
    /// A C++ pointer or reference (`const T*`, `T*`, `const T&` or `T&` in the `.zng` file). In
    /// Rust, all of them are a `zngur_lib::CppPtr<T, Const or Mut>`. The indirection only matters
    /// when calling C++ functions, which take either a pointer or a reference.
    CppPtr(Mutability, CppIndirection, Box<RustType>),
}

/// Whether a C++ function takes (or returns) a [`RustType::CppPtr`] as a pointer or a reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CppIndirection {
    Pointer,
    Reference,
}

impl RustType {
    pub const UNIT: Self = RustType::Tuple(Vec::new());
}

impl Display for RustPathAndGenerics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let RustPathAndGenerics {
            path,
            generics,
            named_generics,
        } = self;
        for p in path {
            if p != "crate" {
                write!(f, "::")?;
            }
            write!(f, "{p}")?;
        }
        if !generics.is_empty() || !named_generics.is_empty() {
            write!(
                f,
                "::<{}>",
                generics
                    .iter()
                    .map(|x| format!("{x}"))
                    .chain(named_generics.iter().map(|x| format!("{} = {}", x.0, x.1)))
                    .join(", ")
            )?;
        }
        Ok(())
    }
}

impl Display for RustTrait {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RustTrait::Normal(tr) => write!(f, "{tr}"),
            RustTrait::Fn {
                name,
                inputs,
                output,
            } => {
                write!(f, "{name}({})", inputs.iter().join(", "))?;
                if **output != RustType::UNIT {
                    write!(f, " -> {output}")?;
                }
                Ok(())
            }
        }
    }
}

impl Display for PrimitiveRustType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PrimitiveRustType::Uint(s) => write!(f, "u{s}"),
            PrimitiveRustType::Int(s) => write!(f, "i{s}"),
            PrimitiveRustType::Float(s) => write!(f, "f{s}"),
            PrimitiveRustType::Usize => write!(f, "usize"),
            PrimitiveRustType::Bool => write!(f, "bool"),
            PrimitiveRustType::Char => write!(f, "char"),
            PrimitiveRustType::Str => write!(f, "str"),
        }
    }
}

impl Display for RustType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RustType::Primitive(s) => write!(f, "{s}"),
            RustType::Ref(Mutability::Not, ty) => write!(f, "&{ty}"),
            RustType::Ref(Mutability::Mut, ty) => write!(f, "&mut {ty}"),
            RustType::Raw(Mutability::Not, ty) => write!(f, "*const {ty}"),
            RustType::Raw(Mutability::Mut, ty) => write!(f, "*mut {ty}"),
            RustType::Boxed(ty) => write!(f, "Box<{ty}>"),
            RustType::Tuple(v) => write!(f, "({})", v.iter().join(", ")),
            RustType::Adt(pg) => write!(f, "{pg}"),
            RustType::Dyn(tr, marker_bounds) => {
                write!(f, "dyn {tr}")?;
                for mb in marker_bounds {
                    write!(f, "+ {mb}")?;
                }
                Ok(())
            }
            RustType::Impl(tr, marker_bounds) => {
                write!(f, "impl {tr}")?;
                for mb in marker_bounds {
                    write!(f, "+ {mb}")?;
                }
                Ok(())
            }
            RustType::Slice(s) => write!(f, "[{s}]"),
            RustType::TypeVar(TypeVar(v)) => write!(f, "{v}"),
            RustType::Cpp(segs) => write!(f, "{}", segs.join("::")),
            RustType::CppPtr(m, _, ty) => {
                let constness = match m {
                    Mutability::Not => "Const",
                    Mutability::Mut => "Mut",
                };
                write!(f, "::zngur_lib::CppPtr<{ty}, ::zngur_lib::{constness}>")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cpp_ptr_display() {
        let ty = RustType::CppPtr(
            Mutability::Not,
            CppIndirection::Reference,
            Box::new(RustType::Cpp(vec!["a".to_owned(), "Name".to_owned()])),
        );
        assert_eq!(
            ty.to_string(),
            "::zngur_lib::CppPtr<a::Name, ::zngur_lib::Const>"
        );
    }

    #[test]
    fn test_cpp_display() {
        let cpp = RustType::Cpp(vec!["a".to_owned(), "b".to_owned(), "Name".to_owned()]);
        assert_eq!(cpp.to_string(), "a::b::Name");
    }
}
