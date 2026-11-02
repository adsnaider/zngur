# Calling C++ from Rust

Until this point, all Zngur features discussed were about using Rust code inside C++,
but no C++/Rust interop tool is complete without supporting the C++ to Rust direction.
In fact, this direction is arguably more important since in a C++/Rust project most of the existing code is in C++
and Rust code needs a way to use it in order to be useful.
So Zngur also supports this direction.

Zngur gives Rust two ways to use C++ code:

- **Call C++ directly.** Declare C++ types with a `c++::` path, and Rust can use them:
  - [hold them by value](./opaque.md), if they can be moved with `memcpy`, or refer to them otherwise;
  - call their [member functions](./cpp_methods.md) and [constructors](./cpp_constructors.md), and
    access their [fields](./cpp_fields.md);
  - call [free C++ functions](./cpp_functions.md), including instantiations of function templates;
  - use instantiations of [class templates](./cpp_templates.md), like `std::unique_ptr<T>`.

  C++ references are [references to C++ objects](./cpp_objects.md) (`&'a const T`,
  `&'a unique var T`, ..., with lifetimes that you declare) or [raw pointers](./cpp_pointers.md),
  and `const` is part of the type (`Point<Const>`, `Cpp<i32, Const>`). Zngur generates the C++
  code that makes the calls, so there's nothing to write in C++.
- **Write a Rusty API in C++.** [`impl` blocks](./rust_impl.md) and [functions](./function.md)
  written in C++ for Rust or `c++::` types, and C++ classes behind
  [trait objects](./trait_object.md), let you shape the API that Rust sees.

Calling C++ directly is the quickest way to use a C++ library. But C++ APIs don't follow Rust's
rules: they have no lifetimes, and `const` doesn't mean "not mutated". So Rust code that uses a
C++ library heavily usually still wants a safe, idiomatic wrapper. Zngur lets you write that
wrapper in Rust, on top of the direct calls, or in C++, with the second set of features.
