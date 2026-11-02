# Writing free functions in C++ for Rust

You can write C++ functions that operate on Rust or `c++::` types, and call them from Rust. (To call
existing C++ functions directly, see [Free C++ functions](./cpp_functions.md).)
First, you need to add that function signature in the `main.zng` file inside an `extern "C++"` block:

```zng
extern "C++" {
    safe fn new_blob_store_client(crate::Flags) -> crate::Reader;
}
```

Then, in the `generated.h` file, the function signature is declared like this:

```C++
namespace rust {
namespace exported_functions {
::rust::crate::Reader new_blob_store_client(::rust::crate::Flags i0);
}
} // namespace rust
```

You need to implement it in a `.cpp` file and link it to the final binary.
