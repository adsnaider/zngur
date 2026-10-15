# C++ member functions

zngur can call the member functions of `#cpp_stack_owned` and `#cpp_ref` types directly, with no
C++ code to write. Declare them in the type's block, the C++ way:

```zng
type c++::geo::Point {
    #cpp_stack_owned;
    #layout(size = 8, align = 4);

    unsafe fn norm() const -> f64;
    unsafe fn scale(i32);
    unsafe fn add(const c++::geo::Point&);
    unsafe fn x() const -> const i32&;
    unsafe fn x() -> i32& as x_mut;
    safe static fn origin() -> c++::geo::Point;
}
```

- `static` declares a static member function, and `const` after the parameters a `const` one.
- The parameter and return types are the C++ ones, written as zngur types: builtins like `i32`,
  `#cpp_stack_owned` types by value, and [C++ pointers and references](./cpp_pointers.md).
- `as` gives the Rust function a different name, e.g. for C++ overloads, which Rust doesn't have.
- Each member function is `safe` or `unsafe`, like the functions in `extern "C++"` blocks. Since
  `this` is a C++ pointer, only static member functions without C++ pointer or reference
  parameters can be `safe`.

## In Rust

Each member function becomes an associated function of the Rust type, which takes `this` as its
first argument, as a `CppPtr<Self, Const>` for `const` member functions and a `CppPtr<Self, Mut>`
otherwise:

```rust
let mut p = Point::origin();
unsafe {
    Point::scale(p.as_mut(), 2);
    let n = Point::norm(p.as_ref());
    let x: CppPtr<i32, Mut> = Point::x_mut(p.as_mut());
}
```

They're not Rust methods (`p.norm()`), because Rust methods can't take a `CppPtr` as `self`.

## In C++

zngur generates a function that calls the member function, so you don't implement anything in
C++. Arguments and results of `#cpp_stack_owned` types are moved, and references are passed as
references.

Since that's an ordinary C++ call, overloads and default arguments work as in C++, and calling a
non-`const` member function through a `const` pointer doesn't compile. The return type must match
the declaration (a `static_assert` checks it), except that builtins only need the same size and
kind, e.g. `long long` for an `i64`. Arguments are converted as in any C++ call, so declare them
exactly.
