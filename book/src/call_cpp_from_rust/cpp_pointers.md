# C++ references and pointers

C++ functions often take or return references and pointers. In a `.zng` file, C++ references
whose lifetime you know are written as references to C++ objects, and the others the C++ way,
which makes them raw pointers in Rust:

| `.zng`                    | C++                           | Rust                                           |
| ------------------------- | ----------------------------- | ---------------------------------------------- |
| `&'a const T`             | `const T&`                    | `&'a T<Const>` (`&'a Cpp<i32, Const>`)         |
| `&'a var T`               | `T&`, shared                  | `&'a T` (`&'a Cpp<i32>`)                       |
| `&'a unique var T`        | `T&`, unique                  | `&'a mut T` (`&'a mut Cpp<i32>`)               |
| `&'a unique const T`      | `const T&`, unique            | `&'a mut T<Const>` (`&'a mut Cpp<i32, Const>`) |
| `const T&` / `const T*`   | `const T&` / `const T*`       | `*const T<Const>` (`*const Cpp<i32, Const>`)   |
| `T&` / `T*`               | `T&` / `T*`                   | `*mut T` (`*mut Cpp<i32>`)                     |
| `&'a i32` / `&'a mut i32` | `const int32_t&` / `int32_t&` | `&'a i32` / `&'a mut i32`                      |

`T` is a `c++::` type, a builtin like `i32` (the Rust type in parentheses) or a C++ pointer. The
last row is a reference to a Rust value rather than to a C++ object, which C++ must treat with
Rust's rules; see [C++ objects vs Rust values](./cpp_objects.md) for the difference, and for
`zngur_lib::Cpp<T>`, the Rust type of builtins that are C++ objects.

```zng
type c++::geo::Point {
    #trivially_relocatable;
    #layout(size = 8, align = 4);

    safe fn x_ref<'a>(&'a const self) -> &'a const i32;
}

mod c++::geo {
    safe fn distance<'a>(&'a const c++::geo::Point, &'a const c++::geo::Point) -> f64;
    safe fn normalize<'a>(&'a unique var c++::geo::Point);
    unsafe fn bounds<'a>(&'a const c++::geo::Point, i32*, i32*);
}
```

## `const` is part of the type

Every C++ class gets a constness parameter in Rust, which defaults to non-`const`: `Point` is
`Point<NonConst>` (a C++ `Point`), and `Point<Const>` is a `const Point` (`zngur_lib::Const` and
`zngur_lib::NonConst`).

- `const` [member functions](./cpp_methods.md) are available on both, and the others only on
  `Point<NonConst>`. So a `&Point<Const>` (a C++ `const Point&`) can only call `const` member
  functions, even through a `&mut Point<Const>`.
- Going from a `Point` to a `Point<Const>` is explicit, and there's no way back: `p.as_const()`
  (`&Point` to `&Point<Const>`), `p.as_const_mut()` (`&mut Point` to `&mut Point<Const>`, unique
  access to a `const` object), and `p.into_const()` (by value).
- Builtins have the same parameter through `zngur_lib::Cpp<T, C>`: `const int32_t&` is
  `&'a Cpp<i32, Const>`. A `const` builtin as a C++ template argument is a `Cpp<i32, Const>` too
  (`c++::std::span<const i32>` is `std::span<Cpp<i32, Const>>`), while a non-`const` one is just
  `i32` (`std::span<i32>`).

## References and lifetimes

A reference in a `.zng` file is a promise from the person who writes it: the C++ reference is
valid for that lifetime, and, for `unique` references, nothing else accesses the object meanwhile.
zngur can't check it against C++, so name the lifetimes explicitly ([Lifetimes](../lifetimes.md)).
References without a lifetime are elided like in Rust, with a warning (`elided_lifetimes`), which
`#allow(elided_lifetimes)` at the top of a `.zng` file silences.

Shared references are the default: `&'a var T` is a `T&` that others may share, through which C++
member functions that aren't `const` can still be called (see
[C++ objects vs Rust values](./cpp_objects.md)).

## Raw pointers

C++-style references and pointers, without a lifetime, are raw pointers in Rust. They can be null
or dangling, so anything that dereferences them is `unsafe`. Rust references coerce to them, so you
can pass `&x` / `&mut x` / `p.as_const()` directly, and `Cpp::from_mut(&mut n)` for a Rust `i32`.

Whether a function is `safe` is up to the `.zng` file: zngur warns about `safe` functions that take
raw pointers (`safe_raw_pointer`, silenced with `#allow(safe_raw_pointer)`), since they should
usually be `unsafe`.

```rust
let mut p: Point = generated::geo::origin();
generated::geo::normalize(&mut p);
let d = generated::geo::distance(p.as_const(), p.as_const());
let (mut lo, mut hi) = (0, 0);
unsafe { generated::geo::bounds(p.as_const(), Cpp::from_mut(&mut lo), Cpp::from_mut(&mut hi)) };
```

## In C++

C++ functions that zngur calls directly ([member functions](./cpp_methods.md),
[free functions](./cpp_functions.md), ...) get references or pointers, as declared: references
(to C++ objects or to Rust values) and C++-style references are C++ references, and pointers are
pointers.

Functions in `extern "C++"` blocks, which you write in C++ for Rust, get raw pointers as plain C++
pointers. The generated code spells them `rust::CppPtr<W>` and `rust::CppPtr<rust::CppConst<W>>`,
which are aliases for `T*` and `const T*`, where `W` is the Rust wrapper (e.g.
`rust::cpp::geo::Point`) and `T` the C++ type it stands for (`::geo::Point`):

```cpp
double rust::exported_functions::distance(const ::geo::Point* a, const ::geo::Point* b) {
  return std::hypot(a->x - b->x, a->y - b->y);
}
```

They get references (to C++ objects or not) as `rust::Ref<W>` / `rust::RefMut<W>`, whose
constness isn't part of their C++ type.
