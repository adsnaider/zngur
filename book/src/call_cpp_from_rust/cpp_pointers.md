# C++ references and pointers

C++ functions often take or return references and pointers to C++ objects. In a `.zng` file, you
write them the C++ way: `const T&`, `T&`, `const T*` or `T*`, where `T` is a `c++::` type or a builtin
like `i32`:

```zng
type c++::geo::Point {
    #trivially_relocatable;
    #layout(size = 8, align = 4);
}

extern "C++" {
    safe fn distance(const c++::geo::Point&, const c++::geo::Point&) -> f64;
    safe fn normalize(c++::geo::Point&);
    unsafe fn bounds(const c++::geo::Point&, i32*, i32*);
}
```

## In Rust: `CppRef` and `CppPtr`

| C++                    | Rust                                                    |
| ---------------------- | ------------------------------------------------------- |
| `const T&` / `T&`      | `zngur_lib::CppRef<T, Const>` / `CppRef<T, Mut>`        |
| `const T*` / `T*`      | `zngur_lib::CppPtr<T, Const>` / `CppPtr<T, Mut>`        |

A **`CppRef`** is like a C++ reference: it always refers to a valid object.

- Making one from a Rust reference is safe: `CppRef::from(&x)`, `CppRef::from(&mut x)`. A C++
  object that Rust holds by value also has `.as_ref()` and `.as_mut()`. Making one from a raw
  pointer (`CppRef::from_raw`) is `unsafe`.
- So functions that only take `CppRef`s can be declared `safe`, and zngur doesn't require them to
  be `unsafe`. Whether they're safe is still up to the `.zng` file: declare a function `unsafe` if
  it has other preconditions.
- It still doesn't follow all of Rust's rules for references: it has no lifetime (the caller keeps
  the object alive), other C++ references can alias it, and `const` only means that the object
  can't be modified through that particular reference. So getting a Rust reference from it
  (`.as_ref()`, `.as_mut()`) is `unsafe`. Copying a `Copy` object (like a builtin) in or out with
  `.read()` and `.write(value)` is safe.

A **`CppPtr`** is like a C++ pointer: it can be null or dangling.

- Creating and copying one is safe: `CppPtr::null()`, `CppPtr::from_raw(ptr)`, or a `CppRef`
  converted with `.as_ptr()` or `into()`.
- Anything that dereferences it is `unsafe`: turning it into a `CppRef` (`.as_cpp_ref()`, which
  returns `None` for null), and `.as_ref()` / `.as_mut()`. Functions with `CppPtr` parameters must be
  declared `unsafe`.

For both, a `Mut` one converts to a `Const` one (`.as_const()` or `into()`), but not the other way
around.

```rust
use zngur_lib::{CppPtr, CppRef};

let mut p: Point = make_point(3, 4); // Some C++ function returning a `c++::geo::Point`.
normalize(p.as_mut());
let d = distance(p.as_ref(), p.as_ref());
let mut lo = 0;
let mut hi = 0;
unsafe {
    bounds(p.as_ref(), CppRef::from(&mut lo).as_ptr(), CppRef::from(&mut hi).as_ptr());
}
```

## In C++: plain pointers

In C++, both are plain pointers to the C++ type. The generated code spells them as
`rust::CppConstPtr<W>` and `rust::CppMutPtr<W>`, which are aliases for `const T*` and `T*`, where
`W` is the Rust wrapper (e.g. `rust::cpp::geo::Point`) and `T` the C++ type it stands for
(`::geo::Point`). So you can implement and call these functions with ordinary pointers:

```cpp
double rust::exported_functions::distance(const ::geo::Point* a, const ::geo::Point* b) {
  return std::hypot(a->x - b->x, a->y - b->y);
}
```

C++ functions that zngur calls directly ([member functions](./cpp_methods.md),
[free functions](./cpp_functions.md), ...) get references or pointers, as declared.
