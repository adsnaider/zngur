# Opaque C++ types

C++ types are declared with a `c++::` path, which is also their C++ name: `type c++::osmium::Way`
is the C++ type `::osmium::Way`. Multiple declarations sharing a prefix can also be grouped with
`mod c++::osmium { type Way { ... } }`. What Rust can do with such a type follows from what the
`.zng` file says about it:

| Declaration                                                                    | Rust can                                                                               |
| ------------------------------------------------------------------------------ | -------------------------------------------------------------------------------------- |
| no layout                                                                      | only refer to it (`&const T`, `&var T`, `&unique var T`, raw pointers, smart pointers) |
| a layout, without `#trivially_relocatable` or `#unsafe(trivially_relocatable)` | only refer to it (the layout is checked in C++)                                        |
| a layout and `#trivially_relocatable` (or `#unsafe(trivially_relocatable)`)    | hold it by value, like a Rust value                                                    |

To own a C++ object that isn't trivially relocatable, hold it through a C++ smart pointer, like
`std::unique_ptr` (see [C++ class templates](./cpp_templates.md)).

> **NOTE**: In C++, the Rust wrapper of a `c++::a::B` type is `rust::cpp::a::B`, apart from the
> wrappers of Rust types (`rust::crate::...`, `rust::std::...`, ...). In Rust, its wrapper is at
> `generated::a::B`.

If a C++ type's name can't be written as a `c++::` path, e.g. because it differs from what you want
to call it in Rust or it's a nested type, give it one with a C++ alias:

```cpp
namespace task { using CppTask = CppTaskForRust; }
```

```zng
type c++::task::CppTask { ... }
```

## Reference-only C++ types

A C++ type without a layout is reference-only:

```zng
type c++::osmium::Way {
}
```

This enables passing references to `osmium::Way` across the boundary as C++ object references
(`&'a const c++::osmium::Way`, `&'a var c++::osmium::Way`, `&'a unique var c++::osmium::Way`; see
[C++ objects vs Rust values](./cpp_objects.md)), which C++ sees directly as `const ::osmium::Way&`
or `::osmium::Way&` and Rust sees as `&'a Way<Const>`, `&'a Way`, or `&'a mut Way`. Rust can't do
anything meaningful with it on its own except passing it back to C++ or calling its
[member functions](./cpp_methods.md). You can also write `extern "C++"` `impl` and `impl Trait for`
blocks for `c++::osmium::Way` in C++. See
[`examples/osmium`](https://github.com/HKalbasi/zngur/blob/main/examples/osmium) for a full working
example.

### Layouts of types that aren't trivially relocatable

You can give a reference-only type a layout anyway (`#layout(...)` or `#layout_conservative(...)`).
Rust still only refers to it, but the generated C++ code checks the size and alignment. zngur warns
about such layouts (`layout_without_relocatable`), since they look like they make the type a value:

```zng
// Rust only refers to `c++::Canvas`: the layout is only checked in C++.
type c++::Canvas {
    #layout(size = 24, align = 8);
}
```

A C++ type that isn't trivially relocatable stays reference-only even with a layout, because Rust
values must be movable with `memcpy`, which would break a type whose move constructor does more
(e.g. one that points to itself). Rust moves values freely, including out of a `&mut T` with
`std::mem::swap` or `std::mem::replace`; for reference-only types, that's sound only because their
Rust wrappers are zero-sized, so such a move doesn't touch the C++ object.

To own such an object in Rust, hold it through a `std::unique_ptr`, constructed in C++ (e.g. with a
`std::make_unique<T>` instantiation, see [free C++ functions](./cpp_functions.md)). If the layout is
there on purpose, as a check, silence the warning with `#allow(layout_without_relocatable)` at the
top of the `.zng` file.

This may change once Rust supports immovable types natively, e.g. with a `Move` trait or pinning
ergonomics like `&pin mut`.

### Semantics of reference-only types

Reference-only types are implemented as ZSTs that can only be obtained by reference. While the Rust
representation is a ZST, when the reference flows into C++, the reference is pointing to the C++
object.

This can sometimes cause behavior that is safe and sound, but surprising and counterintuitive for
someone that expects them to represent the whole C++ object. If you assume that `RustType` is a
newtype wrapper around a reference to a `CppType` class in C++, instead of a ZST, you will find it
surprising that:

- `std::mem::sizeof::<RustType>()` is 0, not the size of `CppType`
- `std::mem::alignof::<RustType>()` is 1, not the align of `CppType`
- `std::mem::swap::<RustType>(a, b)` only swaps the first zero bytes of those, i.e. does nothing.

Those problem might be solved by the `extern type` language feature.

## Trivially Relocatable C++ Types

If we want to have C++ objects directly within the Rust stack, then things become a bit more
complicated. C++'s move semantics are not compatible with Rust's move semantics.

### C++ Move Semantics

For better or for worse, C++ has very complex machinery to enable efficient movement of data in
memory. In the general case, it uses
[`rvalue` references](https://en.cppreference.com/cpp/language/value_category) in order to convey
that the object behind it is "going to expire soon-ish..." and their resources may be reused. When
you construct an `rvalue` reference (specifically an `xvalue`) with `std::move`, you may pass this
reference around to, for instance, a a move constructor which can use the fact that the provided
reference will expire soon, so instead of copying the underlying data, the constructor can
efficiently steal the underlying data.

Note that the C++ compiler will not prevent you from using the moved-from object again, and it
**will** run the moved-from object's destructor. This means that moved-from objects must remain in
a valid state at all times.

One consequence of this design is that C++'s move semantics are significantly broader than Rust's
since C++ enables library authors to run custom logic when moving a value in memory.

Furthermore, C++ allows you to `= delete` a move constructor which effectively prohibits a value
from moving in memory once constructed

### Rust's Move Semantics

Meanwhile, Rust's move semantics are very simple. When you move a value, the Rust compiler will
effectively copy the struct's bits and invalidate the moved from object. The Rust compiler will not
run the `Drop` implementation on the moved-from value.

### Trivial Relocatability

Rust's move semantics can be summarized as "all objects in Rust are trivially relocatable", and for
the purpose of `zngur` interop, a C++ object is considered trivially relocatable if performing a
trivial relocatable move on the object has the same observable effect as move constructing the C++
object in the new memory location and running the moved-from objects destructor.

A lot of C++ objects likely follow this guarantee. A few examples likely include:

- Plain old data types
- any `T` with a `T(T&&) = default;` implementation where all members are trivially relocatable.
- `std::vector<T>` regardless of `T`'s trivial relocatability.
  - Most other collections
- `std::unique_ptr` and `std::shared_ptr`
- `std::future`

A few **negative** examples are:

- Self referential types
- `libstdc++`'s `std::string` since small-string optimization makes it self-referential
  - NOTE: `stdc++`'s implementation of `std::string` does not do small-string optimization. That is
    to say, this is very tricky stuff...

### C++ Trivially Relocatable in Rust

`zngur` allows you to own a C++ object in the Rust stack so long as the C++ object meets the
trivial relocatability guarantees. Let's go over an example (you can follow along the full example
in `examples/stack_owned/`, where `cpp_type.h` aliases `using MyCppWrapper = CppType;` and
`using MyConservativeWrapper = CppConservativeType;`):

```zng
// main.zng
#cpp_additional_includes "
#include <cpp_type.h>
"

type c++::MyCppWrapper {
    #layout(size = 8, align = 4);
    #trivially_relocatable;
}
```

`#layout` gives the size and alignment of the C++ type, and `#trivially_relocatable` says that it
can be moved like a Rust value. With both, `zngur` stores it directly in the Rust stack.

Just like with C++ opaque objects, we can define functions on an `extern "C++"` block:

```zng
extern "C++" {
    safe fn create_cpp_type(i32, i32) -> c++::MyCppWrapper;
    safe fn print_cpp_type<'a>(&'a const c++::MyCppWrapper);
}
```

In C++, references to C++ objects (`&'a const c++::MyCppWrapper`) cross directly as C++ references
(`const ::MyCppWrapper&`; see [C++ references and pointers](./cpp_pointers.md#in-c)), while by-value
`rust::cpp::MyCppWrapper` provides `.cpp()` to access the inner C++ object.

```rust
// main.rs
mod generated;

fn main() {
    println!("Hello from Rust");
    let c = generated::create_cpp_type(10, 20);
    println!("Rust got CppType");
    generated::print_cpp_type(c.as_const());
    println!("Rust dropping CppType");
}
```

The wrapper unconditionally implements the following traits defined in the `zngur-lib` crate.

- The marker `ZngCppObject`
- The marker `ZngCppStackObject`
- The `ZngCppDestruct` trait with an `unsafe fn destruct(&mut self)`.

C++ types that Rust holds by value can also be constructed in place, through
[C++ constructors](./cpp_constructors.md).

#### Conservative Layout

When exact type size or alignment is not known ahead of time (e.g. across platforms or compiler
versions), you can specify an upper bound using `#layout_conservative`:

```zng
type c++::MyConservativeWrapper {
    #layout_conservative(size = 20, align = 8);
    #trivially_relocatable;
}
```

With `#layout_conservative`, `zngur` generates compile-time static assertions ensuring that
`sizeof(T) <= size` and `alignof(T) <= align`, while allocating an upper-bound buffer (rounded up
to a multiple of `align`) in the Rust wrapper.

### Type trait

`#trivially_relocatable` is checked in C++: the generated code `static_assert`s
`::rust::is_trivially_relocatable<T>`. That type trait is true for all `std::is_trivially_copyable`
types and for `std::unique_ptr` (with the default deleter). For other types, specialize it:

```c++
namespace rust {
template <> struct is_trivially_relocatable<CppType> : std::true_type {};
} // namespace rust
```

In a future where `std::is_trivially_relocatable` exists in the standard, we can specialize it for
all types.

If you can't (or don't want to) specialize the type trait, write `#unsafe(trivially_relocatable)`
instead. Rust still holds the type by value, but the C++ code doesn't check that it's trivially
relocatable: you promise it. This is meant for types that are relocatable but not trivially
copyable, like `[[clang::trivial_abi]]` types, and for tools that generate `.zng` files and already
know the answer. If the promise is wrong, moving the value in Rust is undefined behavior.

```zng
type c++::ns::Handle {
    #layout(size = 8, align = 8);
    #unsafe(trivially_relocatable);
}
```

> **NOTE**: `#cpp_stack_owned`, `#cpp_ref`, `#cpp_heap_allocated` and `#cpp_value` were removed.
> zngur explains how to replace them when it finds them.
