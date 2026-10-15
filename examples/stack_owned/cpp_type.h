#pragma once
#include <cstdint>
#include <iostream>
#include <memory>
#include <type_traits>
#include <zngur.h>

struct CppType {
  int x;
  int y;

  CppType() = default;
  CppType(const CppType &) = default;

  CppType(int x, int y) : x(x), y(y) {
    std::cout << "Constructed CppType " << x << " " << y << std::endl;
  }

  // Member functions that Rust calls directly (declared in `main.zng`).
  int sum() const { return x + y; }
  void scale(int factor) {
    x *= factor;
    y *= factor;
  }
  void add(const CppType &other) {
    x += other.x;
    y += other.y;
  }
  const int &x_ref() const { return x; }
  CppType swapped() const { return CppType(y, x); }
  static CppType diagonal(int v) { return CppType(v, v); }
};

struct CppConservativeType {
  int64_t a;
  int32_t b;

  CppConservativeType() = default;
  CppConservativeType(const CppConservativeType &) = default;

  CppConservativeType(int64_t a, int32_t b) : a(a), b(b) {
    std::cout << "Constructed CppConservativeType " << a << " " << b << std::endl;
  }

  ~CppConservativeType() {
    std::cout << "Destructed CppConservativeType " << a << " " << b << std::endl;
  }

  // Takes a stack-owned type by value.
  int64_t total_with(CppConservativeType other) const {
    return a + b + other.a + other.b;
  }
};

template <>
struct rust::is_trivially_relocatable<CppConservativeType> : std::true_type {};

// The `.zng` file names C++ types by their C++ path, so these aliases give them the names it uses.
using MyCppWrapper = CppType;
using MyConservativeWrapper = CppConservativeType;

// Free functions that Rust calls directly (declared in `main.zng`).
namespace geometry {
inline int dot(const CppType &a, const CppType &b) { return a.x * b.x + a.y * b.y; }
inline CppType origin() { return CppType(0, 0); }
inline int64_t twice(int64_t v) { return 2 * v; }
inline double twice(double v) { return 2 * v; }
} // namespace geometry

// A class template, whose instantiations Rust uses (declared in `main.zng`).
template <typename T> struct Pair {
  T first;
  T second;

  Pair(T first, T second) : first(first), second(second) {}
  ~Pair() { std::cout << "Destroyed Pair " << first << " " << second << std::endl; }
  T sum() const { return first + second; }
};

// The destructor makes `Pair` not trivially copyable, but it's still trivially relocatable.
template <typename T>
struct rust::is_trivially_relocatable<Pair<T>> : std::true_type {};

namespace pairs {
inline std::unique_ptr<Pair<int32_t>> boxed(int32_t first, int32_t second) {
  return std::unique_ptr<Pair<int32_t>>(new Pair<int32_t>(first, second));
}
} // namespace pairs


// C++ fields of different kinds: `const`, named like a zngur keyword (`type`), and a pointer.
struct Node {
  const int32_t id;
  int32_t type;
  Node *next;

  explicit Node(int32_t id) : id(id), type(0), next(nullptr) {}
};
