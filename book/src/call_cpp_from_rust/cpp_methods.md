# C++ member functions

zngur can call the member functions of `c++::` types directly, with no
C++ code to write. Declare them in the type's block, the C++ way:

```zng
type c++::geo::Point {
    #trivially_relocatable;
    #layout(size = 8, align = 4);

    safe fn norm() const -> f64;
    safe fn scale(i32);
    safe fn add(const c++::geo::Point&);
    safe fn x() const -> const i32&;
    safe fn x() -> i32& as x_mut;
    unsafe fn copy_from(const c++::geo::Point*);
    safe static fn origin() -> c++::geo::Point;
}
```

- `static` declares a static member function, and `const` after the parameters a `const` one.
- The parameter and return types are the C++ ones, written as zngur types: builtins like `i32`,
  C++ types that Rust holds by value, and [C++ references and pointers](./cpp_pointers.md).
- `as` gives the Rust function a different name, e.g. for C++ overloads, which Rust doesn't have.
- Member function templates take explicit template arguments, like
  [function templates](./cpp_functions.md#function-templates): `safe fn get<i32>() const -> i32 as get_i32;`.
- Each member function is `safe` or `unsafe`, like the functions in `extern "C++"` blocks. `this`
  is a [`CppRef`](./cpp_pointers.md), which refers to a valid object, so member functions can be
  `safe`. Those with [C++ pointer](./cpp_pointers.md) parameters, which may be null or invalid,
  must be `unsafe`.

## In Rust

Each member function becomes an associated function of the Rust type, which takes `this` as its
first argument, as a `CppRef<Self, Const>` for `const` member functions and a `CppRef<Self, Mut>`
otherwise:

```rust
let mut p = Point::origin();
Point::scale(p.as_mut(), 2);
let n = Point::norm(p.as_ref());
let x: CppRef<i32, Mut> = Point::x_mut(p.as_mut());
x.write(x.read() + 1);
```

They're not Rust methods (`p.norm()`), because Rust methods can't take a `CppRef` as `self` on
stable Rust yet.

## In C++

zngur generates a function that calls the member function, so you don't implement anything in
C++. Arguments and results that Rust holds by value are moved, and references are passed as
references.

Since that's an ordinary C++ call, overloads and default arguments work as in C++, and calling a
non-`const` member function through a `const` reference doesn't compile. The return type must match
the declaration (a `static_assert` checks it), except that builtins only need the same size and
kind, e.g. `long long` for an `i64`. Arguments are converted as in any C++ call, so declare them
exactly.
