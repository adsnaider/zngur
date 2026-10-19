# C++ objects vs Rust values

A reference in a `.zng` file says what it refers to:

- A **C++ object** follows C++'s rules: other code may change it while Rust looks at it, unless
  it's `const`, and nothing promises that a `T&` is the only reference to it. References to C++
  objects say whether the object is `const`: `&'a const T` or `&'a var T`.
- A **Rust value** follows Rust's rules: a `&'a T` is never changed while it lives, and a
  `&'a mut T` is the only way to reach the value. It's written like a Rust reference, `&'a T` /
  `&'a mut T`, and C++ code that gets one must keep these rules.

C++ classes (`c++::` types) are always C++ objects, so references to them always say `const` or
`var`. Rust types (`crate::Foo`, `str`, slices, `Box`, ...) are always Rust values, so they never
do. Builtins (`bool`, the integers, `f32`, `f64`) and C++ pointers can be either.

## References to C++ objects

```text
&['a] [unique] (const | var) T
```

| `.zng`                    | C++                                      | Rust (class `T`)   | Rust (builtin `i32`)                     |
| ------------------------- | ---------------------------------------- | ------------------ | ---------------------------------------- |
| `&'a const T`             | `const T&`                               | `&'a T<Const>`     | `&'a Cpp<i32, Const>`                    |
| `&'a var T`               | `T&`, shared                             | `&'a T`            | `&'a Cpp<i32>`                           |
| `&'a unique var T`        | `T&`, and Rust has the only access       | `&'a mut T`        | `&'a mut Cpp<i32>`                       |
| `&'a unique const T`      | `const T&`, and Rust has the only access | `&'a mut T<Const>` | `&'a mut Cpp<i32, Const>`                |
| `&'a i32` / `&'a mut i32` | `const int32_t&` / `int32_t&`            | —                  | `&'a i32` / `&'a mut i32` (a Rust value) |

- **Sharing is the default.** `&'a const T` and `&'a var T` are shared references: Rust may have
  others, and C++ may have others too. A `var` object can change through any of them, including
  by calling a non-`const` member function through a shared reference (`&'a var self`). That's
  sound because the Rust wrappers of C++ objects store them in an `UnsafeCell`.
- **`unique` is a promise from the C++ side**, which whoever writes the `.zng` file makes: for the
  reference's lifetime, nothing else accesses the object. That's what lets it be a Rust `&mut`.
- `unique const` is a unique reference to a `const` object (`&mut T<Const>`). It's rarely needed,
  but someone has to choose it explicitly, so it can't happen by accident.
- `const` and `var` are part of the reference. A `const` object only has `const` member functions
  (see [`const` is part of the type](./cpp_pointers.md#const-is-part-of-the-type)), and a `var`
  one has all of them.
- `unique` and `var` are only keywords right after `&` / `&'a`, so they can still be names
  elsewhere.

Mistakes are errors with a suggestion: `&'a c++::T` (say `const` or `var`), `&'a mut c++::T` (say
`&'a unique var c++::T`), `&'a unique T` (say `const` or `var`), `&'a const crate::Foo` (a Rust
type), and `&'a const char` (`char` can't be a C++ object, since C++ can store invalid code points
in it).

## Builtins as C++ objects: `zngur_lib::Cpp<T>`

A builtin that is a C++ object is a `zngur_lib::Cpp<T, C>` in Rust: `Cpp<i32>` is a C++ `int32_t`,
and `Cpp<i32, Const>` a `const int32_t`. `T` is `bool`, an integer, `f32`, `f64` or a raw pointer
(the sealed trait `CppRustBuiltin`), and `C` is the constness (`NonConst` by default).

`Cpp<T, C>` has the same layout as `T`, and it's an `UnsafeCell<T>` inside, like
[`Cell`](https://doc.rust-lang.org/std/cell/struct.Cell.html): C++ may change the value behind a
shared reference, so Rust copies it out instead of handing out a `&T`.

| Method                                   | Available on     | What it does                                 |
| ---------------------------------------- | ---------------- | -------------------------------------------- |
| `Cpp::new(v)`, `v.into()`                | `Cpp<T>`         | makes one (e.g. to construct a copy)         |
| `Cpp::new_const(v)`                      | `Cpp<T, Const>`  | makes a `const` one                          |
| `get(&self) -> T`                        | both constnesses | reads the value                              |
| `into_inner(self) -> T`, `T::from(c)`    | both constnesses | the value, by value                          |
| `as_ptr(&self) -> *const T`              | both constnesses | a pointer to the value                       |
| `set(&self, v)`                          | `Cpp<T>`         | writes the value, through a shared reference |
| `get_mut(&mut self) -> &mut T`           | `Cpp<T>`         | a Rust reference, through a unique one       |
| `as_const`, `as_const_mut`, `into_const` | `Cpp<T>`         | one-way, like the C++ classes' ones          |
| `Cpp::from_mut(&mut T) -> &mut Cpp<T>`   | `Cpp<T>`         | a Rust value as a C++ object                 |

It's also `Debug`, `PartialEq` (with `Cpp<T, _>` and `T`) and `PartialOrd` (with `Cpp<T, _>`), all
through `get`.

```rust
let count: &Cpp<i32> = counter.count_var(); // `field &var count (type = i32) as count_var;`
count.set(count.get() + 1);

let mut sum = 0;
counter.add_to(Cpp::from_mut(&mut sum)); // `safe fn add_to<'a>(&'a const self, &'a unique var i32);`
```

There's no `Cpp::from_ref(&T) -> &Cpp<T, _>`: C++ could change a `Cpp<T>` behind it (`set` is
safe), and while `Cpp<T, Const>` can't be changed from Rust, zngur can't make sure that C++
doesn't change it either, which a `&T` promises.

### Pointers

C++ pointers are builtins too. A field of type `c++::Node*` is a `Cpp<*mut Node>` behind a
reference (`field &var next (type = c++::Node*)` returns a `&Cpp<*mut Node>`, which can be `set`),
and a `*mut Node` when it's copied. Pointers to pointers are pointers to `Cpp`: `c++::Node**` is a
`*mut Cpp<*mut Node>`, just like `i32*` is a `*mut Cpp<i32>`.

## Raw pointers

C++-style references and pointers (`const T&`, `T*`, ...) point to C++ objects as well, so their
Rust types are the same: `const T*` is `*const T<Const>` (`*const Cpp<i32, Const>` for a builtin),
and `T*` is `*mut T` (`*mut Cpp<i32>`). As parameters, `const` ones are generic over the constness
(`*const T<C1>`), like references. See [C++ references and pointers](./cpp_pointers.md).

## Fields

[Field accessors](./cpp_fields.md) use the same words: `field &const x`, `field &var x`,
`field &unique var x` and `field &unique const x` return references, and `field const x` /
`field var x` return copies (`Cpp<i32, Const>` / `i32`, or `T<Const>` / `T` for a class).
