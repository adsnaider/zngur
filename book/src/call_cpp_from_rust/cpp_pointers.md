# C++ pointers and references

C++ functions often take or return pointers and references to C++ objects. In a `.zng` file, you
write them the C++ way: `const T&`, `T&`, `const T*` or `T*`, where `T` is a `#cpp_stack_owned` or
`#cpp_ref` type, or a builtin like `i32`:

```zng
type c++::geo::Point {
    #cpp_stack_owned;
    #layout(size = 8, align = 4);
}

extern "C++" {
    unsafe fn distance(const c++::geo::Point&, const c++::geo::Point&) -> f64;
    unsafe fn normalize(c++::geo::Point*);
    unsafe fn bounds(const c++::geo::Point&, i32*, i32*);
}
```

## In Rust: `CppPtr`

In Rust, all four are a `zngur_lib::CppPtr<T, Const>` or `CppPtr<T, Mut>`, depending on `const`.

C++ pointers and references don't follow Rust's rules: they can be null or dangling, other pointers
can alias them, and `const` only means that the object can't be modified through that particular
pointer. So unlike a Rust reference, a `CppPtr` makes no promises and has no lifetime:

- Creating and copying one is safe: `CppPtr::from(&x)`, `CppPtr::from(&mut x)`, `CppPtr::null()`,
  `CppPtr::from_raw(ptr)`. A `#cpp_stack_owned` object also has `.as_ref()` and `.as_mut()`.
- A `CppPtr<T, Mut>` converts to a `CppPtr<T, Const>` (`.as_const()` or `into()`), but not the
  other way around.
- Anything that dereferences it is `unsafe`: `.as_ref()` and `.as_mut()` on the pointer, which
  return Rust references with a lifetime the caller picks, and calling C++ functions that take one.
  That's why functions with `CppPtr` arguments are usually declared `unsafe`.

```rust
use zngur_lib::CppPtr;

let mut p: Point = make_point(3, 4); // Some C++ function returning a `c++::geo::Point`.
let mut lo = 0;
let mut hi = 0;
unsafe {
    normalize(p.as_mut());
    bounds(p.as_ref(), CppPtr::from(&mut lo), CppPtr::from(&mut hi));
}
```

## In C++: plain pointers

In C++, they're plain pointers to the C++ type. The generated code spells them as
`rust::CppConstPtr<W>` and `rust::CppMutPtr<W>`, which are aliases for `const T*` and `T*`, where
`W` is the Rust wrapper (e.g. `rust::geo::Point`) and `T` the C++ type it stands for
(`::geo::Point`). So you can implement and call these functions with ordinary pointers:

```cpp
double rust::exported_functions::distance(const ::geo::Point* a, const ::geo::Point* b) {
  return std::hypot(a->x - b->x, a->y - b->y);
}
```

A `const T&` and a `const T*` are the same type in Rust and in the generated C++ functions. The
difference only matters for C++ functions that zngur calls directly.
