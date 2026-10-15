# C++ class templates

`c++::` types can be instantiations of C++ class templates, with the template arguments written as
zngur types:

```zng
type c++::std::unique_ptr<c++::geo::Point> {
    #layout(size = 8, align = 8);
    #cpp_stack_owned;

    unsafe fn get() const -> c++::geo::Point*;
}

type c++::std::map<i32, const c++::geo::Point*> {
    #cpp_ref;
}
```

The C++ type is the path with its template arguments: `::std::unique_ptr< ::geo::Point >` and
`::std::map< ::int32_t, const ::geo::Point* >` above. Template arguments can be `c++::` types,
builtins like `i32` or `bool`, and [C++ pointers and references](./cpp_pointers.md). Each
instantiation is declared separately, with its own layout, member functions, constructors and so
on.

## In Rust

All instantiations of a C++ class template are one generic Rust struct, so Rust code names them
the way C++ code does:

```rust
let p: generated::std::unique_ptr<generated::geo::Point> = make_point();
let raw = unsafe { generated::std::unique_ptr::<generated::geo::Point>::get(p.as_ref()) };
```

Member functions, constructors and fields belong to each instantiation, like in C++. So
`unique_ptr<Point>` only has the functions declared for it.

zngur marks `std::unique_ptr` (with the default deleter) as trivially relocatable, so it can be
`#cpp_stack_owned`. For your own templates, specialize `rust::is_trivially_relocatable` as usual:

```cpp
template <typename T>
struct rust::is_trivially_relocatable<Pair<T>> : std::true_type {};
```

## Limitations

- Every instantiation of a template must have the same number of template arguments.
- Two instantiations can't be the same Rust type. `const T&` and `const T*` arguments are both
  `CppPtr<T, Const>` in Rust.
- Non-type template arguments (`std::array<T, 4>`), `const` arguments (`unique_ptr<const T>`),
  function types (`std::function<int(int)>`) and nested types (`std::vector<int>::iterator`) aren't
  supported. Give them a name with a C++ alias instead, and use that:

  ```cpp
  namespace my { using IntFn = std::function<int(int)>; }
  ```

  ```zng
  type c++::my::IntFn { ... }
  ```
- All instantiations of a template must be declared in the same zngur module (crate). A module
  that imports another one can use the instantiations declared there, but not add its own.

> **Note:** The C++ wrappers of `c++::std::...` types are in the namespace `rust::std`. So, like for
> any other `c++::` namespace, C++ code inside `namespace rust { ... }` must refer to the standard
> library as `::std::...`, not `std::...`.
