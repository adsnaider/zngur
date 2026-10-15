# C++ constructors

zngur can call the constructors of `c++::` types that Rust holds by value (with a layout and
`#trivially_relocatable`) directly, with no C++ code to write.
Declare them in the type's block, with their parameter types:

```zng
type c++::geo::Point {
    #trivially_relocatable;
    #layout(size = 8, align = 4);

    safe constructor();
    safe constructor(i32, i32);
    unsafe constructor(const c++::geo::Point&);
}
```

Each constructor is `safe` or `unsafe`, like the functions in `extern "C++"` blocks. Constructors
with [C++ pointer or reference](./cpp_pointers.md) parameters, like copy constructors, must be
`unsafe`.

## In Rust

Constructors initialize objects in place, which Rust functions can't express. So instead of Rust
functions, each constructor implements the `zngur_lib::ZngCppConstruct<Args>` trait, where `Args`
is the tuple of its parameter types: `ZngCppConstruct<()>`, `ZngCppConstruct<(i32, i32)>`, and so
on. Overloads are just different `Args`, so they don't need names.

The trait has four methods:

| Method                     | Does                                                    | Available for       |
| -------------------------- | ------------------------------------------------------- | ------------------- |
| `construct(slot, args)`    | Runs the constructor on `slot: &mut MaybeUninit<Self>`. | `safe` constructors |
| `build(args) -> Self`      | Constructs an object and returns it.                    | `safe` constructors |
| `construct_unchecked(...)` | Like `construct`, but `unsafe`.                         | all constructors    |
| `build_unchecked(...)`     | Like `build`, but `unsafe`.                             | all constructors    |

```rust
use zngur_lib::ZngCppConstruct;

let origin = Point::build(());
let p = Point::build((3, 4));
let copy = unsafe { Point::build_unchecked((p.as_ref(),)) };

let mut slot = MaybeUninit::uninit();
Point::construct(&mut slot, (5, 6));
let q = unsafe { slot.assume_init() };
```

`build` moves the object out of the slot it was constructed in, which is fine because objects on
the Rust stack are trivially relocatable.

Whether a constructor is safe is the trait's `Safety` associated type: `zngur_lib::IsSafe` or
`zngur_lib::IsUnsafe`. `construct` and `build` require `Self::Safety: zngur_lib::Safe`, which only
`IsSafe` implements, so calling them for an `unsafe` constructor doesn't compile.

## In C++

zngur generates a function that runs the constructor with placement `new`. Since that's an ordinary
C++ constructor call, overloads and default arguments work as in C++, and arguments are converted
as in any C++ call, so declare them exactly.
