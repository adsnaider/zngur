#pragma once

#include <cstdint>

namespace objects {

struct Point {
  int32_t x;
  int32_t y;
};

struct Flag {
  bool active = false;
  const bool *watched = nullptr;

  Flag() = default;
};

struct Counter {
  int32_t count = 0;
  int32_t step = 1;
  Point origin = {0, 0};
  Counter *next = nullptr;

  explicit Counter(int32_t step) : step(step), origin{step, -step} {}

  // A non-`const` member function, which Rust calls through a shared reference
  // (`&'a var self`).
  void bump() {
    count += step;
    if (next != nullptr) {
      next->bump();
    }
  }

  // A `const` member function, which Rust calls through a unique reference to
  // a `const` object (`&'a unique const self`).
  int32_t total() const {
    return count + (next != nullptr ? next->total() : 0);
  }

  int32_t &count_ref() { return count; }

  // Takes a C++ object (`&'a unique var i32`).
  void add_to(int32_t &out) const { out += count; }

  // Takes a Rust value (`&'a i32`).
  void add(const int32_t &value) { count += value; }
};

inline void reset(Counter &c) { c.count = 0; }

inline void double_step(Counter &c) { c.step *= 2; }

inline int32_t peek(const Counter &c) { return c.count; }

// Pointers that may be null, which Rust sees as nullable references (`Option<&'a ...>`).
inline int32_t count_or(const Counter *c, int32_t fallback) {
  return c != nullptr ? c->count : fallback;
}

inline Counter *next_of(Counter &c) { return c.next; }

// Calls a Rust function with a nullable reference (defined in impls.cpp, which needs the generated
// header).
int32_t bump_in_rust(Counter *c);

} // namespace objects
