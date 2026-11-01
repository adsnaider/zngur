#include "generated.h"

namespace {
class CppTupleImpl final : public rust::crate::traits::Tuple {
 public:
  ::int32_t total() override { return 42; }
};
}  // namespace

namespace rust::exported_functions {
rust::crate::Unit use_units(rust::Unit) {
  return rust::crate::Unit::Unspecified{};
}

rust::crate::Tuple make_user_tuple(rust::Tuple<::int32_t, ::int32_t> pair) {
  ::int32_t a = pair.f0;
  ::int32_t b = pair.f1;
  rust::Tuple<::int32_t, ::int32_t> roundtrip(a, b);
  ::int32_t x = roundtrip.f0;
  ::int32_t y = roundtrip.f1;
  return rust::crate::Tuple(x, y);
}

rust::crate::enums::Tuple make_enum_tuple() {
  return rust::crate::enums::Tuple::Single{};
}

rust::Box<rust::Dyn<rust::crate::traits::Tuple>> make_trait_tuple() {
  return rust::Box<rust::Dyn<rust::crate::traits::Tuple>>::make_box<
      CppTupleImpl>();
}

rust::Unit call_unit_fn() {
  rust::crate::fns::Unit();
  return {};
}
}  // namespace rust::exported_functions
