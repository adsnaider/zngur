# Free C++ functions

zngur can call free C++ functions directly, with no C++ code to write. Declare them with a `c++::`
path, or inside a `mod c++::<namespace>` block, which gives their C++ namespace:

```zng
mod c++::geo {
    safe fn distance<'a>(&'a const c++::geo::Point, &'a const c++::geo::Point) -> f64;
    unsafe fn normalize(c++::geo::Point*);
    safe fn longer<'a>(&'a const c++::geo::Line, &'a const c++::geo::Line) -> &'a const c++::geo::Line;
    safe fn origin() -> c++::geo::Point;
    safe fn scale(f64) -> f64;
    safe fn scale(i32) -> i32 as scale_i32;
    safe fn detail::clamp(f64) -> f64;
}

safe fn c++::geo::midpoint<'a>(
    &'a const c++::geo::Point,
    &'a const c++::geo::Point,
) -> c++::geo::Point;
```

- The parameter and return types are the C++ ones, written as zngur types: builtins like `i32`,
  C++ types that Rust holds by value, and [C++ references and pointers](./cpp_pointers.md).
- `as` gives the Rust function a different name, e.g. for C++ overloads or Rust keywords.
- Lifetimes are declared like in Rust (`fn longer<'a>(...)`), and should be named, since zngur
  can't check them against C++ ([Lifetimes](../lifetimes.md)).
- Each function is `safe` or `unsafe`, like the functions in `extern "C++"` blocks. zngur warns about
  `safe` functions with raw-pointer parameters, which may be null or invalid. References are fine.
- Free C++ functions must be in a named namespace (`c++::<namespace>::f` or inside
  `mod c++::<namespace>`); functions in the global namespace aren't supported.

In Rust, each one is a function at the same path in the `c++::` tree as its namespace:

```rust
let o = generated::geo::origin();
let d = generated::geo::distance(&o, &p);
```

zngur generates the C++ function that calls it, like for
[C++ member functions](./cpp_methods.md): overloads and default arguments work as in C++, the
return type is checked with a `static_assert`, and arguments are converted as in any C++ call, so
declare them exactly.

Unlike functions in `extern "C++"` blocks, which you implement in C++ for Rust, these are existing
C++ functions.

## Function templates

Instantiations of C++ function templates are declared with explicit template arguments, which are
zngur types like those of [class templates](./cpp_templates.md). Give each instantiation its own
Rust name with `as`:

```zng
mod c++::std {
    safe fn make_unique<c++::geo::Point>(i32, i32) -> c++::std::unique_ptr<c++::geo::Point> as make_point;
    safe fn make_unique<c++::geo::Canvas>() -> c++::std::unique_ptr<c++::geo::Canvas> as make_canvas;
}
```

zngur generates a non-generic Rust function for each declared instantiation that calls
`::std::make_unique< ::geo::Point >(...)`, so the parameter and return types in `.zng` must be
written out concretely for that call (here, the constructor's arguments), while C++ still deduces
any remaining trailing template parameters (such as `Args...` in `std::make_unique`) from the call
arguments as usual.

`std::make_unique` is especially useful for C++ types that Rust can only refer to (with no
`#layout`), like `Canvas` above: Rust can't construct them in place, but it can own one on the heap
through a `std::unique_ptr`. See `examples/cpp_make_unique`.
