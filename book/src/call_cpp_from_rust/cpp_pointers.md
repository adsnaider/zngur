# C++ references and pointers

C++ functions often take or return references and pointers. In a `.zng` file, C++ references
whose lifetime you know are written as references to C++ objects, and the others the C++ way,
which makes them raw pointers in Rust:

| `.zng`                    | C++                                            | Rust                                           |
| ------------------------- | ---------------------------------------------- | ---------------------------------------------- |
| `&'a const T`             | `const T&`                                     | `&'a T<Const>` (`&'a Cpp<i32, Const>`)         |
| `&'a var T`               | `T&`, shared                                   | `&'a T` (`&'a Cpp<i32>`)                       |
| `&'a unique var T`        | `T&`, unique                                   | `&'a mut T` (`&'a mut Cpp<i32>`)               |
| `&'a unique const T`      | `const T&`, unique                             | `&'a mut T<Const>` (`&'a mut Cpp<i32, Const>`) |
| `const T&` / `const T*`   | `const T&` / `const T*`                        | `*const T<Const>` (`*const Cpp<i32, Const>`)   |
| `T&` / `T*`               | `T&` / `T*`                                    | `*mut T` (`*mut Cpp<i32>`)                     |
| `&'a i32` / `&'a mut i32` | `rust::Ref<int32_t>` / `rust::RefMut<int32_t>` | `&'a i32` / `&'a mut i32`                      |

`T` is a `c++::` type, a builtin like `i32` (the Rust type in parentheses) or a C++ pointer. The
last row is a reference to a Rust value rather than to a C++ object, which C++ must treat with
Rust's rules; see [C++ objects vs Rust values](./cpp_objects.md) for the difference, and for
`zngur_lib::Cpp<T>`, the Rust type of builtins that are C++ objects.

```zng
type c++::geo::Point {
    #trivially_relocatable;
    #layout(size = 8, align = 4);
}

extern "C++" {
    safe fn distance<'a>(&'a const c++::geo::Point, &'a const c++::geo::Point) -> f64;
    safe fn normalize<'a>(&'a unique var c++::geo::Point);
    unsafe fn bounds<'a>(&'a const c++::geo::Point, i32*, i32*);
}
```

## `const` is part of the type

Every C++ class gets a constness parameter in Rust, which defaults to non-`const`: `Point` is
`Point<NonConst>` (a C++ `Point`), and `Point<Const>` is a `const Point` (`zngur_lib::Const` and
`zngur_lib::NonConst`).

- Going from a `Point` to a `Point<Const>` is explicit, and there's no way back: `p.as_const()`
  (`&Point` to `&Point<Const>`), `p.as_const_mut()` (`&mut Point` to `&mut Point<Const>`, unique
  access to a `const` object), and `p.into_const()` (by value).
- Except for arguments: like C++, which makes a non-`const` object `const` when you pass it, the
  arguments of C++ functions that are `const` objects take both constnesses. They're generic over
  it: `&'a const Point` is a `&'a Point<C1>` there (for any `C1: Constness`),
  `&'a unique const Point` a `&'a mut Point<C1>`, and a raw `const Point&` a `*const Point<C1>`, so
  `distance(&p, &q)` works whether `p` and `q` are `const` or not. C++ only reads them, so that's
  sound. Return values and receivers keep their constness.
- Builtins have the same parameter through `zngur_lib::Cpp<T, C>`: `const int32_t&` is
  `&'a Cpp<i32, Const>` (and `&'a Cpp<i32, C1>` as an argument).

## References and lifetimes

A reference in a `.zng` file is a promise from the person who writes it: the C++ reference is
valid for that lifetime, and, for `unique` references, nothing else accesses the object meanwhile.
zngur can't check it against C++, so name the lifetimes explicitly ([Lifetimes](../lifetimes.md)).
References without a lifetime are elided like in Rust.

Shared references are the default: `&'a var T` is a `T&` that others may share (see
[C++ objects vs Rust values](./cpp_objects.md)).

## Raw pointers

C++-style references and pointers, without a lifetime, are raw pointers in Rust. They can be null
or dangling, so anything that dereferences them is `unsafe`. Rust references coerce to them, so you
can pass `&x` / `&mut x` directly (for a `const T&`, whether `x` is `const` or not), and
`Cpp::from_mut(&mut n)` for a Rust `i32`.

Whether a function is `safe` is up to the `.zng` file: functions that dereference raw pointers
should be `unsafe`.

```rust
generated::normalize(&mut p);
let d = generated::distance(&p, &p);
let (mut lo, mut hi) = (0, 0);
unsafe { generated::bounds(&p, Cpp::from_mut(&mut lo), Cpp::from_mut(&mut hi)) };
```

## In C++

Each side sees its own language's form: Rust sees `&'a T`, and C++ sees `T&`, whichever side calls
and whichever implements.

Functions in `extern "C++"` blocks, which you write in C++ for Rust, and Rust functions that C++
calls, take and return references to C++ objects as C++ references, and raw pointers as C++
pointers. The generated code spells them `rust::CppRef<W>` / `rust::CppRef<rust::CppConst<W>>` and
`rust::CppPtr<W>` / `rust::CppPtr<rust::CppConst<W>>`, which are aliases for `T&` / `const T&` and
`T*` / `const T*`, where `W` is the Rust wrapper (e.g. `rust::cpp::geo::Point`) and `T` the C++
type it stands for (`::geo::Point`). The receivers of `extern "C++"` impls of `c++::` types
(`&self`, `&mut self`) are `T&`:

```cpp
// `safe fn distance<'a>(&'a const c++::geo::Point, &'a const c++::geo::Point) -> f64;`
double rust::exported_functions::distance(const ::geo::Point& a, const ::geo::Point& b) {
  return std::hypot(a.x - b.x, a.y - b.y);
}
```

References to Rust values (`&crate::Foo`, `&str`, `Option<&crate::Foo>`) are still
`rust::Ref<T>` / `rust::RefMut<T>` (see [name mapping](../call_rust_from_cpp/name_mapping.md)).
