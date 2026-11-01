# C++ constructors

zngur can call the constructors of `c++::` types that Rust holds by value (with a layout and
`#trivially_relocatable`) directly, with no C++ code to write.
Declare them in the type's block, with their parameter types and a Rust name:

```zng
type c++::geo::Point {
    #trivially_relocatable;
    #layout(size = 8, align = 4);

    safe constructor() as new;
    safe constructor(i32, i32) as from_xy;
    safe constructor<'a>(&'a const c++::geo::Point) as copy;
    unsafe constructor(const c++::geo::Point*, i32) as shifted;
}
```

- The parameter types are written like those of [member functions](./cpp_methods.md):
  `&'a const T` for a `const T&`, `&'a unique var T` for a `T&` that Rust uses uniquely, and
  C++-style raw pointers.
- The name after `as` is required: Rust has no overloads, and no name is a better default than
  another. It can't be the name of another constructor or member function of the type.
- Each constructor is `safe` or `unsafe`, like the functions in `extern "C++"` blocks. zngur warns
  about `safe` constructors that take [raw pointers](./cpp_pointers.md), which may be null or
  invalid. References are fine, so copy constructors can be `safe`.

## In Rust

Constructors initialize objects in place, which Rust functions can't express. So each constructor
is an associated function that returns the constructor call, which hasn't run yet: an
`impl zngur_lib::CppCtor<Output = Self>`, which holds the arguments. The `CppCtor` trait runs it:

| Method                                      | Does                                                            |
| ------------------------------------------- | --------------------------------------------------------------- |
| `build(self) -> T`                          | Constructs an object and returns it.                            |
| `init(self, &mut MaybeUninit<T>) -> &mut T` | Constructs an object in the slot and returns a reference to it. |

```rust
use std::mem::MaybeUninit;
use zngur_lib::CppCtor;

let origin = Point::new().build();
let p = Point::from_xy(3, 4).build();
let copy = Point::copy(&p).build();
let shifted = unsafe { Point::shifted(&raw const p, 10) }.build();

let mut slot = MaybeUninit::uninit();
let q: &mut Point = Point::from_xy(5, 6).init(&mut slot);
```

- `build` moves the object out of the slot it was constructed in, which is fine because objects on
  the Rust stack are trivially relocatable.
- `init` leaves the object in the slot. `MaybeUninit` doesn't drop what it contains, so the
  object's destructor only runs if you drop it yourself (`unsafe { slot.assume_init_drop() }`);
  otherwise it leaks, which is safe.
- An `unsafe` constructor is an `unsafe fn`: calling it is where you promise its preconditions, and
  running the returned constructor call is safe.
- Like those of other C++ functions, parameters that are `const` objects take non-`const` ones too
  (see [C++ references and pointers](./cpp_pointers.md)): `copy` is
  `fn copy<'a, C1: Constness>(&'a Point<C1>) -> impl CppCtor<Output = Self> + use<'a, C1>`, so
  `Point::copy(&p)` works for a `Point` and a `Point<Const>` alike. The constructor call borrows
  its arguments, so it can't outlive them.
- Constructors make non-`const` objects; `into_const()` makes one `const`.

## In C++

zngur generates a function that runs the constructor with placement `new`. Since that's an ordinary
C++ constructor call, default arguments work as in C++, and arguments are converted as in any C++
call, so declare them exactly: the declared parameter types pick the C++ overload.
