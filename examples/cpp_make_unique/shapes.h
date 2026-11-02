#pragma once

#include <cstdint>
#include <iostream>

namespace shapes {

// Movable with `memcpy`: Rust can hold it by value, or in a `std::unique_ptr`.
struct Point {
  Point(int32_t x, int32_t y) : x(x), y(y) {}

  int32_t x;
  int32_t y;
};

// Not movable with `memcpy` (it has virtual functions and a destructor), so Rust can only refer to
// it. `std::make_unique` constructs it on the heap, for Rust to own through a `std::unique_ptr`.
class Canvas {
public:
  Canvas(int32_t width, int32_t height) : width_(width), height_(height) {
    std::cout << "Created a " << width_ << "x" << height_ << " canvas" << std::endl;
  }
  Canvas(const Canvas &) = delete;
  Canvas &operator=(const Canvas &) = delete;
  virtual ~Canvas() { std::cout << "Destroyed the canvas" << std::endl; }

  virtual void Draw(const Point &p) {
    std::cout << "Drawing (" << p.x << ", " << p.y << ")" << std::endl;
    ++points_;
  }
  int32_t Points() const { return points_; }

private:
  int32_t width_;
  int32_t height_;
  int32_t points_ = 0;
};

} // namespace shapes
