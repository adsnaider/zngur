use std::fmt::Display;

use indexmap::IndexMap;
use itertools::Itertools;

/// Keywords listed at <https://en.cppreference.com/w/cpp/keyword>.
pub const CPP_KEYWORDS: &[&str] = &[
    "alignas",
    "alignof",
    "and",
    "and_eq",
    "asm",
    "atomic_cancel",
    "atomic_commit",
    "atomic_noexcept",
    "auto",
    "bitand",
    "bitor",
    "bool",
    "break",
    "case",
    "catch",
    "char",
    "char8_t",
    "char16_t",
    "char32_t",
    "class",
    "compl",
    "concept",
    "const",
    "consteval",
    "constexpr",
    "constinit",
    "const_cast",
    "continue",
    "contract_assert",
    "co_await",
    "co_return",
    "co_yield",
    "decltype",
    "default",
    "delete",
    "do",
    "double",
    "dynamic_cast",
    "else",
    "enum",
    "explicit",
    "export",
    "extern",
    "false",
    "float",
    "for",
    "friend",
    "goto",
    "if",
    "inline",
    "int",
    "long",
    "mutable",
    "namespace",
    "new",
    "noexcept",
    "not",
    "not_eq",
    "nullptr",
    "operator",
    "or",
    "or_eq",
    "private",
    "protected",
    "public",
    "reflexpr",
    "register",
    "reinterpret_cast",
    "requires",
    "return",
    "short",
    "signed",
    "sizeof",
    "static",
    "static_assert",
    "static_cast",
    "struct",
    "switch",
    "synchronized",
    "template",
    "this",
    "thread_local",
    "throw",
    "true",
    "try",
    "typedef",
    "typeid",
    "typename",
    "union",
    "unsigned",
    "using",
    "virtual",
    "void",
    "volatile",
    "wchar_t",
    "while",
    "xor",
    "xor_eq",
];

pub fn cpp_handle_keyword(name: &str) -> String {
    if CPP_KEYWORDS.contains(&name) {
        format!("{name}_")
    } else {
        name.to_owned()
    }
}

pub fn cpp_handle_field_name(name: &str) -> String {
    if name.parse::<u32>().is_ok() {
        return format!("f{name}");
    }
    cpp_handle_keyword(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpp_keywords_are_escaped() {
        for keyword in CPP_KEYWORDS {
            assert_eq!(cpp_handle_keyword(keyword), format!("{keyword}_"));
            assert_eq!(cpp_handle_field_name(keyword), format!("{keyword}_"));
        }

        assert_eq!(cpp_handle_keyword("ordinary_name"), "ordinary_name");
    }

    #[test]
    fn test_generic_const_input() {
        let a_b = RustType::Cpp(vec!["a".to_owned(), "B".to_owned()].into());
        let i32 = RustType::Primitive(PrimitiveRustType::Int(32));
        let lt = Some(Lifetime("a".to_owned()));
        let const_ref = |m, l: Option<Lifetime>, t: &RustType| {
            RustType::Ref(m, l, Box::new(RustType::CppConst(Box::new(t.clone()))))
        };
        let inputs = [
            const_ref(Mutability::Not, lt.clone(), &a_b),
            const_ref(Mutability::Not, None, &a_b),
            const_ref(Mutability::Mut, lt.clone(), &i32),
            RustType::cpp_ptr(Mutability::Not, CppIndirection::Reference, a_b.clone()),
            RustType::cpp_ptr(Mutability::Not, CppIndirection::Pointer, i32.clone()),
            RustType::cpp_ptr(Mutability::Mut, CppIndirection::Pointer, a_b.clone()),
            // Multi-level pointers: only top-level pointee constness (`i32* const*`) is generic;
            // `const i32**` stays fixed (avoiding `T** -> const T**` unsoundness).
            RustType::cpp_ptr(
                Mutability::Mut,
                CppIndirection::Pointer,
                RustType::cpp_ptr(Mutability::Not, CppIndirection::Pointer, i32.clone()),
            ),
            RustType::cpp_ptr(
                Mutability::Not,
                CppIndirection::Pointer,
                RustType::cpp_ptr(Mutability::Mut, CppIndirection::Pointer, i32.clone()),
            ),
            RustType::Ref(
                Mutability::Not,
                lt.clone(),
                Box::new(RustType::CppVar(Box::new(a_b.clone()))),
            ),
            RustType::Ref(Mutability::Not, lt.clone(), Box::new(i32.clone())),
        ];
        let (params, inputs) = cpp_fn_inputs(&[Lifetime("a".to_owned())], &inputs);
        assert_eq!(
            params,
            "<'a, __C1: ::zngur_lib::Constness, __C2: ::zngur_lib::Constness, \
             __C3: ::zngur_lib::Constness, __C4: ::zngur_lib::Constness, \
             __C5: ::zngur_lib::Constness, __C6: ::zngur_lib::Constness>"
        );
        assert_eq!(
            inputs,
            [
                "&'a a::B::<__C1>",
                "&a::B::<__C2>",
                "&'a mut ::zngur_lib::Cpp<i32, __C3>",
                "*const a::B::<__C4>",
                "*const ::zngur_lib::Cpp<i32, __C5>",
                "*mut a::B",
                "*mut ::zngur_lib::Cpp<*const ::zngur_lib::Cpp<i32, ::zngur_lib::Const>>",
                "*const ::zngur_lib::Cpp<*mut ::zngur_lib::Cpp<i32>, __C6>",
                "&'a a::B",
                "&'a i32",
            ]
        );
        assert_eq!(
            cpp_fn_inputs(&[], &[i32]),
            (String::new(), vec!["i32".to_owned()])
        );
    }

    #[test]
    fn test_nullable_cpp_refs() {
        let a_b = RustType::Cpp(vec!["a".to_owned(), "B".to_owned()].into());
        let lt = Some(Lifetime("a".to_owned()));
        let option = |root: &str, m, constness| {
            RustType::Adt(RustPathAndGenerics {
                path: vec![root.to_owned(), "option".to_owned(), "Option".to_owned()],
                generics: vec![RustType::Ref(
                    m,
                    lt.clone(),
                    Box::new(RustType::cpp_object(constness, a_b.clone())),
                )],
                ..Default::default()
            })
        };
        let const_b = option("std", Mutability::Not, CppConstness::Const);
        let unique_const_b = option("core", Mutability::Mut, CppConstness::Const);
        let unique_var_b = option("core", Mutability::Mut, CppConstness::Var);
        assert_eq!(
            const_b.as_nullable_cpp_ref(),
            Some((
                Mutability::Not,
                lt.as_ref(),
                &RustType::CppConst(Box::new(a_b.clone()))
            ))
        );
        assert_eq!(
            unique_var_b.as_nullable_cpp_ref(),
            Some((
                Mutability::Mut,
                lt.as_ref(),
                &RustType::CppVar(Box::new(a_b.clone()))
            ))
        );
        // A C++ pointer in C++ functions' signatures.
        assert_eq!(
            const_b.cpp_bridge_type(),
            RustType::cpp_ptr(Mutability::Not, CppIndirection::Pointer, a_b.clone())
        );
        assert_eq!(
            unique_var_b.cpp_bridge_type(),
            RustType::cpp_ptr(Mutability::Mut, CppIndirection::Pointer, a_b.clone())
        );
        assert_eq!(
            const_b.generic_const_input("__C1").as_deref(),
            Some("::std::option::Option<&'a a::B::<__C1>>")
        );
        assert_eq!(
            unique_const_b.generic_const_input("__C1").as_deref(),
            Some("::core::option::Option<&'a mut a::B::<__C1>>")
        );
        assert_eq!(
            option("std", Mutability::Not, CppConstness::Var).generic_const_input("__C1"),
            None
        );
        assert_eq!(unique_var_b.generic_const_input("__C1"), None);
        // `Option` with lifetime parameters is invalid and not a nullable C++ reference.
        let with_lifetimes = RustType::Adt(RustPathAndGenerics {
            path: vec!["std".to_owned(), "option".to_owned(), "Option".to_owned()],
            lifetimes: vec![Lifetime("a".to_owned())],
            generics: vec![RustType::Ref(
                Mutability::Not,
                lt.clone(),
                Box::new(RustType::cpp_object(CppConstness::Const, a_b)),
            )],
            ..Default::default()
        });
        assert!(with_lifetimes.as_nullable_cpp_ref().is_none());
        // `Option<&i32>` is a Rust value.
        let rust_ref = RustType::Adt(RustPathAndGenerics {
            path: vec!["std".to_owned(), "option".to_owned(), "Option".to_owned()],
            generics: vec![RustType::Ref(
                Mutability::Not,
                None,
                Box::new(RustType::Primitive(PrimitiveRustType::Int(32))),
            )],
            ..Default::default()
        });
        assert!(rust_ref.as_nullable_cpp_ref().is_none());
    }

    #[test]
    fn test_cpp_ptr_display() {
        let ty = RustType::cpp_ptr(
            Mutability::Not,
            CppIndirection::Reference,
            RustType::Cpp(vec!["a".to_owned(), "Name".to_owned()].into()),
        );
        // C++-style pointers and references are raw pointers.
        assert_eq!(ty.to_string(), "*const a::Name::<::zngur_lib::Const>");
        let ty = RustType::cpp_ptr(
            Mutability::Mut,
            CppIndirection::Reference,
            RustType::Cpp(vec!["a".to_owned(), "Name".to_owned()].into()),
        );
        assert_eq!(ty.to_string(), "*mut a::Name");
        let i32 = RustType::Primitive(PrimitiveRustType::Int(32));
        let ty = RustType::cpp_ptr(Mutability::Mut, CppIndirection::Pointer, i32.clone());
        assert_eq!(ty.to_string(), "*mut ::zngur_lib::Cpp<i32>");
        let const_ptr = RustType::CppConst(Box::new(ty));
        assert_eq!(
            const_ptr.to_string(),
            "::zngur_lib::Cpp<*mut ::zngur_lib::Cpp<i32>, ::zngur_lib::Const>"
        );
        let ty = RustType::cpp_ptr(Mutability::Not, CppIndirection::Pointer, i32.clone());
        assert_eq!(
            ty.to_string(),
            "*const ::zngur_lib::Cpp<i32, ::zngur_lib::Const>"
        );
        // A pointer to a pointer: `const int32_t**`.
        let ty = RustType::cpp_ptr(Mutability::Mut, CppIndirection::Pointer, ty);
        assert_eq!(
            ty.to_string(),
            "*mut ::zngur_lib::Cpp<*const ::zngur_lib::Cpp<i32, ::zngur_lib::Const>>"
        );
    }

    #[test]
    fn test_references_to_cpp_objects() {
        let name = RustType::Cpp(vec!["a".to_owned(), "Name".to_owned()].into());
        let i32 = RustType::Primitive(PrimitiveRustType::Int(32));
        let lifetime = Some(Lifetime("a".to_owned()));
        let reference = |m, constness, ty| {
            RustType::Ref(
                m,
                lifetime.clone(),
                Box::new(RustType::cpp_object(constness, ty)),
            )
        };
        let shared_const = reference(Mutability::Not, CppConstness::Const, name.clone());
        assert_eq!(
            shared_const.to_string(),
            "&'a a::Name::<::zngur_lib::Const>"
        );
        assert_eq!(
            shared_const.cpp_bridge_type(),
            RustType::cpp_ptr(Mutability::Not, CppIndirection::Reference, name.clone())
        );
        let shared_var = reference(Mutability::Not, CppConstness::Var, name.clone());
        assert_eq!(shared_var.to_string(), "&'a a::Name");
        assert_eq!(
            shared_var.cpp_bridge_type(),
            RustType::cpp_ptr(Mutability::Mut, CppIndirection::Reference, name.clone())
        );
        let unique_var = reference(Mutability::Mut, CppConstness::Var, name);
        assert_eq!(unique_var.to_string(), "&'a mut a::Name");
        let unique_const = reference(Mutability::Mut, CppConstness::Const, i32.clone());
        assert_eq!(
            unique_const.to_string(),
            "&'a mut ::zngur_lib::Cpp<i32, ::zngur_lib::Const>"
        );
        assert_eq!(
            unique_const.cpp_bridge_type(),
            RustType::cpp_ptr(Mutability::Not, CppIndirection::Reference, i32.clone())
        );
        let builtin_var = reference(Mutability::Not, CppConstness::Var, i32.clone());
        assert_eq!(builtin_var.to_string(), "&'a ::zngur_lib::Cpp<i32>");
        // References to Rust values.
        let rust_value = RustType::Ref(Mutability::Not, None, Box::new(i32.clone()));
        assert_eq!(rust_value.to_string(), "&i32");
        assert_eq!(
            rust_value.cpp_bridge_type(),
            RustType::cpp_ptr(Mutability::Not, CppIndirection::Reference, i32.clone())
        );
        let rust_value = RustType::Ref(Mutability::Mut, None, Box::new(i32.clone()));
        assert_eq!(
            rust_value.cpp_bridge_type(),
            RustType::cpp_ptr(Mutability::Mut, CppIndirection::Reference, i32)
        );
        // Non-builtin Rust references stay `RustType::Ref` (`rust::Ref<T>` / `rust::RefMut<T>`).
        let rust_str = RustType::Ref(
            Mutability::Not,
            None,
            Box::new(RustType::Primitive(PrimitiveRustType::Str)),
        );
        assert_eq!(rust_str.cpp_bridge_type(), rust_str);
        let rust_adt = RustType::Ref(
            Mutability::Not,
            None,
            Box::new(RustType::Adt(RustPathAndGenerics {
                path: vec!["crate".to_owned(), "Foo".to_owned()],
                ..Default::default()
            })),
        );
        assert_eq!(rust_adt.cpp_bridge_type(), rust_adt);
    }

    #[test]
    fn test_cpp_object_normalization() {
        let i32 = RustType::Primitive(PrimitiveRustType::Int(32));
        let c = RustType::cpp_object(CppConstness::Const, i32.clone());
        let v = RustType::cpp_object(CppConstness::Var, i32.clone());
        assert_eq!(c, RustType::CppConst(Box::new(i32.clone())));
        assert_eq!(v, RustType::CppVar(Box::new(i32.clone())));
        assert_eq!(RustType::cpp_object(CppConstness::Var, c.clone()), c);
        assert_eq!(RustType::cpp_object(CppConstness::Const, c.clone()), c);
        assert_eq!(RustType::cpp_object(CppConstness::Const, v.clone()), c);
        assert_eq!(RustType::cpp_object(CppConstness::Var, v.clone()), v);
        assert_eq!(c.as_cpp_object(), Some((CppConstness::Const, &i32)));
    }

    #[test]
    fn test_cpp_field_accessors() {
        let i32 = RustType::Primitive(PrimitiveRustType::Int(32));
        let field = |access, is_safe| ZngurCppField {
            name: "x".to_owned(),
            rust_name: None,
            ty: i32.clone(),
            access,
            is_safe,
        };
        let method = field(
            CppFieldAccess::Ref {
                unique: true,
                constness: CppConstness::Var,
            },
            true,
        )
        .as_cpp_method();
        assert_eq!(method.receiver, ZngurMethodReceiver::Ref(Mutability::Mut));
        assert_eq!(method.receiver_constness, CppConstness::Var);
        assert_eq!(method.output.to_string(), "&mut ::zngur_lib::Cpp<i32>");
        assert!(method.is_safe);
        let method = field(
            CppFieldAccess::Ref {
                unique: true,
                constness: CppConstness::Const,
            },
            true,
        )
        .as_cpp_method();
        assert_eq!(method.receiver, ZngurMethodReceiver::Ref(Mutability::Mut));
        assert_eq!(method.receiver_constness, CppConstness::Var);
        assert_eq!(
            method.output.to_string(),
            "&mut ::zngur_lib::Cpp<i32, ::zngur_lib::Const>"
        );
        let method = field(
            CppFieldAccess::Copy {
                constness: CppConstness::Const,
            },
            false,
        )
        .as_cpp_method();
        assert_eq!(method.receiver, ZngurMethodReceiver::Ref(Mutability::Not));
        assert_eq!(method.receiver_constness, CppConstness::Const);
        assert_eq!(
            method.output.to_string(),
            "::zngur_lib::Cpp<i32, ::zngur_lib::Const>"
        );
        assert!(!method.is_safe);
        let method = field(
            CppFieldAccess::Copy {
                constness: CppConstness::Var,
            },
            true,
        )
        .as_cpp_method();
        assert_eq!(method.receiver_constness, CppConstness::Const);
        assert_eq!(method.output, i32);
    }

    #[test]
    fn test_cpp_const_spelling() {
        let i32 = RustType::Primitive(PrimitiveRustType::Int(32));
        let span = RustType::Cpp(CppTypePath {
            path: vec!["std".to_owned(), "span".to_owned()],
            generics: vec![RustType::CppConst(Box::new(i32.clone()))],
        });
        assert_eq!(
            span.cpp_spelling().unwrap(),
            "::std::span< const ::int32_t >"
        );
        assert_eq!(
            span.to_string(),
            "std::span::<::zngur_lib::Cpp<i32, ::zngur_lib::Const>>"
        );
        let unique_ptr = RustType::CppConst(Box::new(RustType::Cpp(CppTypePath {
            path: vec!["std".to_owned(), "unique_ptr".to_owned()],
            generics: vec![RustType::Cpp(vec!["a".to_owned(), "B".to_owned()].into())],
        })));
        assert_eq!(
            unique_ptr.to_string(),
            "std::unique_ptr::<a::B, ::zngur_lib::Const>"
        );
        let ptr = RustType::cpp_ptr(Mutability::Not, CppIndirection::Pointer, i32.clone());
        assert_eq!(ptr.cpp_spelling().unwrap(), "const ::int32_t*");
        let ptr = RustType::cpp_ptr(Mutability::Mut, CppIndirection::Pointer, i32);
        assert_eq!(ptr.cpp_spelling().unwrap(), "::int32_t*");
        let const_ptr = RustType::CppConst(Box::new(ptr));
        assert_eq!(const_ptr.cpp_spelling().unwrap(), "::int32_t* const");
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

/// Whether a C++ object is `const` or not, as spelled in the `.zng` file: `const` or `var` in
/// references (`&'a const T`, `&'a unique var T`) or receivers (`&'a const self`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CppConstness {
    Const,
    Var,
}

impl CppConstness {
    /// The `.zng` keyword: `const` or `var`.
    pub fn keyword(self) -> &'static str {
        match self {
            CppConstness::Const => "const",
            CppConstness::Var => "var",
        }
    }
}

/// A Rust lifetime, without the `'`: `Lifetime("a")` is `'a`. `'static` and `'_` are
/// `Lifetime("static")` and `Lifetime("_")`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Lifetime(pub String);

impl Lifetime {
    /// Whether it's `'static` or `'_`, which don't need to be declared.
    pub fn is_builtin(&self) -> bool {
        self.0 == "static" || self.0 == "_"
    }
}

impl Display for Lifetime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "'{}", self.0)
    }
}

/// The `<'a, 'b>` that declares `lifetimes` on a Rust item, or `""` if there are none.
pub fn lifetime_params(lifetimes: &[Lifetime]) -> String {
    if lifetimes.is_empty() {
        String::new()
    } else {
        format!("<{}>", lifetimes.iter().join(", "))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ZngurMethod {
    pub name: String,
    /// The lifetime parameters that the method declares (`fn get<'a>(...)`).
    pub lifetimes: Vec<Lifetime>,
    pub generics: Vec<RustType>,
    pub receiver: ZngurMethodReceiver,
    /// The lifetime of a `&'a self` or `&'a mut self` receiver.
    pub receiver_lifetime: Option<Lifetime>,
    pub inputs: Vec<RustType>,
    pub output: RustType,
    pub is_safe: Option<bool>,
}

impl ZngurMethod {
    /// The Rust receiver: `""`, `"&'a self"`, `"&mut self"` or `"self"`.
    pub fn rust_receiver(&self) -> String {
        let lifetime = self
            .receiver_lifetime
            .as_ref()
            .map(|l| format!("{l} "))
            .unwrap_or_default();
        match self.receiver {
            ZngurMethodReceiver::Static => String::new(),
            ZngurMethodReceiver::Ref(Mutability::Not) => format!("&{lifetime}self"),
            ZngurMethodReceiver::Ref(Mutability::Mut) => format!("&{lifetime}mut self"),
            ZngurMethodReceiver::Move => "self".to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ZngurFn {
    pub path: RustPathAndGenerics,
    /// The lifetime parameters that the function declares. They're only documentation: zngur
    /// calls the function without naming them.
    pub lifetimes: Vec<Lifetime>,
    pub inputs: Vec<RustType>,
    pub output: RustType,
    pub is_safe: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ZngurExternCppFn {
    pub name: String,
    /// The lifetime parameters of the generated Rust function.
    pub lifetimes: Vec<Lifetime>,
    pub inputs: Vec<RustType>,
    pub output: RustType,
    pub is_safe: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ZngurExternCppImpl {
    /// The lifetime parameters of the `impl` block (`impl<'a> crate::Parser<'a>`).
    pub lifetimes: Vec<Lifetime>,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CppStackOwnedData {
    pub destructor_fn: String,
    pub cpp_type: String,
}

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

#[derive(Debug, Clone, PartialEq)]
pub struct ZngurType {
    pub ty: RustType,
    /// The lifetime parameters of a Rust type (`type<'a> crate::Parser<'a>`), which its methods
    /// can use. The generated glue names the type with `'_` in their place.
    pub lifetimes: Vec<Lifetime>,
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
    /// The C++ constructors of a C++ type that Rust holds by value.
    pub cpp_constructors: Vec<ZngurCppConstructor>,
    /// Accessors for the fields of a C++ type.
    pub cpp_fields: Vec<ZngurCppField>,
    pub constructor: Option<ZngurConstructor>,
    pub variants: Vec<ZngurVariant>,
    pub fields: Vec<ZngurField>,
    pub cpp_heap_allocated: Option<CppHeapAllocated>,
    pub cpp_ref: Option<CppRef>,
    pub cpp_stack_owned: Option<CppStackOwned>,
}

impl ZngurType {
    /// The C++ name of a C++ type, e.g. `::a::B` (or the explicit name from a legacy
    /// `#cpp_stack_owned`, `#cpp_ref`, or `#cpp_heap_allocated` directive).
    pub fn cpp_name(&self) -> Option<String> {
        if let Some(c) = &self.cpp_stack_owned {
            return Some(c.0.clone());
        }
        if let Some(c) = &self.cpp_ref {
            return Some(c.0.clone());
        }
        if let Some(c) = &self.cpp_heap_allocated {
            return Some(c.0.clone());
        }
        match &self.ty {
            RustType::Cpp(_) => self.ty.cpp_spelling().ok(),
            _ => None,
        }
    }

    /// The size and alignment of a C++ type that Rust holds by value (see [`ZngurType::layout`]).
    pub fn cpp_by_value_layout(&self) -> Option<LayoutPolicy> {
        if self.cpp_stack_owned.is_some() {
            return match self.layout {
                Some(
                    layout @ (LayoutPolicy::StackAllocated { .. }
                    | LayoutPolicy::Conservative { .. }),
                ) => Some(layout),
                _ => None,
            };
        }
        if self.cpp_ref.is_some() || self.cpp_heap_allocated.is_some() {
            return None;
        }
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
///
/// NOTE: Variant declaration order (`No < Checked < Unchecked`) defines merge precedence via
/// [`Ord`] (an unchecked promise wins over a checked one, which wins over none). Do not reorder
/// variants without updating [`TriviallyRelocatable::combine`].
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

    /// Combines two declarations according to merge precedence (`No < Checked < Unchecked`).
    pub fn combine(self, other: Self) -> Self {
        self.max(other)
    }

    /// The `.zng` directive spelling for this variant, if any.
    pub fn directive(self) -> Option<&'static str> {
        match self {
            TriviallyRelocatable::No => None,
            TriviallyRelocatable::Checked => Some("#trivially_relocatable"),
            TriviallyRelocatable::Unchecked => Some("#unsafe(trivially_relocatable)"),
        }
    }
}

/// A C++ member function of a C++ type, declared C++-style in the type's block, e.g.
/// `unsafe fn value<'a>(&'a const self) -> i32;` or `safe fn make(i32) -> c++::ns::Bar;`. zngur
/// generates a Rust function for it that calls the C++ member function directly.
#[derive(Debug, PartialEq, Eq, Clone, Hash)]
pub struct ZngurCppMethod {
    /// The name of the C++ member function.
    pub cpp_name: String,
    /// The lifetime parameters that it declares (`fn get<'a>(&'a const self) -> &'a const i32;`).
    pub lifetimes: Vec<Lifetime>,
    /// The name of the Rust function (from `as`), if it isn't `cpp_name`.
    pub rust_name: Option<String>,
    /// `Static` for static member functions, and `Ref(m)` for the others: `Ref(Not)` for a shared
    /// receiver (`&'a const self` / `&'a var self`, `&self` in Rust), and `Ref(Mut)` for a unique
    /// one (`&'a unique const self` / `&'a unique var self`, `&mut self` in Rust).
    pub receiver: ZngurMethodReceiver,
    /// Whether the member function is `const` (`Const`, a method of `Self<C>` for both
    /// constnesses) or not (`Var`, a method of `Self<NonConst>`), for both Rust-style and raw
    /// receivers. `Var` for static member functions.
    pub receiver_constness: CppConstness,
    /// The lifetime of a `&'a const self` / `&'a unique var self` / ... receiver.
    pub receiver_lifetime: Option<Lifetime>,
    /// For a raw receiver (`const self&`, `self*`, ...), whether it's a C++ reference or pointer.
    /// The Rust function is then an associated function taking `i0: *const Self` (`Self<C>`) or
    /// `i0: *mut Self`, rather than a method.
    pub raw_receiver: Option<CppIndirection>,
    pub inputs: Vec<RustType>,
    pub output: RustType,
    pub is_safe: bool,
}

impl ZngurCppMethod {
    /// The name of the Rust function.
    pub fn rust_name(&self) -> &str {
        self.rust_name.as_deref().unwrap_or(&self.cpp_name)
    }

    /// A copy of `self` with all lifetimes erased (for span lookup keys).
    pub fn with_erased_lifetimes(&self) -> Self {
        Self {
            cpp_name: self.cpp_name.clone(),
            lifetimes: vec![],
            rust_name: self.rust_name.clone(),
            receiver: self.receiver,
            receiver_constness: self.receiver_constness,
            receiver_lifetime: None,
            raw_receiver: self.raw_receiver,
            inputs: self
                .inputs
                .iter()
                .map(RustType::with_erased_lifetimes)
                .collect(),
            output: self.output.with_erased_lifetimes(),
            is_safe: self.is_safe,
        }
    }
}

/// A C++ constructor of a C++ type that Rust holds by value, declared like
/// `safe constructor(i32) as from_i32;`. zngur generates the associated function `rust_name`,
/// which returns the constructor call as a `zngur_lib::CppCtor` (to `build()` or `init()`).
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct ZngurCppConstructor {
    /// The name of the Rust function (from `as`, which is required).
    pub rust_name: String,
    /// The lifetime parameters that it declares (`safe constructor<'a>(&'a c++::B) as copy;`).
    pub lifetimes: Vec<Lifetime>,
    pub inputs: Vec<RustType>,
    pub is_safe: bool,
}

/// An accessor for a field (data member) of a C++ type, declared like
/// `field &const x (type = i32);`, `field &unique var x (type = i32) as x_mut;` or
/// `unsafe field &var next (type = c++::Node*) as next_var;`. zngur generates a method that
/// returns a reference to the field (`&Cpp<i32, Const>`, `&mut Cpp<i32>`, ...) or a copy of it
/// (see [`CppFieldAccess`]).
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct ZngurCppField {
    /// The name of the C++ field.
    pub name: String,
    /// The name of the Rust function (from `as`), if it isn't `name`.
    pub rust_name: Option<String>,
    /// The type of the field.
    pub ty: RustType,
    /// What the accessor returns.
    pub access: CppFieldAccess,
    /// Whether calling the generated Rust accessor is safe (`true` by default, or `false` for
    /// `unsafe field`).
    pub is_safe: bool,
}

/// What a C++ field accessor ([`ZngurCppField`]) returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CppFieldAccess {
    /// A reference to the field (`field &[unique] (const | var) x ...`): `&'a [unique] const T` or
    /// `&'a [unique] var T`, through a receiver of the same kind (`&'a [unique] const self`, ...).
    Ref {
        unique: bool,
        constness: CppConstness,
    },
    /// A copy of the field (`field (const | var) x ...`): a `const T` (`T<Const>` /
    /// `Cpp<i32, Const>` in Rust) or a `T` (`T` / `i32`). Copying only reads the field, so the
    /// accessor is a `&'a const self` method either way.
    Copy { constness: CppConstness },
}

impl ZngurCppField {
    /// The name of the Rust function.
    pub fn rust_name(&self) -> &str {
        self.rust_name.as_deref().unwrap_or(&self.name)
    }

    /// The accessor as a C++ member function, which returns a reference to the field or a copy
    /// of it.
    pub fn as_cpp_method(&self) -> ZngurCppMethod {
        let (receiver, receiver_constness, output) = match self.access {
            CppFieldAccess::Ref { unique, constness } => {
                let m = if unique {
                    Mutability::Mut
                } else {
                    Mutability::Not
                };
                // Because `&mut` in Rust allows replacing a `Sized` field value via `*r = ...` or
                // `core::mem::swap`, a `unique` reference accessor requires a non-`const` receiver
                // (`Owner<NonConst>`) even when `constness` is `Const`.
                let receiver_constness = if unique { CppConstness::Var } else { constness };
                (
                    m,
                    receiver_constness,
                    RustType::Ref(
                        m,
                        None,
                        Box::new(RustType::cpp_object(constness, self.ty.clone())),
                    ),
                )
            }
            CppFieldAccess::Copy { constness } => (
                Mutability::Not,
                CppConstness::Const,
                match constness {
                    CppConstness::Const => RustType::cpp_object(constness, self.ty.clone()),
                    CppConstness::Var => self.ty.clone(),
                },
            ),
        };
        ZngurCppMethod {
            cpp_name: self.name.clone(),
            lifetimes: vec![],
            rust_name: self.rust_name.clone(),
            receiver: ZngurMethodReceiver::Ref(receiver),
            receiver_constness,
            receiver_lifetime: None,
            raw_receiver: None,
            inputs: vec![],
            output,
            is_safe: self.is_safe,
        }
    }
}

/// A free C++ function, declared with a `c++::` path or in a `c++::` scope like
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
    /// The lifetime parameters that it declares.
    pub lifetimes: Vec<Lifetime>,
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

#[derive(Debug, Default, PartialEq, Eq)]
pub struct AdditionalIncludes(pub String);

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ConvertPanicToException(pub bool);

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Import(pub std::path::PathBuf);

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModuleImport {
    pub path: std::path::PathBuf,
    /// The `#crate` of the imported module, if it has one.
    pub crate_name: Option<String>,
    /// The `c++::` types that the imported module declares (their paths, without `c++::`). Their
    /// Rust wrappers live at `::<crate_name>::generated::<path>`.
    pub cpp_types: Vec<Vec<String>>,
    /// Concrete instantiations of C++ class templates declared in the imported module (with erased
    /// lifetimes). Each entry has non-empty `generics`, and its `path` is also in `cpp_types`.
    pub cpp_template_instantiations: Vec<CppTypePath>,
    /// Subset of `cpp_types` declared with `#cpp_heap_allocated`.
    pub cpp_heap_allocated_types: Vec<Vec<String>>,
    /// The `c++::` types from this module that Rust holds by value, paired with their C++
    /// spelling.
    pub cpp_by_value_types: Vec<(RustType, String)>,
}

#[derive(Debug, Default, PartialEq)]
pub struct ZngurSpec {
    pub imported_modules: Vec<ModuleImport>,
    pub types: Vec<ZngurType>,
    pub traits: IndexMap<RustTrait, ZngurTrait>,
    pub funcs: Vec<ZngurFn>,
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
        /// The lifetimes of a `for<'a>` binder: `for<'a> Fn(&'a i32) -> &'a i32`.
        lifetimes: Vec<Lifetime>,
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

    /// `self`, with all lifetimes erased.
    pub fn with_erased_lifetimes(&self) -> Self {
        match RustType::Dyn(self.clone(), vec![]).with_erased_lifetimes() {
            RustType::Dyn(tr, _) => tr,
            _ => unreachable!(),
        }
    }

    /// Canonicalizes declared `outer` lifetimes and `for<...>` binders to positional indices.
    pub fn canonicalize_lifetimes(&self, outer: &[Lifetime]) -> Self {
        match RustType::Dyn(self.clone(), vec![]).canonicalize_lifetimes(outer) {
            RustType::Dyn(tr, _) => tr,
            _ => unreachable!(),
        }
    }

    /// Every non-builtin lifetime appearing in `self` (both free lifetimes and `for<...>` binders).
    pub fn all_lifetimes(&self) -> Vec<Lifetime> {
        RustType::Dyn(self.clone(), vec![]).all_lifetimes()
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

#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct RustPathAndGenerics {
    pub path: Vec<String>,
    /// Lifetime arguments (`::std::slice::Iter<'a, i32>`), which come first.
    pub lifetimes: Vec<Lifetime>,
    pub generics: Vec<RustType>,
    pub named_generics: Vec<(String, RustType)>,
}

impl RustPathAndGenerics {
    /// `self`, with each named lifetime that isn't bound by a `for<...>` inside it replaced by
    /// `'_`.
    pub fn with_anonymous_lifetimes(&self) -> Self {
        match RustType::Adt(self.clone()).with_anonymous_lifetimes() {
            RustType::Adt(pg) => pg,
            _ => unreachable!(),
        }
    }

    /// `self`, with all lifetimes erased.
    pub fn with_erased_lifetimes(&self) -> Self {
        RustPathAndGenerics {
            path: self.path.clone(),
            lifetimes: vec![],
            generics: self
                .generics
                .iter()
                .map(RustType::with_erased_lifetimes)
                .collect(),
            named_generics: self
                .named_generics
                .iter()
                .map(|(n, t)| (n.clone(), t.with_erased_lifetimes()))
                .collect(),
        }
    }

    /// Canonicalizes declared `outer` lifetimes and `for<...>` binders to positional indices.
    pub fn canonicalize_lifetimes(&self, outer: &[Lifetime]) -> Self {
        match RustType::Adt(self.clone()).canonicalize_lifetimes(outer) {
            RustType::Adt(pg) => pg,
            _ => unreachable!(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RustType {
    Primitive(PrimitiveRustType),
    /// `&T`, `&'a T`, `&mut T` or `&'a mut T`: a reference to a Rust value. Or, when `T` is a
    /// [`RustType::CppConst`] or a [`RustType::CppVar`], a reference to a C++ object, where `Mut`
    /// means `unique`: `&'a const T` is `Ref(Not, 'a, CppConst(T))`, `&'a var T` is
    /// `Ref(Not, 'a, CppVar(T))`, `&'a unique var T` is `Ref(Mut, 'a, CppVar(T))` and
    /// `&'a unique const T` is `Ref(Mut, 'a, CppConst(T))`.
    Ref(Mutability, Option<Lifetime>, Box<RustType>),
    Raw(Mutability, Box<RustType>),
    Boxed(Box<RustType>),
    Slice(Box<RustType>),
    Dyn(RustTrait, Vec<String>),
    Impl(RustTrait, Vec<String>),
    Tuple(Vec<RustType>),
    Adt(RustPathAndGenerics),
    TypeVar(TypeVar),
    /// A C++ type, declared with a `c++::` path. Its C++ name is the path, e.g. `c++::a::B` is
    /// `::a::B`, and `c++::std::unique_ptr<c++::a::B>` is `::std::unique_ptr< ::a::B >`.
    Cpp(CppTypePath),
    /// A C++-style pointer or reference (`T*` or `T&` in the `.zng` file) to a C++ object, which is
    /// always a [`RustType::CppConst`] (`const T*`, `const T&`) or a [`RustType::CppVar`] (`T*`,
    /// `T&`). Both are raw pointers in Rust (`*const T<Const>` / `*mut T`, `*const Cpp<i32, Const>`
    /// / `*mut Cpp<i32>`), which may be null or invalid. (C++ references with a lifetime are Rust
    /// references, [`RustType::Ref`].)
    CppPtr(CppIndirection, Box<RustType>),
    /// A `const` C++ object (`const T` in the `.zng` file), where `T` is a `c++::` type, a builtin
    /// or a C++ pointer. It's a `T<Const>` in Rust for `c++::` types, and a
    /// `zngur_lib::Cpp<T, Const>` for builtins and pointers. It's the pointee of C++ references
    /// and pointers to `const`, or a C++ template argument (`c++::std::span<const i32>`).
    CppConst(Box<RustType>),
    /// A non-`const` C++ object (`var T` in a reference, or the pointee of `T*` / `T&`). It's a
    /// `T` in Rust for `c++::` types, and a `zngur_lib::Cpp<T>` for builtins and pointers. It's
    /// only the pointee of references and pointers.
    CppVar(Box<RustType>),
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

    /// A C++ reference or pointer to `ty`, to `const` if `mutability` is `Not`: `const ty&` /
    /// `const ty*` (a [`RustType::CppPtr`] to a [`RustType::CppConst`]), or `ty&` / `ty*` (to a
    /// [`RustType::CppVar`]).
    pub fn cpp_ptr(mutability: Mutability, indirection: CppIndirection, ty: RustType) -> Self {
        let constness = match mutability {
            Mutability::Not => CppConstness::Const,
            Mutability::Mut => CppConstness::Var,
        };
        RustType::CppPtr(indirection, Box::new(RustType::cpp_object(constness, ty)))
    }

    /// `ty` as a C++ object: a [`RustType::CppConst`] or a [`RustType::CppVar`]. If `ty` already
    /// is one, it's `const` if either is (`const` of a `const T` is a `const T`).
    pub fn cpp_object(constness: CppConstness, ty: RustType) -> Self {
        match (constness, ty) {
            (_, RustType::CppConst(t)) | (CppConstness::Const, RustType::CppVar(t)) => {
                RustType::CppConst(t)
            }
            (CppConstness::Var, ty @ RustType::CppVar(_)) => ty,
            (CppConstness::Const, ty) => RustType::CppConst(Box::new(ty)),
            (CppConstness::Var, ty) => RustType::CppVar(Box::new(ty)),
        }
    }

    /// For a C++ object ([`RustType::CppConst`] or [`RustType::CppVar`]), its constness and type.
    pub fn as_cpp_object(&self) -> Option<(CppConstness, &RustType)> {
        match self {
            RustType::CppConst(t) => Some((CppConstness::Const, t)),
            RustType::CppVar(t) => Some((CppConstness::Var, t)),
            _ => None,
        }
    }

    /// For a nullable reference to a C++ object (`::std::option::Option<&'a const T>`,
    /// `::core::option::Option<&'a unique var T>`, ...), the reference: whether it's `unique`
    /// (`Mut`), its lifetime and what it refers to (a [`RustType::CppConst`] or a
    /// [`RustType::CppVar`]). It's a `const T*` or a `T*` in C++.
    pub fn as_nullable_cpp_ref(&self) -> Option<(Mutability, Option<&Lifetime>, &RustType)> {
        let RustType::Adt(pg) = self else {
            return None;
        };
        let is_option = matches!(
            pg.path.as_slice(),
            [root, opt_mod, opt_ty]
                if matches!(root.as_str(), "std" | "core")
                    && opt_mod == "option"
                    && opt_ty == "Option"
        );
        match pg.generics.as_slice() {
            [RustType::Ref(m, lifetime, pointee)]
                if is_option
                    && pg.lifetimes.is_empty()
                    && pg.named_generics.is_empty()
                    && pointee.as_cpp_object().is_some() =>
            {
                Some((*m, lifetime.as_ref(), pointee))
            }
            _ => None,
        }
    }

    /// How `self` crosses to C++ in a C++ function's signature: C++ object and builtin references
    /// are C++ references, which cross as pointers like the raw C++-style ones. A reference to a
    /// C++ object keeps its constness (`&'a const T` and `&'a unique const T` are `const T&`,
    /// `&'a var T` and `&'a unique var T` are `T&`), and a reference to a builtin Rust value is a
    /// `const T&` if it's shared (`&'a i32` is `const int32_t&`) and a `T&` otherwise; references
    /// to other Rust types (`&'a crate::Foo`, `&'a str`, ...) cross as `rust::Ref<T>` /
    /// `rust::RefMut<T>`. A nullable reference to a C++ object (`Option<&'a const T>`) is a C++
    /// pointer (`const T*`).
    pub fn cpp_bridge_type(&self) -> RustType {
        match self {
            RustType::Ref(_, _, inner) if inner.as_cpp_object().is_some() => {
                RustType::CppPtr(CppIndirection::Reference, inner.clone())
            }
            RustType::Ref(m, _, inner)
                if matches!(
                    &**inner,
                    RustType::Primitive(
                        PrimitiveRustType::Int(_)
                            | PrimitiveRustType::Uint(_)
                            | PrimitiveRustType::Float(_)
                            | PrimitiveRustType::Usize
                            | PrimitiveRustType::Bool,
                    )
                ) =>
            {
                RustType::cpp_ptr(*m, CppIndirection::Reference, (**inner).clone())
            }
            // A nullable reference to a C++ object is a C++ pointer.
            ty => match ty.as_nullable_cpp_ref() {
                Some((_, _, pointee)) => {
                    RustType::CppPtr(CppIndirection::Pointer, Box::new(pointee.clone()))
                }
                None => ty.clone(),
            },
        }
    }

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
                RustType::Ref(_, _, t)
                | RustType::Raw(_, t)
                | RustType::Boxed(t)
                | RustType::Slice(t)
                | RustType::CppPtr(_, t)
                | RustType::CppConst(t)
                | RustType::CppVar(t) => collect(t, out),
                RustType::Tuple(tys) => all(tys, out),
                RustType::Adt(pg) => {
                    all(&pg.generics, out);
                    pg.named_generics.iter().for_each(|(_, t)| collect(t, out));
                }
                RustType::Cpp(path) => all(&path.generics, out),
                RustType::Dyn(tr, _) | RustType::Impl(tr, _) => match tr {
                    RustTrait::Normal(pg) => {
                        all(&pg.generics, out);
                        pg.named_generics.iter().for_each(|(_, t)| collect(t, out));
                    }
                    RustTrait::Fn { inputs, output, .. } => {
                        all(inputs, out);
                        collect(output, out);
                    }
                },
                RustType::Primitive(_) => {}
            }
        }
        let mut out = vec![];
        collect(self, &mut out);
        out
    }

    /// The named lifetimes that `self` uses (not `'static` or `'_`), other than those bound by a
    /// `for<...>` inside it, in order of appearance (with duplicates).
    pub fn free_lifetimes(&self) -> Vec<Lifetime> {
        let mut out = vec![];
        let _ = self.map_free_lifetimes(&mut |l| {
            out.push(l.clone());
            l.clone()
        });
        out
    }

    /// Every named lifetime appearing in `self` (not `'static` or `'_`), including both free
    /// lifetimes and `for<...>` binders, in order of appearance.
    pub fn all_lifetimes(&self) -> Vec<Lifetime> {
        fn push(l: &Lifetime, out: &mut Vec<Lifetime>) {
            if !l.is_builtin() {
                out.push(l.clone());
            }
        }
        fn go_path(pg: &RustPathAndGenerics, out: &mut Vec<Lifetime>) {
            pg.lifetimes.iter().for_each(|l| push(l, out));
            pg.generics.iter().for_each(|t| go(t, out));
            pg.named_generics.iter().for_each(|(_, t)| go(t, out));
        }
        fn go_trait(tr: &RustTrait, out: &mut Vec<Lifetime>) {
            match tr {
                RustTrait::Normal(pg) => go_path(pg, out),
                RustTrait::Fn {
                    lifetimes,
                    inputs,
                    output,
                    ..
                } => {
                    lifetimes.iter().for_each(|l| push(l, out));
                    inputs.iter().for_each(|t| go(t, out));
                    go(output, out);
                }
            }
        }
        fn go(ty: &RustType, out: &mut Vec<Lifetime>) {
            match ty {
                RustType::Ref(_, l, t) => {
                    if let Some(l) = l {
                        push(l, out);
                    }
                    go(t, out);
                }
                RustType::Raw(_, t)
                | RustType::Boxed(t)
                | RustType::Slice(t)
                | RustType::CppPtr(_, t)
                | RustType::CppConst(t)
                | RustType::CppVar(t) => go(t, out),
                RustType::Tuple(tys) => tys.iter().for_each(|t| go(t, out)),
                RustType::Adt(pg) => go_path(pg, out),
                RustType::Cpp(path) => path.generics.iter().for_each(|t| go(t, out)),
                RustType::Dyn(tr, _) | RustType::Impl(tr, _) => go_trait(tr, out),
                RustType::Primitive(_) | RustType::TypeVar(_) => {}
            }
        }
        let mut out = Vec::new();
        go(self, &mut out);
        out
    }

    /// Alpha-renames any `for<...>` binder in `self` whose lifetime name is in `outer_in_scope`
    /// (or an enclosing `for<...>` binder), avoiding all lifetimes in `avoid` and recording newly
    /// chosen lifetimes in `avoid`.
    pub fn rename_colliding_for_binders(
        &self,
        outer_in_scope: &std::collections::HashSet<Lifetime>,
        avoid: &mut std::collections::HashSet<Lifetime>,
    ) -> RustType {
        fn go_path(
            pg: &RustPathAndGenerics,
            in_scope: &std::collections::HashSet<Lifetime>,
            avoid: &mut std::collections::HashSet<Lifetime>,
        ) -> RustPathAndGenerics {
            RustPathAndGenerics {
                path: pg.path.clone(),
                lifetimes: pg.lifetimes.clone(),
                generics: pg.generics.iter().map(|t| go(t, in_scope, avoid)).collect(),
                named_generics: pg
                    .named_generics
                    .iter()
                    .map(|(n, t)| (n.clone(), go(t, in_scope, avoid)))
                    .collect(),
            }
        }
        fn go_trait(
            tr: &RustTrait,
            in_scope: &std::collections::HashSet<Lifetime>,
            avoid: &mut std::collections::HashSet<Lifetime>,
        ) -> RustTrait {
            match tr {
                RustTrait::Normal(pg) => RustTrait::Normal(go_path(pg, in_scope, avoid)),
                RustTrait::Fn {
                    name,
                    lifetimes,
                    inputs,
                    output,
                } => {
                    let mut local_avoid = avoid.clone();
                    local_avoid.extend(in_scope.iter().cloned());
                    local_avoid.extend(lifetimes.iter().cloned());
                    for t in inputs.iter().chain([output.as_ref()]) {
                        local_avoid.extend(t.all_lifetimes());
                    }
                    let mut next_in_scope = in_scope.clone();
                    let mut renames: Vec<(Lifetime, Lifetime)> = Vec::new();
                    let mut counter = 0usize;
                    let new_lifetimes: Vec<Lifetime> = lifetimes
                        .iter()
                        .map(|bl| {
                            if next_in_scope.contains(bl) {
                                let fresh = loop {
                                    let candidate = Lifetime(format!("{}_{counter}", bl.0));
                                    counter += 1;
                                    if !local_avoid.contains(&candidate) {
                                        break candidate;
                                    }
                                };
                                local_avoid.insert(fresh.clone());
                                avoid.insert(fresh.clone());
                                next_in_scope.insert(fresh.clone());
                                renames.push((bl.clone(), fresh.clone()));
                                fresh
                            } else {
                                next_in_scope.insert(bl.clone());
                                bl.clone()
                            }
                        })
                        .collect();
                    let remap = |t: &RustType| -> RustType {
                        if renames.is_empty() {
                            t.clone()
                        } else {
                            t.map_free_lifetimes(&mut |l| {
                                renames
                                    .iter()
                                    .find(|(old, _)| old == l)
                                    .map(|(_, new)| new.clone())
                                    .unwrap_or_else(|| l.clone())
                            })
                        }
                    };
                    let inputs: Vec<RustType> = inputs
                        .iter()
                        .map(|t| go(&remap(t), &next_in_scope, avoid))
                        .collect();
                    let output = Box::new(go(&remap(output), &next_in_scope, avoid));
                    RustTrait::Fn {
                        name: name.clone(),
                        lifetimes: new_lifetimes,
                        inputs,
                        output,
                    }
                }
            }
        }
        fn go(
            ty: &RustType,
            in_scope: &std::collections::HashSet<Lifetime>,
            avoid: &mut std::collections::HashSet<Lifetime>,
        ) -> RustType {
            match ty {
                RustType::Ref(m, l, t) => {
                    RustType::Ref(*m, l.clone(), Box::new(go(t, in_scope, avoid)))
                }
                RustType::Raw(m, t) => RustType::Raw(*m, Box::new(go(t, in_scope, avoid))),
                RustType::Boxed(t) => RustType::Boxed(Box::new(go(t, in_scope, avoid))),
                RustType::Slice(t) => RustType::Slice(Box::new(go(t, in_scope, avoid))),
                RustType::CppPtr(k, t) => RustType::CppPtr(*k, Box::new(go(t, in_scope, avoid))),
                RustType::CppConst(t) => RustType::CppConst(Box::new(go(t, in_scope, avoid))),
                RustType::CppVar(t) => RustType::CppVar(Box::new(go(t, in_scope, avoid))),
                RustType::Tuple(tys) => {
                    RustType::Tuple(tys.iter().map(|t| go(t, in_scope, avoid)).collect())
                }
                RustType::Adt(pg) => RustType::Adt(go_path(pg, in_scope, avoid)),
                RustType::Dyn(tr, bounds) => {
                    RustType::Dyn(go_trait(tr, in_scope, avoid), bounds.clone())
                }
                RustType::Impl(tr, bounds) => {
                    RustType::Impl(go_trait(tr, in_scope, avoid), bounds.clone())
                }
                RustType::Cpp(path) => RustType::Cpp(CppTypePath {
                    path: path.path.clone(),
                    generics: path
                        .generics
                        .iter()
                        .map(|t| go(t, in_scope, avoid))
                        .collect(),
                }),
                RustType::Primitive(_) | RustType::TypeVar(_) => ty.clone(),
            }
        }
        go(self, outer_in_scope, avoid)
    }

    /// `self`, with each named lifetime that isn't bound by a `for<...>` inside it replaced by
    /// `'_`. The generated glue uses it for types whose lifetimes aren't declared there.
    pub fn with_anonymous_lifetimes(&self) -> RustType {
        self.map_free_lifetimes(&mut |_| Lifetime("_".to_owned()))
    }

    /// `self`, with `f` applied to each named lifetime that isn't bound by a `for<...>` inside it.
    pub fn map_free_lifetimes(&self, f: &mut impl FnMut(&Lifetime) -> Lifetime) -> RustType {
        fn go(
            ty: &RustType,
            bound: &[Lifetime],
            f: &mut dyn FnMut(&Lifetime) -> Lifetime,
        ) -> RustType {
            let lt = |l: &Lifetime, f: &mut dyn FnMut(&Lifetime) -> Lifetime| {
                if l.is_builtin() || bound.contains(l) {
                    l.clone()
                } else {
                    f(l)
                }
            };
            let all = |tys: &[RustType], f: &mut dyn FnMut(&Lifetime) -> Lifetime| {
                tys.iter().map(|t| go(t, bound, f)).collect::<Vec<_>>()
            };
            match ty {
                RustType::Ref(m, l, t) => {
                    RustType::Ref(*m, l.as_ref().map(|l| lt(l, f)), Box::new(go(t, bound, f)))
                }
                RustType::Raw(m, t) => RustType::Raw(*m, Box::new(go(t, bound, f))),
                RustType::Boxed(t) => RustType::Boxed(Box::new(go(t, bound, f))),
                RustType::Slice(t) => RustType::Slice(Box::new(go(t, bound, f))),
                RustType::CppPtr(k, t) => RustType::CppPtr(*k, Box::new(go(t, bound, f))),
                RustType::CppConst(t) => RustType::CppConst(Box::new(go(t, bound, f))),
                RustType::CppVar(t) => RustType::CppVar(Box::new(go(t, bound, f))),
                RustType::Tuple(tys) => RustType::Tuple(all(tys, f)),
                RustType::Adt(pg) => RustType::Adt(go_path(pg, bound, f)),
                RustType::Cpp(path) => RustType::Cpp(CppTypePath {
                    path: path.path.clone(),
                    generics: all(&path.generics, f),
                }),
                RustType::Dyn(tr, bounds) => RustType::Dyn(go_trait(tr, bound, f), bounds.clone()),
                RustType::Impl(tr, bounds) => {
                    RustType::Impl(go_trait(tr, bound, f), bounds.clone())
                }
                RustType::Primitive(_) | RustType::TypeVar(_) => ty.clone(),
            }
        }
        fn go_path(
            pg: &RustPathAndGenerics,
            bound: &[Lifetime],
            f: &mut dyn FnMut(&Lifetime) -> Lifetime,
        ) -> RustPathAndGenerics {
            RustPathAndGenerics {
                path: pg.path.clone(),
                lifetimes: pg
                    .lifetimes
                    .iter()
                    .map(|l| {
                        if l.is_builtin() || bound.contains(l) {
                            l.clone()
                        } else {
                            f(l)
                        }
                    })
                    .collect(),
                generics: pg.generics.iter().map(|t| go(t, bound, f)).collect(),
                named_generics: pg
                    .named_generics
                    .iter()
                    .map(|(n, t)| (n.clone(), go(t, bound, f)))
                    .collect(),
            }
        }
        fn go_trait(
            tr: &RustTrait,
            bound: &[Lifetime],
            f: &mut dyn FnMut(&Lifetime) -> Lifetime,
        ) -> RustTrait {
            match tr {
                RustTrait::Normal(pg) => RustTrait::Normal(go_path(pg, bound, f)),
                RustTrait::Fn {
                    name,
                    lifetimes,
                    inputs,
                    output,
                } => {
                    let bound: Vec<Lifetime> = bound.iter().chain(lifetimes).cloned().collect();
                    RustTrait::Fn {
                        name: name.clone(),
                        lifetimes: lifetimes.clone(),
                        inputs: inputs.iter().map(|t| go(t, &bound, f)).collect(),
                        output: Box::new(go(output, &bound, f)),
                    }
                }
            }
        }
        go(self, &[], f)
    }

    /// `self` with all lifetime arguments, reference lifetimes, and `for<...>` binders removed.
    /// Two `RustType`s produce the same C++ wrapper type when their lifetime-erased forms match.
    pub fn with_erased_lifetimes(&self) -> RustType {
        fn erase_trait(tr: &RustTrait) -> RustTrait {
            match tr {
                RustTrait::Normal(pg) => RustTrait::Normal(pg.with_erased_lifetimes()),
                RustTrait::Fn {
                    name,
                    lifetimes: _,
                    inputs,
                    output,
                } => RustTrait::Fn {
                    name: name.clone(),
                    lifetimes: vec![],
                    inputs: inputs.iter().map(RustType::with_erased_lifetimes).collect(),
                    output: Box::new(output.with_erased_lifetimes()),
                },
            }
        }
        match self {
            RustType::Ref(m, _, t) => RustType::Ref(*m, None, Box::new(t.with_erased_lifetimes())),
            RustType::Raw(m, t) => RustType::Raw(*m, Box::new(t.with_erased_lifetimes())),
            RustType::Boxed(t) => RustType::Boxed(Box::new(t.with_erased_lifetimes())),
            RustType::Slice(t) => RustType::Slice(Box::new(t.with_erased_lifetimes())),
            RustType::CppPtr(k, t) => RustType::CppPtr(*k, Box::new(t.with_erased_lifetimes())),
            RustType::CppConst(t) => RustType::CppConst(Box::new(t.with_erased_lifetimes())),
            RustType::CppVar(t) => RustType::CppVar(Box::new(t.with_erased_lifetimes())),
            RustType::Tuple(tys) => {
                RustType::Tuple(tys.iter().map(RustType::with_erased_lifetimes).collect())
            }
            RustType::Adt(pg) => RustType::Adt(pg.with_erased_lifetimes()),
            RustType::Cpp(path) => RustType::Cpp(CppTypePath {
                path: path.path.clone(),
                generics: path
                    .generics
                    .iter()
                    .map(RustType::with_erased_lifetimes)
                    .collect(),
            }),
            RustType::Dyn(tr, bounds) => RustType::Dyn(erase_trait(tr), bounds.clone()),
            RustType::Impl(tr, bounds) => RustType::Impl(erase_trait(tr), bounds.clone()),
            RustType::Primitive(_) | RustType::TypeVar(_) => self.clone(),
        }
    }

    /// Whether `self` contains any non-`'static` lifetime (named `'a`, anonymous `'_`, or elided
    /// `&T`) that is not bound by an enclosing `Fn` trait binder.
    pub fn has_non_static_lifetime(&self) -> bool {
        fn check_path(pg: &RustPathAndGenerics, fn_bound: &[Lifetime], in_fn: bool) -> bool {
            pg.lifetimes
                .iter()
                .any(|l| l.0 != "static" && !(in_fn && l.0 == "_") && !fn_bound.contains(l))
                || pg.generics.iter().any(|t| check(t, fn_bound, in_fn))
                || pg
                    .named_generics
                    .iter()
                    .any(|(_, t)| check(t, fn_bound, in_fn))
        }
        fn check_trait(tr: &RustTrait, fn_bound: &[Lifetime], in_fn: bool) -> bool {
            match tr {
                RustTrait::Normal(pg) => check_path(pg, fn_bound, in_fn),
                RustTrait::Fn {
                    lifetimes,
                    inputs,
                    output,
                    ..
                } => {
                    let bound: Vec<Lifetime> = fn_bound.iter().chain(lifetimes).cloned().collect();
                    inputs
                        .iter()
                        .chain([output.as_ref()])
                        .any(|t| check(t, &bound, true))
                }
            }
        }
        fn check(ty: &RustType, fn_bound: &[Lifetime], in_fn: bool) -> bool {
            match ty {
                RustType::Ref(_, l, t) => {
                    let ref_non_static = match l {
                        Some(l) => {
                            l.0 != "static" && !(in_fn && l.0 == "_") && !fn_bound.contains(l)
                        }
                        None => !in_fn,
                    };
                    ref_non_static || check(t, fn_bound, in_fn)
                }
                RustType::Raw(_, t)
                | RustType::Boxed(t)
                | RustType::Slice(t)
                | RustType::CppPtr(_, t)
                | RustType::CppConst(t)
                | RustType::CppVar(t) => check(t, fn_bound, in_fn),
                RustType::Tuple(tys) => tys.iter().any(|t| check(t, fn_bound, in_fn)),
                RustType::Adt(pg) => check_path(pg, fn_bound, in_fn),
                RustType::Cpp(path) => path.generics.iter().any(|t| check(t, fn_bound, in_fn)),
                RustType::Dyn(tr, _) | RustType::Impl(tr, _) => check_trait(tr, fn_bound, in_fn),
                RustType::Primitive(_) | RustType::TypeVar(_) => false,
            }
        }
        check(self, &[], false)
    }

    /// Canonicalizes declared `outer` lifetimes and `for<...>` binders to positional indices so
    /// alpha-equivalent declarations (e.g. `type<'a> Foo<'a>` and `type<'b> Foo<'b>`) compare
    /// equal while distinct instantiations (e.g. `Iter<'static, i32>` vs `Iter<'a, i32>`) differ.
    pub fn canonicalize_lifetimes(&self, outer: &[Lifetime]) -> RustType {
        fn map_lt(l: &Lifetime, env: &[(Lifetime, Lifetime)]) -> Lifetime {
            if l.is_builtin() {
                return l.clone();
            }
            env.iter()
                .rev()
                .find(|(from, _)| from == l)
                .map(|(_, to)| to.clone())
                .unwrap_or_else(|| l.clone())
        }
        fn go_path(
            pg: &RustPathAndGenerics,
            env: &[(Lifetime, Lifetime)],
            depth: usize,
        ) -> RustPathAndGenerics {
            RustPathAndGenerics {
                path: pg.path.clone(),
                lifetimes: pg.lifetimes.iter().map(|l| map_lt(l, env)).collect(),
                generics: pg.generics.iter().map(|t| go(t, env, depth)).collect(),
                named_generics: pg
                    .named_generics
                    .iter()
                    .map(|(n, t)| (n.clone(), go(t, env, depth)))
                    .collect(),
            }
        }
        fn go_trait(tr: &RustTrait, env: &[(Lifetime, Lifetime)], depth: usize) -> RustTrait {
            match tr {
                RustTrait::Normal(pg) => RustTrait::Normal(go_path(pg, env, depth)),
                RustTrait::Fn {
                    name,
                    lifetimes,
                    inputs,
                    output,
                } => {
                    let mut next_env = env.to_vec();
                    let canon_lts: Vec<Lifetime> = lifetimes
                        .iter()
                        .enumerate()
                        .map(|(i, l)| {
                            let canon = Lifetime(format!("__fn_{depth}_{i}"));
                            next_env.push((l.clone(), canon.clone()));
                            canon
                        })
                        .collect();
                    RustTrait::Fn {
                        name: name.clone(),
                        lifetimes: canon_lts,
                        inputs: inputs.iter().map(|t| go(t, &next_env, depth + 1)).collect(),
                        output: Box::new(go(output, &next_env, depth + 1)),
                    }
                }
            }
        }
        fn go(ty: &RustType, env: &[(Lifetime, Lifetime)], depth: usize) -> RustType {
            match ty {
                RustType::Ref(m, l, t) => RustType::Ref(
                    *m,
                    l.as_ref().map(|l| map_lt(l, env)),
                    Box::new(go(t, env, depth)),
                ),
                RustType::Raw(m, t) => RustType::Raw(*m, Box::new(go(t, env, depth))),
                RustType::Boxed(t) => RustType::Boxed(Box::new(go(t, env, depth))),
                RustType::Slice(t) => RustType::Slice(Box::new(go(t, env, depth))),
                RustType::CppPtr(k, t) => RustType::CppPtr(*k, Box::new(go(t, env, depth))),
                RustType::CppConst(t) => RustType::CppConst(Box::new(go(t, env, depth))),
                RustType::CppVar(t) => RustType::CppVar(Box::new(go(t, env, depth))),
                RustType::Tuple(tys) => {
                    RustType::Tuple(tys.iter().map(|t| go(t, env, depth)).collect())
                }
                RustType::Adt(pg) => RustType::Adt(go_path(pg, env, depth)),
                RustType::Cpp(path) => RustType::Cpp(CppTypePath {
                    path: path.path.clone(),
                    generics: path.generics.iter().map(|t| go(t, env, depth)).collect(),
                }),
                RustType::Dyn(tr, bounds) => {
                    RustType::Dyn(go_trait(tr, env, depth), bounds.clone())
                }
                RustType::Impl(tr, bounds) => {
                    RustType::Impl(go_trait(tr, env, depth), bounds.clone())
                }
                RustType::Primitive(_) | RustType::TypeVar(_) => ty.clone(),
            }
        }
        let env: Vec<(Lifetime, Lifetime)> = outer
            .iter()
            .enumerate()
            .map(|(i, l)| (l.clone(), Lifetime(format!("__outer_{i}"))))
            .collect();
        go(self, &env, 0)
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
                PrimitiveRustType::Int(n @ (8 | 16 | 32 | 64)) => format!("::int{n}_t"),
                PrimitiveRustType::Uint(n @ (8 | 16 | 32 | 64)) => format!("::uint{n}_t"),
                PrimitiveRustType::Float(32) => "float".to_owned(),
                PrimitiveRustType::Float(64) => "double".to_owned(),
                PrimitiveRustType::Usize => "::size_t".to_owned(),
                PrimitiveRustType::Bool => "bool".to_owned(),
                p => return Err(format!("`{p}` has no C++ equivalent")),
            },
            RustType::CppPtr(indirection, t) => {
                let sigil = match indirection {
                    CppIndirection::Pointer => "*",
                    CppIndirection::Reference => "&",
                };
                format!("{}{sigil}", t.cpp_spelling()?)
            }
            // `T* const` for a `const` pointer, rather than `const T*`.
            RustType::CppConst(t) if matches!(**t, RustType::CppPtr(..)) => {
                format!("{} const", t.cpp_spelling()?)
            }
            RustType::CppConst(t) => format!("const {}", t.cpp_spelling()?),
            RustType::CppVar(t) => t.cpp_spelling()?,
            ty => {
                return Err(format!(
                    "`{ty}` can't be a C++ template argument (only `c++::` types, builtins and \
                     C++ pointers and references can, `const` or not)"
                ));
            }
        })
    }
}

impl Display for RustPathAndGenerics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let RustPathAndGenerics {
            path,
            lifetimes,
            generics,
            named_generics,
        } = self;
        for p in path {
            if p != "crate" {
                write!(f, "::")?;
            }
            write!(f, "{p}")?;
        }
        if !lifetimes.is_empty() || !generics.is_empty() || !named_generics.is_empty() {
            write!(
                f,
                "::<{}>",
                lifetimes
                    .iter()
                    .map(|x| format!("{x}"))
                    .chain(generics.iter().map(|x| format!("{x}")))
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
                lifetimes,
                inputs,
                output,
            } => {
                if !lifetimes.is_empty() {
                    write!(f, "for{} ", lifetime_params(lifetimes))?;
                }
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
            RustType::Ref(m, lifetime, ty) => {
                write!(f, "&")?;
                if let Some(lifetime) = lifetime {
                    write!(f, "{lifetime} ")?;
                }
                if *m == Mutability::Mut {
                    write!(f, "mut ")?;
                }
                write!(f, "{ty}")
            }
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
            // C++-style pointers and references are raw pointers in Rust.
            RustType::CppPtr(_, pointee) => match &**pointee {
                RustType::CppConst(_) => write!(f, "*const {pointee}"),
                pointee => write!(f, "*mut {pointee}"),
            },
            RustType::CppConst(inner) => write_cpp_object(f, inner, "::zngur_lib::Const"),
            RustType::CppVar(inner) => match &**inner {
                RustType::Primitive(_) | RustType::CppPtr(..) => {
                    write!(f, "::zngur_lib::Cpp<{inner}>")
                }
                // C++ classes (whose constness defaults to non-`const`).
                inner => write!(f, "{inner}"),
            },
        }
    }
}

/// Writes the C++ object `inner` (a `c++::` type, a builtin or a C++ pointer) with the constness
/// `constness` (`::zngur_lib::Const`, or a generic parameter).
fn write_cpp_object(
    f: &mut std::fmt::Formatter<'_>,
    inner: &RustType,
    constness: &str,
) -> std::fmt::Result {
    match inner {
        // C++ classes have a constness parameter: `a::B::<::zngur_lib::Const>`.
        RustType::Cpp(path) => {
            write!(f, "{}::<", path.path.join("::"))?;
            for g in &path.generics {
                write!(f, "{g}, ")?;
            }
            write!(f, "{constness}>")
        }
        // Builtins and pointers.
        inner => write!(f, "::zngur_lib::Cpp<{inner}, {constness}>"),
    }
}

impl RustType {
    /// For an input of a C++-implemented function, if it's a reference, nullable reference or C++
    /// pointer to a `const` C++ object (`&'a const T`, `&'a unique const T`,
    /// `Option<&'a const T>`, `Option<&'a unique const T>`, `const T&` or `const T*`), its Rust
    /// type with the generic constness parameter `constness` in place of `Const`: `&'a T<C1>`,
    /// `&'a mut T<C1>`, `Option<&'a T<C1>>`, `Option<&'a mut T<C1>>` or `*const T<C1>`. C++ only
    /// reads the object, so Rust may pass a non-`const` one too, as C++ does implicitly.
    pub fn generic_const_input(&self, constness: &str) -> Option<String> {
        struct Generic<'a>(&'a RustType, &'a str);
        impl Display for Generic<'_> {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write_cpp_object(f, self.0, self.1)
            }
        }
        match self {
            RustType::Ref(m, lifetime, pointee) => {
                let RustType::CppConst(inner) = &**pointee else {
                    return None;
                };
                let lifetime = lifetime
                    .as_ref()
                    .map(|l| format!("{l} "))
                    .unwrap_or_default();
                let m = if *m == Mutability::Mut { "mut " } else { "" };
                Some(format!("&{lifetime}{m}{}", Generic(inner, constness)))
            }
            RustType::CppPtr(_, pointee) => {
                let RustType::CppConst(inner) = &**pointee else {
                    return None;
                };
                Some(format!("*const {}", Generic(inner, constness)))
            }
            // `Option<&'a const T>` is `Option<&'a T<C1>>`.
            RustType::Adt(pg) if self.as_nullable_cpp_ref().is_some() => {
                let reference = pg.generics[0].generic_const_input(constness)?;
                Some(format!("::{}<{reference}>", pg.path.join("::")))
            }
            _ => None,
        }
    }
}

/// The Rust types of the inputs of a C++-implemented function that Rust calls, and the generic
/// parameters (`<'a, __C1: ::zngur_lib::Constness>`) for its lifetimes and for the constness of
/// each input that is a `const` C++ object (see [`RustType::generic_const_input`]).
pub fn cpp_fn_inputs(lifetimes: &[Lifetime], inputs: &[RustType]) -> (String, Vec<String>) {
    let mut params: Vec<String> = lifetimes.iter().map(|l| l.to_string()).collect();
    let mut n = 0;
    let inputs = inputs
        .iter()
        .map(|ty| {
            let constness = format!("__C{}", n + 1);
            match ty.generic_const_input(&constness) {
                Some(ty) => {
                    n += 1;
                    params.push(format!("{constness}: ::zngur_lib::Constness"));
                    ty
                }
                None => ty.to_string(),
            }
        })
        .collect();
    let params = if params.is_empty() {
        String::new()
    } else {
        format!("<{}>", params.join(", "))
    };
    (params, inputs)
}
