# Example: Lifetimes

Lifetimes in a `.zng` file: on `extern "C++"` functions and `impl` blocks, on trait methods
implemented in C++, on `Box<dyn for<'a> Fn(...)>`, and on a Rust type with a lifetime parameter.

To run this example:

```
make
./a.out
```
