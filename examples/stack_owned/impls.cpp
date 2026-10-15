#include "cpp_type.h"

#include <iostream>
#include <utility>

#include "generated.h"

namespace rust {

rust::cpp::MyCppWrapper Impl<rust::cpp::MyCppWrapper>::new_(int32_t x, int32_t y) {
  return rust::cpp::MyCppWrapper::build(x, y);
}

rust::Unit
Impl<rust::cpp::MyCppWrapper>::print(rust::Ref<rust::cpp::MyCppWrapper> c) {
  const CppType &cpp = c.cpp();
  ::std::cout << "CppType " << cpp.x << " " << cpp.y << ::std::endl;
  return {};
}

rust::cpp::MyConservativeWrapper
Impl<rust::cpp::MyConservativeWrapper>::new_(int64_t a, int32_t b) {
  return rust::cpp::MyConservativeWrapper::build(a, b);
}

rust::Unit
Impl<rust::cpp::MyConservativeWrapper>::print(
    rust::Ref<rust::cpp::MyConservativeWrapper> c) {
  const CppConservativeType &cpp = c.cpp();
  ::std::cout << "CppConservativeType " << cpp.a << " " << cpp.b << ::std::endl;
  return {};
}

// `CppConstPtr<rust::cpp::MyCppWrapper>` is `const CppType*`, and `CppMutPtr<int32_t>` is `int32_t*`.
rust::Unit exported_functions::sum_into(CppConstPtr<rust::cpp::MyCppWrapper> c,
                                        CppMutPtr<int32_t> out) {
  *out = c->x + c->y;
  return {};
}

rust::Unit exported_functions::swap_xy(CppMutPtr<rust::cpp::MyCppWrapper> c) {
  ::std::swap(c->x, c->y);
  return {};
}

CppConstPtr<rust::cpp::MyCppWrapper>
exported_functions::larger_x(const ::CppType *a, const ::CppType *b) {
  return a->x >= b->x ? a : b;
}

} // namespace rust
