#include "generated.h"

namespace rust {

rust::cpp::Pair<::int32_t>
Impl<rust::cpp::Pair<::int32_t>, rust::core::default_::Default>::default_() {
    return rust::cpp::Pair<::int32_t>::build(10, 20);
}

rust::Bool Impl<
    rust::cpp::Pair<::int32_t>,
    rust::core::cmp::PartialEq<rust::cpp::Pair<::int32_t>>>::
    eq(rust::CppRef<rust::cpp::Pair<::int32_t>> self,
       rust::CppRef<rust::cpp::Pair<::int32_t>> other) {
    return self.a == other.a && self.b == other.b;
}

::int32_t
Impl<::int32_t, rust::core::convert::From<rust::cpp::Pair<
                    rust::CppConst<::int32_t>>>>::
    from(rust::cpp::Pair<rust::CppConst<::int32_t>> p) {
    return p.cpp().a + p.cpp().b;
}

}  // namespace rust
