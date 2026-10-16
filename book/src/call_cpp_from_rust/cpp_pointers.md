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

## In Rust: `CppRef`, `CppPtr` and `Const`

In C++, `const` is part of the type: `const T&` is a reference to a `const T`. zngur does the same:
`const T` is `zngur_lib::Const<T>` in Rust.

| C++                    | Rust                                            |
| ---------------------- | ----------------------------------------------- |
| `const T&` / `T&`      | `zngur_lib::CppRef<Const<T>>` / `CppRef<T>`     |
| `const T*` / `T*`      | `zngur_lib::CppPtr<Const<T>>` / `CppPtr<T>`     |
| `const T`              | `zngur_lib::Const<T>`                           |

`Const<T>` has the same layout as `T`, and works for `c++::` types, builtins like `i32`, and C++
pointers (`T* const`). A `const T` can't be passed by value, but it can be a C++ template argument:
`type c++::std::span<const i32> { ... }` is `std::span<Const<i32>>` in Rust.

Only the `const` [member functions](./cpp_methods.md) of a C++ type are available through a
`Const<T>` or a `CppRef<Const<T>>`. So there's no `Deref` from `Const<T>` to `T`, and going from a
`T` to a `const T` is explicit:

- `x.as_const()` turns a `&T` into a `&Const<T>`, and `x.as_const_mut()` a `&mut T` into a
  `&mut Const<T>` (still `const`, but nothing else can access it). Both come from the
  `zngur_lib::ZngCppObject` trait.
- `r.as_const()` turns a `CppRef<T>` into a `CppRef<Const<T>>`, and likewise for `CppPtr`.
- `Const::new(x)` makes a `const` object out of `x`.

A **`CppRef`** is like a C++ reference: it always refers to a valid object.

- Making one from a Rust reference is safe: `CppRef::from(&x)` is a `CppRef<Const<T>>`, and
  `CppRef::from(&mut x)` a `CppRef<T>` (or a `CppRef<Const<T>>` for a `&mut Const<T>`). A C++
  object that Rust holds by value also has `.as_ref()` (`const T&`) and `.as_mut()` (`T&`). Making
  one from a raw pointer (`CppRef::from_raw`, `CppRef::from_const_raw`) is `unsafe`.
- So functions that only take `CppRef`s can be declared `safe`, and zngur doesn't require them to
  be `unsafe`. Whether they're safe is still up to the `.zng` file: declare a function `unsafe` if
  it has other preconditions.
- It still doesn't follow all of Rust's rules for references: it has no lifetime (the caller keeps
  the object alive), other C++ references can alias it, and `const` only means that the object
  can't be modified through that particular reference. So getting a Rust reference from it
  (`.as_ref()`, `.as_mut()`) is `unsafe`. Copying a builtin or a pointer in or out with `.read()`
  and `.write(value)` is safe; references to `const` only have `.read()`.

A **`CppPtr`** is like a C++ pointer: it can be null or dangling.

- Creating and copying one is safe: `CppPtr::null()`, `CppPtr::from_raw(ptr)`,
  `CppPtr::from_const_raw(ptr)`, or a `CppRef` converted with `.as_ptr()` or `into()`.
- Anything that dereferences it is `unsafe`: turning it into a `CppRef` (`.as_cpp_ref()`, which
  returns `None` for null), and `.as_ref()` / `.as_mut()`. Functions with `CppPtr` parameters must be
  declared `unsafe`.

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
`rust::CppPtr<W>` and `rust::CppPtr<rust::CppConst<W>>`, which are aliases for `T*` and `const T*`,
where `W` is the Rust wrapper (e.g. `rust::cpp::geo::Point`) and `T` the C++ type it stands for
(`::geo::Point`). So you can implement and call these functions with ordinary pointers:

```cpp
double rust::exported_functions::distance(const ::geo::Point* a, const ::geo::Point* b) {
  return std::hypot(a->x - b->x, a->y - b->y);
}
```

C++ functions that zngur calls directly ([member functions](./cpp_methods.md),
[free functions](./cpp_functions.md), ...) get references or pointers, as declared.
