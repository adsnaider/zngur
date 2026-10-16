# C++ references and pointers

C++ functions often take or return references and pointers. In a `.zng` file, C++ references
whose lifetime you know are written as Rust references, and the others the C++ way, which makes
them raw pointers in Rust:

| `.zng`                  | Rust                                     | C++                     |
| ----------------------- | ---------------------------------------- | ----------------------- |
| `&'a T`                 | `&'a T<Const>` (`&'a i32` for builtins)  | `const T&`              |
| `&'a mut T`             | `&'a mut T`                              | `T&`                    |
| `const T&` / `const T*` | `*const T<Const>` (`*const i32`)         | `const T&` / `const T*` |
| `T&` / `T*`             | `*mut T`                                 | `T&` / `T*`             |

`T` is a `c++::` type or a builtin like `i32`.

```zng
type c++::geo::Point {
    #trivially_relocatable;
    #layout(size = 8, align = 4);

    safe fn x_ref<'a>(&'a self) -> &'a i32;
}

mod c++::geo {
    safe fn distance(&c++::geo::Point, &c++::geo::Point) -> f64;
    safe fn normalize(&mut c++::geo::Point);
    unsafe fn bounds(&c++::geo::Point, i32*, i32*);
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
  (`&Point` to `&Point<Const>`), `p.as_const_mut()` (`&mut Point` to `&mut Point<Const>`, exclusive
  access to a `const` object), and `p.into_const()` (by value).
- Builtins don't have a constness parameter: `const int&` is just `&'a i32`. Where a `const`
  builtin has to be named as a type, i.e. as a C++ template argument, it's
  `zngur_lib::ConstValue<i32>` (`c++::std::span<const i32>` is `std::span<ConstValue<i32>>`).

## References and lifetimes

A Rust reference in a `.zng` file is a promise from the person who writes it: the C++ reference is
valid for that lifetime, and (for `&mut`) nothing else accesses the object meanwhile. zngur can't
check it against C++, so name the lifetimes explicitly ([Lifetimes](../lifetimes.md)). References
without a lifetime are elided like in Rust, with a warning (`elided_lifetimes`), which
`#allow(elided_lifetimes)` at the top of a `.zng` file silences.

Shared references to non-`const` objects (an aliasing `T&`) and exclusive access to `const` objects
can't be declared yet.

## Raw pointers

C++-style references and pointers, without a lifetime, are raw pointers in Rust. They can be null
or dangling, so anything that dereferences them is `unsafe`. Rust references coerce to them, so you
can pass `&x` / `&mut x` / `p.as_const()` directly.

Whether a function is `safe` is up to the `.zng` file: zngur warns about `safe` functions that take
raw pointers (`safe_raw_pointer`, silenced with `#allow(safe_raw_pointer)`), since they should
usually be `unsafe`.

```rust
let mut p: Point = generated::geo::origin();
generated::geo::normalize(&mut p);
let d = generated::geo::distance(p.as_const(), p.as_const());
let (mut lo, mut hi) = (0, 0);
unsafe { generated::geo::bounds(p.as_const(), &mut lo, &mut hi) };
```

## In C++

C++ functions that zngur calls directly ([member functions](./cpp_methods.md),
[free functions](./cpp_functions.md), ...) get references or pointers, as declared: Rust references
and C++-style references are C++ references, and pointers are pointers.

Functions in `extern "C++"` blocks, which you write in C++ for Rust, get raw pointers as plain C++
pointers. The generated code spells them `rust::CppPtr<W>` and `rust::CppPtr<rust::CppConst<W>>`,
which are aliases for `T*` and `const T*`, where `W` is the Rust wrapper (e.g.
`rust::cpp::geo::Point`) and `T` the C++ type it stands for (`::geo::Point`):

```cpp
double rust::exported_functions::distance(const ::geo::Point* a, const ::geo::Point* b) {
  return std::hypot(a->x - b->x, a->y - b->y);
}
```
