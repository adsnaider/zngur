#include "cpp_type.h"

#include <iostream>
#include <utility>

#include "generated.h"

namespace rust {

rust::MyCppWrapper Impl<rust::MyCppWrapper>::new_(int32_t x, int32_t y) {
  return rust::MyCppWrapper::build(x, y);
}

rust::Unit
Impl<rust::MyCppWrapper>::print(rust::Ref<rust::MyCppWrapper> c) {
  const CppType &cpp = c.cpp();
  std::cout << "CppType " << cpp.x << " " << cpp.y << std::endl;
  return {};
}

rust::crate::MyConservativeWrapper
Impl<rust::crate::MyConservativeWrapper>::new_(int64_t a, int32_t b) {
  return rust::crate::MyConservativeWrapper::build(a, b);
}

rust::Unit
Impl<rust::crate::MyConservativeWrapper>::print(
    rust::Ref<rust::crate::MyConservativeWrapper> c) {
  const CppConservativeType &cpp = c.cpp();
  std::cout << "CppConservativeType " << cpp.a << " " << cpp.b << std::endl;
  return {};
}

// `CppConstPtr<rust::MyCppWrapper>` is `const CppType*`, and `CppMutPtr<int32_t>` is `int32_t*`.
rust::Unit exported_functions::sum_into(CppConstPtr<rust::MyCppWrapper> c,
                                        CppMutPtr<int32_t> out) {
  *out = c->x + c->y;
  return {};
}

rust::Unit exported_functions::swap_xy(CppMutPtr<rust::MyCppWrapper> c) {
  std::swap(c->x, c->y);
  return {};
}

CppConstPtr<rust::MyCppWrapper>
exported_functions::larger_x(const ::CppType *a, const ::CppType *b) {
  return a->x >= b->x ? a : b;
}

} // namespace rust
