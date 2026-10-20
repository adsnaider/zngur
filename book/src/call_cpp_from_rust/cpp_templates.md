# C++ class templates

`c++::` types can be instantiations of C++ class templates, with the template arguments written as
zngur types:

```zng
type c++::std::unique_ptr<c++::geo::Point> {
    #layout(size = 8, align = 8);
    #trivially_relocatable;

    safe fn get<'a>(&'a const self) -> ::std::option::Option<&'a var c++::geo::Point>;
}

type c++::std::map<i32, const c++::geo::Point*> {
}
```

The C++ type is the path with its template arguments: `::std::unique_ptr< ::geo::Point >` and
`::std::map< ::int32_t, const ::geo::Point* >` above. Template arguments can be `c++::` types,
builtins like `i32` or `bool`, [C++ references and pointers](./cpp_pointers.md), and `const` ones
(`c++::std::span<const i32>`, which is `std::span<Cpp<i32, Const>>` in Rust). Each
instantiation is declared separately, with its own layout, member functions, constructors and so
on.

## In Rust

Rust code names instantiations the way C++ code does:

```rust
let p: generated::std::unique_ptr<generated::geo::Point> = make_point();
let point: Option<&generated::geo::Point> = p.get();
```

`generated::std::unique_ptr<T>` is an alias, and each instantiation is its own Rust struct, with
only the functions declared for it. Member functions, constructors and fields belong to each
instantiation, like in C++. You can implement traits for an instantiation through the alias
(`impl Deref for unique_ptr<Point>`), but not add inherent methods to it. The alias's last
parameter is the constness: `unique_ptr<Point, Const>` is a `const std::unique_ptr<Point>`.

zngur marks `std::unique_ptr` (with the default deleter) as trivially relocatable, so it can be
held by value. For your own templates, specialize `rust::is_trivially_relocatable` as usual:

```cpp
template <typename T>
struct rust::is_trivially_relocatable<Pair<T>> : std::true_type {};
```

## Shared members: `type<T>`

A `type<T>` declaration gives members to every instantiation of a template, so they aren't repeated
for each one:

```zng
type<T> c++::std::unique_ptr<T> {
    #layout(size = 8, align = 8);
    #trivially_relocatable;

    safe fn get<'a>(&'a const self) -> ::std::option::Option<&'a var T>;
}

// Both get the layout, `#trivially_relocatable` and `get()`.
type c++::std::unique_ptr<c++::geo::Point> {}
type c++::std::unique_ptr<c++::geo::Line> {}
```

The module with the `type<T>` declaration is the template's *home*: it defines the template's Rust
alias.

## Across modules

To use a template in several zngur [modules](../import.md#import) (crates), declare its `type<T>`
in one module, and import that module from the others, which then share its alias, and get its
members for their own instantiations. Like Rust's orphan rule, an instantiation can be declared in
the home, or in a module that declares one of its template arguments. So `unique_ptr<Point>` goes
next to `Point`, and instantiations whose arguments are all builtins (`vector<i32>`) go in the
home.

```zng
// aggregation.zng
#crate aggregation

type<T> c++::std::unique_ptr<T> {
    #layout(size = 8, align = 8);
    #trivially_relocatable;
    safe fn get<'a>(&'a const self) -> ::std::option::Option<&'a var T>;
}
```

```zng
// processor.zng
import "../aggregation/aggregation.zng";

type c++::cpp_processor::RunConfig {}

// Has the layout and `get()` from aggregation.zng.
type c++::std::unique_ptr<c++::cpp_processor::RunConfig> {}
```

A module can also use the instantiations declared in the modules it imports, like their other
`c++::` types.

A module that instantiates a template without a `type<T>` in sight is its home, so a single module
needs no `type<T>` declaration. But such templates can't be shared: if two modules instantiate the
same template, one of them has to declare its `type<T>`, and the other one has to import it directly
(the homes of an imported module's own imports aren't visible).

See `examples/multiple_modules` for a complete example.

## Limitations

- Every instantiation of a template must have the same number of template arguments.
- Instantiations whose arguments are all `const` or builtins (`unique_ptr<const Point>`) can only
  be declared in the template's home, since `Const<Point>` isn't local to `Point`'s crate.
- Non-type template arguments (`std::array<T, 4>`),
  function types (`std::function<int(int)>`) and nested types (`std::vector<int>::iterator`) aren't
  supported. Give them a name with a C++ alias instead, and use that. An alias template keeps the
  type parameterized, so zngur sees an ordinary class template:

  ```cpp
  namespace my {
  template <typename R, typename A> using Fn = std::function<R(A)>;
  using IntFn = std::function<int(int)>;
  }
  ```

  ```zng
  type c++::my::Fn<i32, i32> { ... }
  type c++::my::IntFn { ... }
  ```

> **Note:** In C++, the wrappers of `c++::std::...` types are in the namespace `rust::cpp::std`, like
> those of other `c++::` types (e.g. `rust::cpp::std::unique_ptr< rust::cpp::geo::Point >`).
