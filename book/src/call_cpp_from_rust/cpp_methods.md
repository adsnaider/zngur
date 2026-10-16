# C++ member functions

zngur can call the member functions of `c++::` types directly, with no
C++ code to write. Declare them in the type's block, with an explicit receiver:

```zng
type c++::geo::Point {
    #trivially_relocatable;
    #layout(size = 8, align = 4);

    safe fn norm(&self) -> f64;
    safe fn scale(&mut self, i32);
    safe fn add(&mut self, &c++::geo::Point);
    safe fn x<'a>(&'a self) -> &'a i32;
    safe fn x<'a>(&'a mut self) -> &'a mut i32 as x_mut;
    unsafe fn copy_from(&mut self, const c++::geo::Point*);
    unsafe fn raw_norm(const self*) -> f64;
    safe static fn origin() -> c++::geo::Point;
}
```

- The receiver says what kind of member function it is:

  | Receiver                      | C++ member function | Rust                                                           |
  | ----------------------------- | ------------------- | -------------------------------------------------------------- |
  | `&'a self`                    | `const`             | method on `Point<C>` (`const` and not), `&'a self`             |
  | `&'a mut self`                | non-`const`         | method on `Point` (`Point<NonConst>`), `&'a mut self`          |
  | `const self&` / `const self*` | `const`             | associated function on `Point<C>`, `this: *const Point<C>`     |
  | `self&` / `self*`             | non-`const`         | associated function on `Point`, `this: *mut Point`             |
  | none, with `static`           | static              | associated function on `Point`                                 |

  A member function that isn't `static` must have a receiver.
- The parameter and return types are the C++ ones, written as zngur types: builtins like `i32`,
  C++ types that Rust holds by value, and [C++ references and pointers](./cpp_pointers.md)
  (`&'a T` for `const T&`, `&'a mut T` for `T&`, or C++-style raw pointers).
- Lifetimes are declared like in Rust (`fn x<'a>(...)`), before any template arguments. Name them:
  zngur can't check them against C++, and warns about elided ones
  ([Lifetimes](../lifetimes.md)).
- `as` gives the Rust function a different name, e.g. for C++ overloads, which Rust doesn't have.
- Member function templates take explicit template arguments, like
  [function templates](./cpp_functions.md#function-templates):
  `safe fn get<i32>(&self) -> i32 as get_i32;`.
- Each member function is `safe` or `unsafe`, like the functions in `extern "C++"` blocks. That's up
  to the `.zng` file, but zngur warns about `safe` ones that take raw pointers.

## In Rust

Member functions with `&self` / `&mut self` receivers are Rust methods:

```rust
let mut p = Point::origin();
p.scale(2);
let n = p.norm();
*p.x_mut() += 1;
let c: &Point<Const> = p.as_const();
c.norm(); // `const` member functions work on `const` objects...
// c.scale(2); // ...but the others don't exist on `Point<Const>`.
```

`const` member functions are in an `impl<C: Constness> Point<C>` block, so both `Point` and
`Point<Const>` have them, and the others in an `impl Point` block, so only non-`const` objects have
them, like in C++. Static member functions are in the `impl Point` block too, so `Point::origin()`
works as usual.

## In C++

zngur generates a function that calls the member function, so you don't implement anything in
C++. Arguments and results that Rust holds by value are moved, and references are passed as
references.

Since that's an ordinary C++ call, overloads and default arguments work as in C++, and calling a
non-`const` member function through a `const` reference doesn't compile. The return type must match
the declaration (a `static_assert` checks it), except that builtins only need the same size and
kind, e.g. `long long` for an `i64`. Arguments are converted as in any C++ call, so declare them
exactly.
