#include "generated.h"

namespace objects {

// `rust::crate::bump_chain` takes `Option<&var c++::objects::Counter>`, a `Counter*` in C++.
int32_t bump_in_rust(Counter *c) { return rust::crate::bump_chain(c); }

} // namespace objects
