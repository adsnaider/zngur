#pragma once
#include <cstdint>
#include <iostream>
#include <memory>
#include <type_traits>
#include <zngur.h>

namespace rust {
namespace crate {
struct RustTag;
} // namespace crate
} // namespace rust

struct CppType {
  int x;
  int y;

  CppType() = default;
  CppType(const CppType &) = default;

  CppType(int x, int y) : x(x), y(y) {
    std::cout << "Constructed CppType " << x << " " << y << std::endl;
  }

  // Takes a pointer, which must not be null, so Rust can only call it with `unsafe`.
  CppType(const CppType *other, int dx) : x(other->x + dx), y(other->y) {}

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
  int &x_ref() { return x; }
  void bump_x(int delta) { x += delta; }
  bool is_positive(bool strict) const {
    return strict ? (x > 0 && y > 0) : (x >= 0 && y >= 0);
  }
  int with_rust_tag(rust::Ref<rust::crate::RustTag> tag) const;
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

struct OpaqueRef {
  int value;
};

// The `.zng` file names C++ types by their C++ path, so these aliases give them the names it uses.
using MyCppWrapper = CppType;
using MyConservativeWrapper = CppConservativeType;
using BoxedInt = std::unique_ptr<int>;

// Free functions that Rust calls directly (declared in `main.zng`).
namespace geometry {
inline int dot(const CppType &a, const CppType &b) { return a.x * b.x + a.y * b.y; }
inline CppType origin() { return CppType(0, 0); }
inline const CppType &pick_first(const CppType &a, const CppType &) { return a; }
inline int sum_from_ptr(const CppType *p) { return p->x + p->y; }
struct Tag {};
inline int read_tag(const Tag &) { return 9; }
inline int64_t twice(int64_t v) { return 2 * v; }
inline double twice(double v) { return 2 * v; }
} // namespace geometry

// C++ fields of different kinds: `const`, named like a zngur keyword (`type`), and a pointer.
struct Node {
  const int32_t id;
  int32_t type;
  Node *next;

  explicit Node(int32_t id) : id(id), type(0), next(nullptr) {}
};
