#pragma once
#include <cstdint>
#include <iostream>
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

// Free functions that Rust calls directly (declared in `main.zng`).
namespace geometry {
inline int dot(const CppType &a, const CppType &b) { return a.x * b.x + a.y * b.y; }
inline CppType origin() { return CppType(0, 0); }
inline int64_t twice(int64_t v) { return 2 * v; }
inline double twice(double v) { return 2 * v; }
} // namespace geometry

