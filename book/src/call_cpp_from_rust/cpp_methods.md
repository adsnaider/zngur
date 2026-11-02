# C++ member functions

zngur can call the member functions of `c++::` types directly, with no
C++ code to write. Declare them in the type's block, with an explicit receiver:

```zng
type c++::geo::Point {
    #trivially_relocatable;
    #layout(size = 8, align = 4);

    safe fn norm<'a>(&'a const self) -> f64;
    safe fn scale<'a>(&'a unique var self, i32);
    safe fn add<'a>(&'a unique var self, &'a const c++::geo::Point);
    safe fn x<'a>(&'a const self) -> &'a const i32;
    safe fn x<'a>(&'a unique var self) -> &'a unique var i32 as x_mut;
    safe fn touch<'a>(&'a var self);
    unsafe fn copy_from<'a>(&'a unique var self, const c++::geo::Point*);
    unsafe fn raw_norm(const self*) -> f64;
    safe fn origin() -> c++::geo::Point;
}
```

- The receiver is a [reference to a C++ object](./cpp_objects.md) (or a raw pointer), which says
  what kind of member function it is:

  | Receiver                      | C++ member function | Rust                                                        |
  | ----------------------------- | ------------------- | ----------------------------------------------------------- |
  | `&'a const self`              | `const`             | method on `Point<C>` (`const` and not), `&'a self`          |
  | `&'a var self`                | non-`const`         | method on `Point` (`Point<NonConst>`), `&'a self`           |
  | `&'a unique var self`         | non-`const`         | method on `Point`, `&'a mut self`                           |
  | `&'a unique const self`       | `const`             | method on `Point<C>`, `&'a mut self`                        |
  | `const self&` / `const self*` | `const`             | associated function on `Point<C>`, taking `*const Point<C>` |
  | `self&` / `self*`             | non-`const`         | associated function on `Point`, taking `*mut Point`         |
  | none                          | static              | associated function on `Point`                              |

  A member function without a receiver is a static member function (`static` isn't written: it's
  an error). `&'a self` and `&'a mut self` are errors, which suggest `&'a const self` and
  `&'a unique var self`.
- `&'a var self` is a non-`const` member function called through a shared reference: the object
  may change while other references to it exist, which is sound because Rust stores C++ objects in
  an `UnsafeCell`. `&'a unique var self` is the usual choice when Rust should have the only access
  (`&mut self`), and `&'a unique const self` a `const` member function that needs it.
- The parameter and return types are the C++ ones, written as zngur types: builtins like `i32`,
  C++ types that Rust holds by value, and [C++ references and pointers](./cpp_pointers.md)
  (`&'a const T` for `const T&`, `&'a unique var T` for a `T&` that Rust uses uniquely, ..., or
  C++-style raw pointers).
- Lifetimes are declared like in Rust (`fn x<'a>(...)`), before any template arguments. Name them:
  zngur can't check them against C++, and warns about elided ones (`#allow(elided_lifetimes)`
  silences it; see [Lifetimes](../lifetimes.md)).
- `as` gives the Rust function a different name, e.g. for C++ overloads, which Rust doesn't have,
  or when the C++ name is a Rust keyword (`safe fn type<'a>(&'a const self) -> i32 as kind;`).
- Member function templates take explicit template arguments, like
  [function templates](./cpp_functions.md#function-templates):
  `safe fn get<'a, i32>(&'a const self) -> i32 as get_i32;`.
- Each member function is `safe` or `unsafe`, like the functions in `extern "C++"` blocks. That's up
  to the `.zng` file, but zngur warns about `safe` ones that take raw pointers
  (`#allow(safe_raw_pointer)` silences it).

## In Rust

Member functions with reference receivers are Rust methods:

```rust
let mut p = Point::origin();
p.scale(2);
let n = p.norm();
*p.x_mut().get_mut() += 1; // `x_mut()` is a `&mut Cpp<i32>`.
let x: i32 = p.x().get(); // `x()` is a `&Cpp<i32, Const>`.
let (a, b) = (&p, &p);
a.touch(); // Non-`const`, through a shared reference.
let c: &Point<Const> = p.as_const();
c.norm(); // `const` member functions work on `const` objects...
// c.scale(2); // ...but the others don't exist on `Point<Const>`.
```

`const` member functions are in an `impl<C: Constness> Point<C>` block, so both `Point` and
`Point<Const>` have them, and the others in an `impl Point` block, so only non-`const` objects have
them, like in C++. Static member functions are in the `impl Point` block too, so `Point::origin()`
works as usual.

Like in C++, parameters that are `const` objects take non-`const` ones too: `add`'s
`&'a const c++::geo::Point` parameter is a `&'a Point<C1>`, generic over the constness, so
`p.add(&q)` works whether `q` is a `Point` or a `Point<Const>` (see
[C++ references and pointers](./cpp_pointers.md)).

## In C++

zngur generates a function that calls the member function, so you don't implement anything in
C++. Arguments and results that Rust holds by value are moved, and references are passed as
references. `this` is a `const` pointer for `const` member functions.

Since that's an ordinary C++ call, overloads and default arguments work as in C++, and calling a
non-`const` member function through a `const` reference doesn't compile. The return type must match
the declaration (a `static_assert` checks it), except that builtins only need the same size and
kind, e.g. `long long` for an `i64`. Arguments are converted as in any C++ call, so declare them
exactly.
