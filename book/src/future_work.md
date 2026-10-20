# Future work

Ideas and known gaps, mostly in calling C++ from Rust. None of these is designed in detail yet.

## Calling C++ from Rust

- **Checking Rust signatures.** Lifetimes on Rust functions that C++ calls are only documentation;
  a generated check (coercing the function to a `fn` pointer with the declared signature) could
  verify them.
- **Inheritance.** There's no way to convert a reference to a derived class into a reference to its
  base. A possible design: `unsafe` marker traits, which zngur implements for the bases that the
  `.zng` file declares (e.g. `unsafe impl Derives<Base> for Derived`), and which let a reference cast
  itself to the base (`&Derived` to `&Base`) with the C++ pointer adjustment.
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
- **More builtin types.** `char` (whose signedness varies), `void*`, and rvalue references (`T&&`)
  can't be written in a `.zng` file yet.

## `.zng` files

- **Numbers in `cfg!` paths.** `cfg!(target_pointer_width.64)` doesn't parse; the value has to be
  quoted: `cfg!(target_pointer_width."64")`.
