# Writing `impl` blocks for Rust or C++ types in C++

## Inherent `impl`

You can write inherent `impl` blocks for types defined in your crate
(this is not a Zngur restriction, it's the orphan rule) in C++.
First, you need to declare the functions of that block in the `main.zng` file:

```zng
extern "C++" {
    impl crate::TagList {
        safe fn get_value_by_key(&self, &str) -> ::std::option::Option<&str>;
    }
}
```

Then, some class like this will be generated in the `generated.h` file:

```C++
namespace rust {
template <> class Impl<::rust::crate::TagList> {
public:
  static ::rust::std::option::Option<::rust::Ref<::rust::Str>>
  get_value_by_key(::rust::Ref<::rust::crate::TagList> self,
                   ::rust::Ref<::rust::Str> i0);
};
}
```

And you need to implement that in a `.cpp` file, and link it to the crate:

```C++
rust::std::option::Option<rust::Ref<rust::Str>>
rust::Impl<TagList>::get_value_by_key(rust::Ref<TagList> self,
                                      rust::Ref<rust::Str> key) {
  // Your code here
}
```

## Implementing Rust traits for Rust types

You can write `impl Trait for Type` blocks for types defined in your crate
or traits defined in your crate (this is not a Zngur restriction, it's the orphan rule) in C++.
First, you need to declare the functions of that block in the `main.zng` file:

```zng
extern "C++" {
    impl ::std::ops::Index<usize, Output = crate::Node> for crate::WayNodeList {
        safe fn index(&self, usize) -> &crate::Node;
    }
}
```

Then, some class like this will be generated in the `generated.h` file:

```C++
namespace rust {
template <>
class Impl<::rust::crate::WayNodeList,
           ::rust::std::ops::Index<::size_t, ::rust::crate::Node>> {
public:
  static ::rust::Ref<::rust::crate::Node>
  index(::rust::Ref<::rust::crate::WayNodeList> self, ::size_t i0);
};
}
```

And you need to implement that in a `.cpp` file, and link it to the crate:

```C++
rust::Ref<Node>
rust::Impl<WayNodeList, rust::std::ops::Index<size_t, Node>>::index(
    rust::Ref<WayNodeList> self, size_t i) {
  // Your code here
}
```

## `impl` blocks for C++ types

`extern "C++"` blocks can also have `impl` blocks for `c++::` types, and functions:

```zng
extern "C++" {
    impl c++::geo::Point {
        safe fn print(&self);
    }
    safe fn larger_x<'a>(&'a const c++::geo::Point, &'a const c++::geo::Point) -> &'a const c++::geo::Point;
}
```

There, references to C++ objects, and the receivers, are C++ references, and nullable references
(`::std::option::Option<&'a const c++::geo::Point>`) are C++ pointers (see
[C++ references and pointers](./cpp_pointers.md#in-c)):

```C++
rust::Unit rust::Impl<rust::cpp::geo::Point>::print(::geo::Point& self) {
  std::cout << self.x << ", " << self.y << std::endl;
  return {};
}

const ::geo::Point& rust::exported_functions::larger_x(const ::geo::Point& a,
                                                        const ::geo::Point& b) {
  return a.x >= b.x ? a : b;
}
```
