# Free C++ functions

zngur can call free C++ functions directly, with no C++ code to write. Declare them inside a
`mod c++::<namespace>` block, which is their C++ namespace:

```zng
mod c++::geo {
    safe fn distance(const c++::geo::Point&, const c++::geo::Point&) -> f64;
    unsafe fn normalize(c++::geo::Point*);
    safe fn origin() -> c++::geo::Point;
    safe fn scale(f64) -> f64;
    safe fn scale(i32) -> i32 as scale_i32;
}
```

- The parameter and return types are the C++ ones, written as zngur types: builtins like `i32`,
  C++ types that Rust holds by value, and [C++ references and pointers](./cpp_pointers.md).
- `as` gives the Rust function a different name, e.g. for C++ overloads, which Rust doesn't have.
- Each function is `safe` or `unsafe`, like the functions in `extern "C++"` blocks. Functions with
  C++ pointer parameters, which may be null or invalid, must be `unsafe`. C++ references are fine.
- Functions in the global namespace aren't supported.

In Rust, each one is a function at the same path in the `c++::` tree as its namespace:

```rust
let o = generated::geo::origin();
let d = generated::geo::distance(o.as_ref(), p.as_ref());
```

zngur generates the C++ function that calls it, like for
[C++ member functions](./cpp_methods.md): overloads and default arguments work as in C++, the
return type is checked with a `static_assert`, and arguments are converted as in any C++ call, so
declare them exactly.

Unlike functions in `extern "C++"` blocks, which you implement in C++ for Rust, these are existing
C++ functions.
