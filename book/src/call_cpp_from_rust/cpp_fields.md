# C++ fields

zngur can give Rust access to the fields (data members) of `#cpp_stack_owned` and `#cpp_ref` types,
with no C++ code to write. Declare an accessor in the type's block, with the field's type:

```zng
type c++::geo::Point {
    #cpp_stack_owned;
    #layout(size = 8, align = 4);

    field const x (type = i32);
    field x (type = i32) as x_mut;
}
```

- A `const` accessor gives a `const` pointer to the field, and the others a non-`const` one.
- `as` gives the Rust function a different name, e.g. to have both kinds of accessor for a field.
- There's no offset to declare, since C++ knows it. That's also what distinguishes these from the
  fields of Rust types, which have an `offset`.

In Rust, each accessor is an `unsafe` associated function, which takes `this` as a
[`CppPtr`](./cpp_pointers.md) and returns a `CppPtr` to the field: a `CppPtr<Self, Const>` and a
`CppPtr<T, Const>` for `const` accessors, and `Mut` ones otherwise.

```rust
let mut p = Point::build((1, 2));
unsafe {
    *Point::x_mut(p.as_mut()).as_mut().unwrap() += 10;
    let x: i32 = *Point::x(p.as_ref()).as_ref().unwrap();
}
```

zngur generates the C++ function that takes the field's address, `&this->x`, so C++ checks that the
field is accessible: accessors for private fields don't compile. A `static_assert` checks that the
field has the declared type. Bitfields and fields of reference type aren't supported.
