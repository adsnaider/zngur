#include "cpp_type.h"

#include <iostream>

#include "generated.h"

namespace rust {

using cpp::MyCppWrapper;
using crate::MyConservativeWrapper;

MyCppWrapper Impl<MyCppWrapper>::new_(int32_t x, int32_t y) {
  return MyCppWrapper::build(x, y);
}

Unit Impl<MyCppWrapper>::print(Ref<MyCppWrapper> c) {
  const CppType &cpp = c.cpp();
  std::cout << "CppType " << cpp.x << " " << cpp.y << std::endl;
  return {};
}

MyConservativeWrapper
Impl<MyConservativeWrapper>::new_(int64_t a, int32_t b) {
  return MyConservativeWrapper::build(a, b);
}

Unit Impl<MyConservativeWrapper>::print(Ref<MyConservativeWrapper> c) {
  const CppConservativeType &cpp = c.cpp();
  std::cout << "CppConservativeType " << cpp.a << " " << cpp.b << std::endl;
  return {};
}

} // namespace rust
