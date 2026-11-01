# Lifetimes

Rust references in a `.zng` file can have lifetimes, written and declared like in Rust:

```zng
type<'a> crate::Parser<'a> {
    #layout(size = 24, align = 8);

    fn new(&'a str) -> crate::Parser<'a>;
    fn rest(&self) -> &'a str;
    fn peek<'b>(&'b self) -> ::std::option::Option<&'b str>;
}

trait crate::Chooser {
    fn choose<'a>(&self, &'a str, &'a str) -> &'a str;
}

type Box<dyn for<'a> Fn(&'a str, &'a str) -> &'a str> {
    #layout(size = 16, align = 8);
}

extern "C++" {
    safe fn shortest<'a>(&'a str, &'a str) -> &'a str;

    impl<'a> crate::Parser<'a> {
        safe fn skip_and_rest(&mut self, usize) -> &'a str;
    }
}
```

- A function or method declares its lifetimes in `<...>`, before any type arguments:
  `fn f<'a, T>(...)`. (The type arguments still say which instantiation of a generic Rust function
  to call.)
- An `impl` block in `extern "C++"` declares them as `impl<'a>`, and a type as `type<'a>`. The type's
  methods can use the type's lifetimes.
- `dyn Fn(...)` declares them with `for<'a>`.
- References (`&'a T`, `&'a mut T`, and `&'a const T`, `&'a unique var T`, ... for
  [C++ objects](./call_cpp_from_rust/cpp_objects.md)), receivers (`&'a self`, `&'a const self`,
  ...), and lifetime arguments on Rust types (`::std::slice::Iter<'a, i32>`) use them. `'static`
  and `'_` need no declaration.
- References without a lifetime follow Rust's elision rules, as before.

Using a lifetime that isn't declared is an error.

## What they do

zngur writes the lifetimes into the Rust signatures that it generates:

- for `extern "C++"` functions and `impl` blocks, which C++ implements: `shortest` above is
  `pub fn shortest<'a>(i0: &'a str, i1: &'a str) -> &'a str`. Without the lifetime, Rust couldn't
  tell which argument the result borrows from, and the generated code wouldn't compile;
- for the methods of Rust traits that C++ implements, which must match the trait;
- for `Box<dyn for<'a> Fn(...)>`s made from C++ functions.

For Rust functions and methods that C++ calls, they're documentation: zngur calls them, and Rust
works out the lifetimes. In C++, lifetimes don't exist: `crate::Parser<'a>` is `rust::crate::Parser`,
and `&'a str` is `rust::Ref<rust::Str>`. Using references correctly in C++ is up to you (see
[Safety](./safety.md#lifetimes)).

C++ functions that Rust calls directly ([member functions](./call_cpp_from_rust/cpp_methods.md),
[free functions](./call_cpp_from_rust/cpp_functions.md),
[constructors](./call_cpp_from_rust/cpp_constructors.md)) take and return
[references](./call_cpp_from_rust/cpp_pointers.md) too: `&'a const T` for a C++ `const T&`,
`&'a unique var T` for a `T&` that Rust uses uniquely, and so on. Their lifetimes can't be checked
against C++, so zngur warns about elided ones (`elided_lifetimes`, silenced with `#allow(elided_lifetimes)` at the top of the file):

```zng
type c++::IntVec {
    #layout(size = 24, align = 8);
    #trivially_relocatable;

    unsafe fn at<'a>(&'a const self, usize) -> &'a const i32; // borrows from the vector...
    safe fn push_back<'a>(&'a unique var self, i32);          // ...so this can't be called meanwhile
}
```

See `examples/lifetimes` for a complete example.
