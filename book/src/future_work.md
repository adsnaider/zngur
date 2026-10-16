# Future work

Ideas and known gaps, mostly in calling C++ from Rust. None of these is designed in detail yet.

## Calling C++ from Rust

- **Lifetimes on `CppRef`.** A [`CppRef`](./call_cpp_from_rust/cpp_pointers.md) refers to a valid
  object, but has no lifetime, so keeping the object alive is up to the caller. A lifetime
  (`CppRef<'a, T>`, made from `&'a T`) would let the compiler check that, at least for references
  made from Rust references and for references returned by functions (with Rust's elision rules).
- **Lifetimes on C++ functions.** [Lifetimes](./lifetimes.md) work for Rust signatures (`extern
  "C++"` functions, trait methods, ...), but not yet for the C++ functions that Rust calls directly,
  whose references are `CppRef`s.
- **Method-call syntax.** C++ member functions are associated functions (`Point::norm(p.as_ref())`).
  With `self: CppRef<Self>` receivers, they could be methods (`p.as_ref().norm()`). That needs the
  `arbitrary_self_types` feature, which is still unstable.
- **Inheritance.** There's no way to convert a reference to a derived class into a reference to its
  base. A possible design: `unsafe` marker traits, which zngur implements for the bases that the
  `.zng` file declares (e.g. `unsafe impl Derives<Base> for Derived`), and which let a `CppRef` cast
  itself to the base (`CppRef<Derived>` to `CppRef<Base>`) with the C++ pointer adjustment.
- **C++ enums.** Not supported yet. They need a design discussion: how to represent values that
  aren't enumerators, scoped and unscoped enums, and the underlying type.
- **Exceptions.** The C++ glue doesn't catch them, so an exception that reaches Rust aborts the
  program. Converting them (to panics, or to a `Result`) needs a proposal.
- **Functions in the global namespace.** C++ functions need a namespace (`mod c++::ns { ... }`).
  Global ones could be declared in `mod c++ { ... }`, for Rust functions at the root of the
  generated `c++::` tree.
- **More template arguments.** [Class templates](./call_cpp_from_rust/cpp_templates.md) don't take
  non-type arguments (`std::array<T, 4>`), function types or nested types yet. Alias templates
  cover many of these cases today. (`const` arguments, like `std::span<const i32>`, work.)
- **Implicit `const` conversions.** Going from a `T` to a `Const<T>` is explicit (`.as_const()`).
  A conversion trait, used by the generated functions, could make it implicit.
- **More builtin types.** `char` (whose signedness varies), `void*`, and rvalue references (`T&&`)
  can't be written in a `.zng` file yet.

## `.zng` files

- **Numbers in `cfg!` paths.** `cfg!(target_pointer_width.64)` doesn't parse; the value has to be
  quoted: `cfg!(target_pointer_width."64")`.
