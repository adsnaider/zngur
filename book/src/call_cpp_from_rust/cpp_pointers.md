# C++ references and pointers

C++ functions often take or return references and pointers. In a `.zng` file, C++ references
whose lifetime you know are written as references to C++ objects, C++ pointers whose lifetime you
know (and which may be null) as `Option`s of them, and the others the C++ way, which makes them raw
pointers in Rust:

| `.zng`                       | C++                                                                                                 | Rust                                           |
| ---------------------------- | --------------------------------------------------------------------------------------------------- | ---------------------------------------------- |
| `&'a const T`                | `const T&`                                                                                          | `&'a T<Const>` (`&'a Cpp<i32, Const>`)         |
| `&'a var T`                  | `T&`, shared                                                                                        | `&'a T` (`&'a Cpp<i32>`)                       |
| `&'a unique var T`           | `T&`, unique                                                                                        | `&'a mut T` (`&'a mut Cpp<i32>`)               |
| `&'a unique const T`         | `const T&`, unique                                                                                  | `&'a mut T<Const>` (`&'a mut Cpp<i32, Const>`) |
| `Option<&'a const T>`        | `const T*`                                                                                          | `Option<&'a T<Const>>`                         |
| `Option<&'a var T>`          | `T*`, shared                                                                                        | `Option<&'a T>`                                |
| `Option<&'a unique var T>`   | `T*`, unique                                                                                        | `Option<&'a mut T>`                            |
| `Option<&'a unique const T>` | `const T*`, unique                                                                                  | `Option<&'a mut T<Const>>`                     |
| `const T&` / `const T*`      | `const T&` / `const T*`                                                                             | `*const T<Const>` (`*const Cpp<i32, Const>`)   |
| `T&` / `T*`                  | `T&` / `T*`                                                                                         | `*mut T` (`*mut Cpp<i32>`)                     |
| `&'a i32` / `&'a mut i32`    | `const int32_t&` / `int32_t&` (or `rust::Ref<int32_t>` / `rust::RefMut<int32_t>` in `extern "C++"`) | `&'a i32` / `&'a mut i32`                      |

`T` is a `c++::` type, a builtin like `i32` (the Rust type in parentheses) or a C++ pointer, and
`Option` in `.zng` stands for `::std::option::Option` or `::core::option::Option`. The last row is
a reference to a Rust value rather than to a C++ object, which C++ must treat with Rust's rules; see
[C++ objects vs Rust values](./cpp_objects.md) for the difference, and for `zngur_lib::Cpp<T>`, the
Rust type of builtins that are C++ objects.

```zng
type c++::geo::Point {
    #trivially_relocatable;
    #layout(size = 8, align = 4);

    safe fn x_ref<'a>(&'a const self) -> &'a const i32;
}

mod c++::geo {
    safe fn origin() -> c++::geo::Point;
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
- Except for arguments: like C++, which makes a non-`const` object `const` when you pass it, the
  arguments of C++ functions that are `const` objects take both constnesses. They're generic over
  it: `&'a const Point` is a `&'a Point<C1>` there (for any `C1: Constness`),
  `&'a unique const Point` a `&'a mut Point<C1>`, and a raw `const Point&` a `*const Point<C1>`, so
  `distance(&p, &q)` works whether `p` and `q` are `const` or not. C++ only reads them, so that's
  sound. Return values, receivers and fields keep their constness.
- Builtins have the same parameter through `zngur_lib::Cpp<T, C>`: `const int32_t&` is
  `&'a Cpp<i32, Const>` (and `&'a Cpp<i32, C1>` as an argument).

## References and lifetimes

A reference in a `.zng` file is a promise from the person who writes it: the C++ reference is
valid for that lifetime, and, for `unique` references, nothing else accesses the object meanwhile.
zngur can't check it against C++, so name the lifetimes explicitly ([Lifetimes](../lifetimes.md)).
References without a lifetime are elided like in Rust, with a warning (`elided_lifetimes`) on C++
functions that zngur calls directly (member functions, free functions, and constructors), which
`#allow(elided_lifetimes)` at the top of a `.zng` file silences.

Shared references are the default: `&'a var T` is a `T&` that others may share, through which C++
member functions that aren't `const` can still be called (see
[C++ objects vs Rust values](./cpp_objects.md)).

## Nullable references

A C++ pointer that is either null or valid for a known lifetime is a nullable reference:
`Option<&'a const T>` / `Option<&'a var T>` / `Option<&'a unique var T>` (`Option` is
`::std::option::Option` or `::core::option::Option`). It's a `const T*` / `T*` in C++, and an
`Option<&'a T<Const>>` / `Option<&'a T>` / `Option<&'a mut T>` in Rust, so functions that take or
return one can be `safe`, like those with references: `None` is null. The same promises apply (the
lifetime, and uniqueness for `unique`), and so does implicit `const` for arguments: an
`Option<&'a const T>` argument takes `Some(&x)` whether `x` is `const` or not, which also means
that a bare `None` needs a type (`None::<&T>`).

Nullable references only describe C++ pointers, so they can't be declared as types
(`type ::std::option::Option<&var c++::T>` is an error); they're used in signatures directly.

## Raw pointers

C++-style references and pointers, without a lifetime, are raw pointers in Rust. They can be null
or dangling, so anything that dereferences them is `unsafe`. Rust references coerce to them, so you
can pass `&x` / `&mut x` directly (for a `const T&`, whether `x` is `const` or not), and
`Cpp::from_mut(&mut n)` for a Rust `i32`.

Whether a function is `safe` is up to the `.zng` file: functions that dereference raw pointers
should be `unsafe`, and zngur warns (`safe_raw_pointer`) when a `safe` C++ function that zngur
calls directly (member function, free function, or constructor) takes a raw pointer or a `safe`
[field](./cpp_fields.md) accessor exposes mutable access to one, which `#allow(safe_raw_pointer)`
silences.

```rust
let mut p: Point = generated::geo::origin();
generated::geo::normalize(&mut p);
let d = generated::geo::distance(&p, &p);
let (mut lo, mut hi) = (0, 0);
unsafe { generated::geo::bounds(&p, Cpp::from_mut(&mut lo), Cpp::from_mut(&mut hi)) };
```

## In C++

Each side sees its own language's form: Rust sees `&'a T` / `Option<&'a T>`, and C++ sees `T&` /
`T*`, whichever side calls and whichever implements.

C++ functions that zngur calls directly ([member functions](./cpp_methods.md),
[free functions](./cpp_functions.md), [constructors](./cpp_constructors.md)) get references or
pointers, as declared: references (to C++ objects or to builtin Rust values) and C++-style
references are C++ references, nullable references and pointers are pointers, and references to
other Rust values (`&crate::Foo`, `&str`, ...) are `rust::Ref<T>` / `rust::RefMut<T>`. They're
ordinary C++ calls, with arguments of those types:

```cpp
namespace geo {
double distance(const Point& a, const Point& b);
}
```

Functions in `extern "C++"` blocks, which you write in C++ for Rust, and Rust functions that C++
calls, take and return references to C++ objects as C++ references, and nullable references and raw
pointers as C++ pointers. The generated code spells them `rust::CppRef<W>` /
`rust::CppRef<rust::CppConst<W>>` and `rust::CppPtr<W>` / `rust::CppPtr<rust::CppConst<W>>`, which
are aliases for `T&` / `const T&` and `T*` / `const T*`, where `W` is the Rust wrapper (e.g.
`rust::cpp::geo::Point`) and `T` the C++ type it stands for (`::geo::Point`). The receivers of
`extern "C++"` impls of `c++::` types (`&self`, `&mut self`) are `T&`:

```cpp
// In `extern "C++"`:
// `safe fn midpoint<'a>(&'a const c++::geo::Point, &'a const c++::geo::Point) -> c++::geo::Point;`
::geo::Point rust::exported_functions::midpoint(
    const ::geo::Point& a, const ::geo::Point& b) {
  return ::geo::Point{(a.x + b.x) / 2, (a.y + b.y) / 2};
}

// `safe fn first<'a>(::std::option::Option<&'a const c++::geo::Point>) -> i32;`
int32_t rust::exported_functions::first(const ::geo::Point* p) { return p ? p->x : 0; }
```

References to Rust values (`&crate::Foo`, `&str`, `Option<&crate::Foo>`) are still
`rust::Ref<T>` / `rust::RefMut<T>` (see [name mapping](../call_rust_from_cpp/name_mapping.md)).
