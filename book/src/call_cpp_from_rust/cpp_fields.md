# C++ fields

zngur can give Rust access to the fields (data members) of `c++::` types,
with no C++ code to write. Declare an accessor in the type's block, with the field's type:

```zng
type c++::geo::Point {
    #trivially_relocatable;
    #layout(size = 8, align = 4);

    field const x (type = i32);
    field x (type = i32) as x_mut;
}
```

- A `const` accessor gives a `const` reference to the field, and the others a non-`const` one.
- `as` gives the Rust function a different name, e.g. to have both kinds of accessor for a field.
  It's also needed for C++ names that are Rust keywords: `field const type (type = i32) as type_;`.
- There's no offset to declare, since C++ knows it. That's also what distinguishes these from the
  fields of Rust types, which have an `offset`.

In Rust, each accessor is a safe method that returns a reference to the field:
`fn x(&self) -> &i32` for `const` accessors, which `const` objects (`Point<Const>`) have too, and
`fn x_mut(&mut self) -> &mut i32` for the others (see [`const`](./cpp_pointers.md)). A field of
class type is a `&T<Const>` from a `const` accessor.

```rust
let mut p = Point::build((1, 2));
*p.x_mut() += 10;
let x: i32 = *p.x();
```

zngur generates the C++ function that takes the field's address, `&this->x`, so C++ checks that the
field is accessible: accessors for private fields don't compile. A `static_assert` checks that the
field has the declared type (ignoring a top-level `const`; `const` fields only have `const`
accessors). Fields can be C++ pointers (`field next (type = c++::Node*)`). Bitfields and fields of
reference type aren't supported.
