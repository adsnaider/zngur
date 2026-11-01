#include <cassert>
#include <iostream>

#include "./generated.h"

using rust::Ref;
using rust::RefMut;
using rust::Str;

// C++ implementations of the `extern "C++"` items. Lifetimes don't exist in C++, so these take
// and return plain `rust::Ref`s.
namespace rust {
Ref<Str> exported_functions::shortest(Ref<Str> a, Ref<Str> b) {
  return a.len() <= b.len() ? a : b;
}

Ref<Str> Impl<crate::Parser>::skip_and_rest(RefMut<crate::Parser> self, size_t n) {
  self.advance(n);
  return self.rest();
}
} // namespace rust

// A C++ implementation of a Rust trait whose method has a lifetime parameter.
class ShortestChooser : public rust::crate::Chooser {
public:
  Ref<Str> choose(Ref<Str> a, Ref<Str> b) override {
    return a.len() <= b.len() ? a : b;
  }
};

int main() {
  assert(rust::crate::identity("ok"_rs).len() == 2);

  auto longest = rust::crate::longest("short"_rs, "longer"_rs);
  std::cout << "C++: longest has " << longest.len() << " bytes" << std::endl;

  auto parser = rust::crate::Parser::new_("abc def"_rs);
  parser.advance(4);
  std::cout << "C++: rest has " << parser.rest().len() << " bytes" << std::endl;

  rust::crate::print_pair(rust::crate::Pair{"hi"_rs, "there"_rs});

  rust::crate::run_demo();

  rust::crate::use_chooser(
      rust::Box<rust::Dyn<rust::crate::Chooser>>::make_box<ShortestChooser>());
  rust::crate::use_chooser_fn(
      rust::Box<rust::Dyn<rust::Fn<Ref<Str>, Ref<Str>, Ref<Str>>>>::make_box(
          [](Ref<Str> a, Ref<Str> b) { return a.len() >= b.len() ? a : b; }));
}
