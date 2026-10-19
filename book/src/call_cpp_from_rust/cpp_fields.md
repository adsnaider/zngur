# C++ fields

zngur can give Rust access to the fields (data members) of `c++::` types,
with no C++ code to write. Declare an accessor in the type's block, with the field's type:

```zng
type c++::geo::Point {
    #trivially_relocatable;
    #layout(size = 8, align = 4);

    field &const x (type = i32);
    field &unique var x (type = i32) as x_mut;
    field var y (type = i32);
}
```

The accessor says what it returns, using the same words as
[references to C++ objects](./cpp_objects.md):
`field [& [unique]] (const | var) name (type = T) [as rust_name];`.

| `.zng`                      | Rust (builtin `i32` field)                | Rust (class `T` field) | Available on |
| --------------------------- | ----------------------------------------- | ---------------------- | ------------ |
| `field &const x ...`        | `fn x(&self) -> &Cpp<i32, Const>`         | `-> &T<Const>`         | `Point<C>`   |
| `field &var x ...`          | `fn x(&self) -> &Cpp<i32>`                | `-> &T`                | `Point`      |
| `field &unique var x ...`   | `fn x(&mut self) -> &mut Cpp<i32>`        | `-> &mut T`            | `Point`      |
| `field &unique const x ...` | `fn x(&mut self) -> &mut Cpp<i32, Const>` | `-> &mut T<Const>`     | `Point<C>`   |
| `field const x ...`         | `fn x(&self) -> Cpp<i32, Const>` (a copy) | `-> T<Const>` (a copy) | `Point<C>`   |
| `field var x ...`           | `fn x(&self) -> i32` (a copy)             | `-> T` (a copy)        | `Point<C>`   |

- With `&`, the accessor returns a reference to the field: shared by default, or `unique`.
  `unique` only applies to references, so `field unique var x` is an error.
- Without `&`, it returns a **copy** of the field. Copying only reads the field, so both kinds of
  copy are available on `const` objects too. A copy of a class needs a type that Rust holds by
  value (with a layout and `#trivially_relocatable`), and C++ copy constructs it.
- `const` or `var` is always required (a missing one is an error that lists the choices).
- `(type = T)` is the field's own type, which a `static_assert` checks (ignoring a top-level
  `const`). `const` fields only have `&const`, `&unique const`, `const` and `var` accessors: the
  others don't compile.
- `as` gives the Rust function a different name, e.g. to have several accessors for a field.
  It's also needed for C++ names that are Rust keywords: `field &const type (type = i32) as type_;`.
- There's no offset to declare, since C++ knows it. That's also what distinguishes these from the
  fields of Rust types, which have an `offset`.

Each accessor is a safe method. Builtins behind references are
[`zngur_lib::Cpp`](./cpp_objects.md) objects, whose value
`get()` reads:

```rust
let mut p = Point::build((1, 2));
*p.x_mut().get_mut() += 10;
let x: i32 = p.x().get();
let y: i32 = p.y();
```

Fields can be C++ pointers, which are builtins too: `field &var next (type = c++::Node*)` returns a
`&Cpp<*mut Node>`, which can be `set` even through a shared reference, and `field var next ...` a
`*mut Node`.

zngur generates the C++ function that takes the field's address, `&this->x` (or copies it), so C++
checks that the field is accessible: accessors for private fields don't compile. Bitfields and
fields of reference type aren't supported.
