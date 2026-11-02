# Future work

Ideas and known gaps, mostly in calling C++ from Rust. None of these is designed in detail yet.

## Calling C++ from Rust

- **Inheritance.** There's no way to convert a reference to a derived class into a reference to its
  base. A possible design: `unsafe` marker traits, which Zngur implements for the bases that the
  `.zng` file declares (e.g. `unsafe impl Derives<Base> for Derived`), and which let a reference cast
  itself to the base (`&Derived` to `&Base`) with the C++ pointer adjustment.
- **C++ enums.** Not supported yet. They need a design discussion: how to represent values that
  aren't enumerators, scoped and unscoped enums, and the underlying type.
- **Exceptions.** The generated `extern "C"` C++ glue is neither `try`/`catch`-wrapped nor
  `noexcept`, and Rust declares those bridges `extern "C"` rather than `extern "C-unwind"`, so an
  uncaught C++ exception unwinding into Rust is undefined behavior today. Converting exceptions (to
  panics, or to a `Result`) or marking bridges `noexcept` needs a proposal.
- **Functions in the global namespace.** C++ functions need a namespace (`c++::ns::f` or
  `mod c++::ns { ... }`). Global ones could be declared as `c++::f` or in `mod c++ { ... }`, for
  Rust functions at the root of the `generated` module.
- **More template arguments.** [Class templates](./call_cpp_from_rust/cpp_templates.md) don't take
  non-type arguments (`std::array<T, 4>`), function types or nested types yet. Alias templates
  cover many of these cases today. (`const` arguments, like `std::span<const i32>`, work.)
- **More builtin types.** C++ `char` (which may be signed or unsigned depending on the platform),
  `void*`, and rvalue references (`T&&`) can't be written in a `.zng` file yet.

## Calling Rust from C++

- **Checking Rust signatures.** Lifetimes on Rust functions that C++ calls are only documentation;
  a generated check (coercing the function to a `fn` pointer with the declared signature) could
  verify them.

## `.zng` files

- **Numbers in `cfg!` paths.** `cfg!(target_pointer_width.64)` doesn't parse; the value has to be
  quoted: `cfg!(target_pointer_width."64")`.
