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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZngurConstructor {
    pub inputs: Vec<(String, RustType)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZngurVariant {
    pub name: String,
    pub fields: Vec<ZngurField>,
    pub exhaustive: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZngurMethodDetails {
    pub data: ZngurMethod,
    pub use_path: Option<String>,
    pub deref: Option<(RustType, Mutability)>,
    pub cpp_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ZngurType {
    pub ty: RustType,
    /// Required for Rust types. For C++ (`c++::`) types, Rust holds them by value if they have a
    /// `StackAllocated` or `Conservative` layout and are `trivially_relocatable`, and only refers
    /// to them otherwise.
    pub layout: Option<LayoutPolicy>,
    /// Whether the C++ type can be moved with `memcpy`, so that Rust can hold it by value.
    pub trivially_relocatable: TriviallyRelocatable,
    pub wellknown_traits: Vec<ZngurWellknownTrait>,
    pub exhaustive: bool,
    pub methods: Vec<ZngurMethodDetails>,
    /// The C++ member functions of a C++ type, which Rust can call.
    pub cpp_methods: Vec<ZngurCppMethod>,
    /// The C++ constructors of a C++ type that Rust holds by value, which Rust can call.
    pub cpp_constructors: Vec<ZngurCppConstructor>,
    /// Accessors for the fields (data members) of a C++ type.
    pub cpp_fields: Vec<ZngurCppField>,
    pub constructor: Option<ZngurConstructor>,
    pub variants: Vec<ZngurVariant>,
    pub fields: Vec<ZngurField>,
}

impl ZngurType {
    /// The C++ name of a C++ (`c++::`) type, e.g. `::std::unique_ptr< ::a::B >`.
    pub fn cpp_name(&self) -> Option<String> {
        match &self.ty {
            RustType::Cpp(_) => self.ty.cpp_spelling().ok(),
            _ => None,
        }
    }

    /// The size and alignment of a C++ type that Rust holds by value (see [`ZngurType::layout`]).
    pub fn cpp_by_value_layout(&self) -> Option<LayoutPolicy> {
        match (&self.ty, self.layout) {
            (
                RustType::Cpp(_),
                Some(
                    layout @ (LayoutPolicy::StackAllocated { .. }
                    | LayoutPolicy::Conservative { .. }),
                ),
            ) if self.trivially_relocatable.is_relocatable() => Some(layout),
            _ => None,
        }
    }
}

/// Whether a C++ (`c++::`) type can be moved with `memcpy` (without running its move constructor
/// and destructor), which Rust needs to hold it by value.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TriviallyRelocatable {
    /// No directive: Rust only refers to the type.
    #[default]
    No,
    /// `#trivially_relocatable`. The generated C++ code checks it with
    /// `rust::is_trivially_relocatable`, which is true for trivially copyable types and
    /// `std::unique_ptr`, and which C++ code can specialize for other types.
    Checked,
    /// `#unsafe(trivially_relocatable)`. The author promises it, and the generated C++ code
    /// doesn't check it, e.g. for `[[clang::trivial_abi]]` types or when a tool (like a bindings
    /// generator) knows it.
    Unchecked,
}

impl TriviallyRelocatable {
    /// Whether the type is trivially relocatable (checked or not).
    pub fn is_relocatable(self) -> bool {
        self != TriviallyRelocatable::No
    }
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

/// A C++ constructor of a C++ type that Rust holds by value, declared like `safe constructor(i32);`. zngur
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
    /// The `c++::` types that the imported module declares (their paths, without `c++::`), other
    /// than instantiations of C++ class templates. Their Rust wrappers live at
    /// `::<crate_name>::generated::<path>`.
    pub cpp_types: Vec<Vec<String>>,
    /// The C++ class templates whose home is the imported module (see
    /// [`ZngurSpec::cpp_template_homes`]).
    pub cpp_template_homes: Vec<CppTemplateHome>,
}

/// A C++ class template whose home is a module: the module that defines the Rust alias for its
/// instantiations (`pub type unique_ptr<T> = <T as __ZngurCppTemplate_unique_ptr>::Type;`), and
/// the trait through which each instantiation names its Rust struct.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CppTemplateHome {
    /// The path of the template, without `c++::` (e.g. `["std", "unique_ptr"]`).
    pub path: Vec<String>,
    /// The number of template arguments.
    pub arity: usize,
    /// Whether the home declares a template (`type<T> c++::...`) for it, rather than just being
    /// the only module that instantiates it. Only explicit homes can be shared: other modules that
    /// import them can declare instantiations of the template too.
    pub explicit: bool,
}

#[derive(Debug, Default)]
pub struct ZngurSpec {
    pub imported_modules: Vec<ModuleImport>,
    pub types: Vec<ZngurType>,
    /// Templates of C++ class templates (`type<T> c++::std::unique_ptr<T> { ... }`) declared in
    /// this module. Their members are defaults for every instantiation, including those in modules
    /// that import this one, and this module is the template's home.
    pub cpp_templates: Vec<ZngurType>,
    /// The C++ class templates whose home is this module: those it declares a template for, and
    /// those it instantiates without a template being declared in it or a module it imports.
    /// Computed by the parser.
    pub cpp_template_homes: Vec<CppTemplateHome>,
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
    /// A C++ type, declared with a `c++::` path. Its C++ name is the path, e.g. `c++::a::B` is
    /// `::a::B`, and `c++::std::unique_ptr<c++::a::B>` is `::std::unique_ptr<::a::B>`.
    Cpp(CppTypePath),
    /// A C++ pointer or reference (`const T*`, `T*`, `const T&` or `T&` in the `.zng` file). In
    /// Rust, all of them are a `zngur_lib::CppPtr<T, Const or Mut>`. The indirection only matters
    /// when calling C++ functions, which take either a pointer or a reference.
    CppPtr(Mutability, CppIndirection, Box<RustType>),
}

/// The path of a [`RustType::Cpp`] type, without the `c++::` prefix, and its template arguments.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CppTypePath {
    pub path: Vec<String>,
    /// The template arguments: other C++ types, builtins and C++ pointers.
    pub generics: Vec<RustType>,
}

impl From<Vec<String>> for CppTypePath {
    fn from(path: Vec<String>) -> Self {
        CppTypePath {
            path,
            generics: vec![],
        }
    }
}

impl Display for CppTypePath {
    /// The Rust path of the wrapper, relative to the generated code's root (e.g.
    /// `std::unique_ptr::<a::B>`).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.path.join("::"))?;
        if !self.generics.is_empty() {
            write!(f, "::<{}>", self.generics.iter().join(", "))?;
        }
        Ok(())
    }
}

/// Whether a C++ function takes (or returns) a [`RustType::CppPtr`] as a pointer or a reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CppIndirection {
    Pointer,
    Reference,
}

impl RustType {
    pub const UNIT: Self = RustType::Tuple(Vec::new());

    /// The type variables in `self` (e.g. `T` in the template `c++::std::unique_ptr<T>`), in order
    /// of appearance, without duplicates.
    pub fn type_vars(&self) -> Vec<&TypeVar> {
        fn collect<'a>(ty: &'a RustType, out: &mut Vec<&'a TypeVar>) {
            let all = |tys: &'a [RustType], out: &mut Vec<&'a TypeVar>| {
                tys.iter().for_each(|t| collect(t, out))
            };
            match ty {
                RustType::TypeVar(v) => {
                    if !out.contains(&v) {
                        out.push(v);
                    }
                }
                RustType::Ref(_, t)
                | RustType::Raw(_, t)
                | RustType::Boxed(t)
                | RustType::Slice(t)
                | RustType::CppPtr(_, _, t) => collect(t, out),
                RustType::Tuple(tys) => all(tys, out),
                RustType::Adt(pg) => {
                    all(&pg.generics, out);
                    pg.named_generics.iter().for_each(|(_, t)| collect(t, out));
                }
                RustType::Cpp(path) => all(&path.generics, out),
                RustType::Primitive(_) | RustType::Dyn(..) | RustType::Impl(..) => {}
            }
        }
        let mut out = vec![];
        collect(self, &mut out);
        out
    }

    /// The C++ spelling of a `c++::` type or of one of its template arguments, e.g. `::a::B`,
    /// `::std::unique_ptr< ::a::B >` or `const int32_t*`. `Err` explains why `self` can't be a
    /// C++ template argument.
    pub fn cpp_spelling(&self) -> Result<String, String> {
        Ok(match self {
            RustType::Cpp(path) => {
                let name = format!("::{}", path.path.join("::"));
                if path.generics.is_empty() {
                    name
                } else {
                    let args = path
                        .generics
                        .iter()
                        .map(RustType::cpp_spelling)
                        .collect::<Result<Vec<_>, _>>()?;
                    // Spaces, so that `<::` and `>>` are never digraphs or shifts, even in C++03.
                    format!("{name}< {} >", args.join(", "))
                }
            }
            RustType::Primitive(p) => match p {
                PrimitiveRustType::Int(n) => format!("::int{n}_t"),
                PrimitiveRustType::Uint(n) => format!("::uint{n}_t"),
                PrimitiveRustType::Float(32) => "float".to_owned(),
                PrimitiveRustType::Float(64) => "double".to_owned(),
                PrimitiveRustType::Usize => "::size_t".to_owned(),
                PrimitiveRustType::Bool => "bool".to_owned(),
                p => return Err(format!("`{p}` has no C++ equivalent")),
            },
            RustType::CppPtr(m, indirection, t) => {
                let constness = match m {
                    Mutability::Not => "const ",
                    Mutability::Mut => "",
                };
                let sigil = match indirection {
                    CppIndirection::Pointer => "*",
                    CppIndirection::Reference => "&",
                };
                format!("{constness}{}{sigil}", t.cpp_spelling()?)
            }
            ty => {
                return Err(format!(
                    "`{ty}` can't be a C++ template argument (only `c++::` types, builtins and \
                     C++ pointers and references can)"
                ));
            }
        })
    }
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
            RustType::Cpp(path) => write!(f, "{path}"),
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
            Box::new(RustType::Cpp(
                vec!["a".to_owned(), "Name".to_owned()].into(),
            )),
        );
        assert_eq!(
            ty.to_string(),
            "::zngur_lib::CppPtr<a::Name, ::zngur_lib::Const>"
        );
    }

    #[test]
    fn test_cpp_display() {
        let cpp = RustType::Cpp(vec!["a".to_owned(), "b".to_owned(), "Name".to_owned()].into());
        assert_eq!(cpp.to_string(), "a::b::Name");
        let generic = RustType::Cpp(CppTypePath {
            path: vec!["std".to_owned(), "unique_ptr".to_owned()],
            generics: vec![cpp, RustType::Primitive(PrimitiveRustType::Int(32))],
        });
        assert_eq!(generic.to_string(), "std::unique_ptr::<a::b::Name, i32>");
    }
}
