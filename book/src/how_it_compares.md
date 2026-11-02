# How Zngur compares to other tools

Zngur is not the only tool in the Rust/C++ interoperability space.
Its pros and cons relative to other popular tools are described on this page.

## CXX

CXX says:

> Be aware that the design of this library is intentionally restrictive and opinionated!
> It isn't a goal to be powerful enough to handle arbitrary signatures in either language.
> Instead, this project is about carving out a reasonably expressive set of functionality
> about which we can make useful safety guarantees today and maybe extend over time.

Zngur also makes safety guarantees but also tries to be powerful enough to handle arbitrary signatures from Rust code,
including generics, `impl` and `dyn` traits, and owned objects. So with Zngur you can use almost any Rust library in C++,
but with CXX you need to write some manual glue code (potentially with runtime costs) to avoid inaccessible types and signatures.

For using C++ in Rust, Zngur lets you declare `c++::` types directly — holding
[trivially relocatable](./call_cpp_from_rust/opaque.md#trivially-relocatable-c-types) ones by value
on the Rust stack, referring to non-relocatable ones through references or C++ smart pointers like
`c++::std::unique_ptr<T>` (without `Pin` ceremonies), and calling their member functions,
constructors, field accessors, and free functions directly. When a C++ library's raw API doesn't fit
Rust's ownership or lifetime model, you can wrap those direct calls in Rust, or shape a Rusty API on
the C++ side: define the behavior in a Rust trait and convert C++ classes into `Box<dyn Trait>`, or
write `impl` blocks and free functions in C++ for Rust or `c++::` types.

Zngur's benefits over CXX are:

- Zngur supports owning Rust variables on the C++ stack, and trivially relocatable C++ objects on
  the Rust stack, saving unnecessary allocations.
- CXX has limited support for some types in the standard library,
  but Zngur supports almost everything (`Vec<T>`, `Vec<Vec<T>>`, `HashMap<T, T>`, `Box<[T]>`, `Arc<dyn T>`, ...)
  with almost full API.
- Zngur keeps the Rust side clean and normal: direct `c++::` calls avoid `Pin` ceremonies, and any
  richer adaptation glue can live either in Rust or on the C++ side.
  Using CXX, foreign wrapper ceremonies and glue code tend to leak into both languages.

### Migrating from CXX to Zngur

- Create a `main.zng` file.
- Add everything in the `extern "Rust"` block into the `main.zng`.
  Zngur should support everything supported by CXX.
- Declare types from CXX's `extern "C++"` block as `type c++::...` in `main.zng`, along with the
  member functions, constructors, and fields you call from Rust, and declare free C++ functions as
  `c++::...` or in `mod c++::...` blocks. Add `#layout(size = ..., align = ...)` and
  `#trivially_relocatable` to C++ types that can be moved with `memcpy` so Rust can hold them by
  value; leave non-relocatable C++ types without them to refer to them through references or smart
  pointers. (When you want to shape a higher-level Rust API on the C++ side, you can also write
  `extern "C++"` `impl` blocks or functions, or convert C++ classes into `Box<dyn Trait>`.)
- For builtin C++ types that CXX supports:
  - `UniquePtr<T>`:
    - Declare `type<T> c++::std::unique_ptr<T>` with `#layout(size = 8, align = 8);` and
      `#trivially_relocatable;`, and instantiate `type c++::std::unique_ptr<c++::Foo> {}` (see
      [C++ class templates](./call_cpp_from_rust/cpp_templates.md)).
  - `CxxString`:
    - Declare `type c++::std::string {}` to pass or inspect C++ strings by reference (or hold them via
      `c++::std::unique_ptr<c++::std::string>`, since `std::string` may not be trivially relocatable
      under small-string optimization).
    - Convert it to a Rust `&CStr`, `&[u8]`, or `&str` when borrowing as a Rust string slice, or
      copy it into a `CString`, `Vec<u8>`, or `String` when an owned Rust string is desired.
  - `CxxVector<T>`:
    - Declare `type<T> c++::std::vector<T>` and instantiate `type c++::std::vector<i32> {}` or
      `type c++::std::vector<c++::Foo> {}` to pass or mutate C++ vectors by reference without `Pin` (or
      own them via `c++::std::unique_ptr<c++::std::vector<...>>`, since `std::vector` is not
      specialized as trivially relocatable out of the box and is not trivially relocatable in debug
      iterator builds such as MSVC debug builds; holding `std::vector` by value on the Rust stack
      requires `#layout` or `#layout_conservative` plus `#unsafe(trivially_relocatable)` and is only
      sound on standard-library configurations where `std::vector` can be moved with `memcpy`).
    - Or borrow its elements as `&[T]` (for layout-compatible elements) or copy them into a Rust
      `Vec` when an idiomatic Rust container is desired.

## AutoCXX

AutoCXX is different.
It also tries to be powerful enough to handle arbitrary signatures but from C++ code.
It does a nice job, but due the fundamental difference between Rust and C++, it has several drawbacks:

- It wraps every function result that is not behind a reference in a `UniquePtr`,
  which has both performance and ergonomic costs.
- Every potentially mutable reference (`&mut`, `Box`, ...) should be wrapped in a `Pin`, similar to CXX.
- It has poor support for templates,
  though it might be temporary and an ideal AutoCXX might fix some of its problems with template.

So, an auto-generated binding for a C++ library using AutoCXX is not an idiomatic and ergonomic Rust.
So unless you are writing a very little amount of Rust code,
or can tolerate `UniquePtr` and `Pin` to be propagated in your Rust code,
you will need to write a Rustic wrapper for your C++ classes.
This is even recommended in the AutoCXX docs:

> The C++ API may have documented usage invariants. Your ideal is to encode as many as possible of those into compile-time checks in Rust.
> Some options to consider:
> Wrap the bindings in a new type wrapper which enforces compile-time variants in its APIs;
> for example, taking a mutable reference to enforce exclusive access.

Using Zngur, you can write that wrapper either in Rust — on top of direct `c++::` declarations,
without `Pin` or forced `UniquePtr` returns on trivially relocatable types — or in C++!
Zngur supports implementing Rust traits for C++ types and casting them to `Box<dyn Trait>`,
implementing inherent and trait methods on Rust or `c++::` types in C++,
and calling free functions written in C++ that operate on Rust or `c++::` types.
Using those, you can create idiomatic Rust wrappers for your C++ library on whichever side is most
natural.

Idiomatic C++ and idiomatic Rust are like night and day, so glue code between them often helps.
Whereas AutoCXX forces all of that adaptation to live in Rust (working around `Pin` and `UniquePtr`),
Zngur lets you put glue code in Rust or in C++.
Writing glue code in C++ when needed lets you construct C++ objects on the stack without macros and
call their move and copy constructors naturally, without hitting major problems when using Rust
types.

### Safety of AutoCXX

Idiomatic Rust libraries encapsulate unsafe, and expose safe functions for user code.
Safety is another reason that a Rust wrapper should exist between Rust most of their API surface.
Safe functions in Rust are designed to be memory safe even in presence of adversary safe code.
(Adversary safe code can exploit soundness bugs in the compiler, but that's the north star Rust follows,
and it is not that far from it) Almost no C++ API is designed with that level of safety in mind.
By writing a Rusty wrapper for C++ APIs, we can think about safety and design a safe (in Rust terms) API.

AutoCXX has a pretty hand-wavy approach to safety. It gives you three options:

- Make all functions safe, which is almost always wrong.
- Make all functions unsafe, which leads to spamming unsafe keyword everywhere, making unsafe lose
  its meaning.
- Make all functions which uses C++ references as unsafe, and all other functions safe. The fact that
  this option exists shows that declaring all functions as safe isn't working very well.

From the AutoCXX docs:

> Irrespective, C++ code is of course unsafe.
> It’s worth noting that use of C++ can cause unexpected unsafety at a distance in faraway Rust code.
> As with any use of the unsafe keyword in Rust,
> you the human are declaring that you’ve analyzed all possible ways that the code can be used
> and you are guaranteeing to the compiler that no badness can occur. Good luck.

### Using Zngur and AutoCXX together

All being said, if you want to have fully auto-generated bindings for both sides,
nothing will prevent you from using Zngur and AutoCXX at the same time.

## Crubit

Crubit is similar to AutoCXX, but it heavily integrates with rust and C++ compilers so it can handle owned
types of other sides without manual annotations.
The problems of AutoCXX about idiomatic and safe APIs applies to Crubit as well.

Crubit requires heavy control over the Rust and C++ build system,
but Zngur can work with almost any C++ compiler and any build system, as it only generates some header files.

An auto generator for zng files is planned, which can bridge the gap between Zngur and Crubit in ease of use,
but currently it does not exist.

At the time of writing this, Crubit is limited to Google build system and is not ready for general use.
It also lacks some expressive power relative to Zngur, for example it can't handle generics and `dyn` traits,
but it has plans to change this.
