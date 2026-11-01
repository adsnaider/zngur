#include "cpp_type.h"

#include <iostream>
#include <utility>

#include "generated.h"

int CppType::with_rust_tag(rust::Ref<rust::crate::RustTag> tag) const {
  return x + y + tag.bonus();
}

namespace rust {

using cpp::BoxedInt;
using cpp::MyConservativeWrapper;
using cpp::MyCppWrapper;
using cpp::OpaqueRef;

MyCppWrapper Impl<MyCppWrapper>::new_(int32_t x, int32_t y) {
  return MyCppWrapper::build(x, y);
}

// The receivers (`&self`) are C++ references to the C++ objects.
Unit Impl<MyCppWrapper>::print(::CppType &c) {
  ::std::cout << "CppType " << c.x << " " << c.y << ::std::endl;
  return {};
}

MyConservativeWrapper
Impl<MyConservativeWrapper>::new_(int64_t a, int32_t b) {
  return MyConservativeWrapper::build(a, b);
}

Unit Impl<MyConservativeWrapper>::print(::CppConservativeType &c) {
  ::std::cout << "CppConservativeType " << c.a << " " << c.b << ::std::endl;
  return {};
}

BoxedInt Impl<BoxedInt>::new_(int32_t v) {
  return BoxedInt::build(new int(v));
}

int32_t Impl<BoxedInt>::get(::std::unique_ptr<int> &b) { return *b; }

int32_t Impl<OpaqueRef>::read(::OpaqueRef &r) { return r.value; }

int32_t exported_functions::run_with_opaque_ref() {
  ::OpaqueRef obj{7};
  const ::OpaqueRef &const_obj = obj;
  return crate::read_opaque(const_obj) + crate::inspect_mut_opaque(obj);
}

int32_t exported_functions::run_rust_cpp_refs() {
  ::CppType a(1, 10);
  ::CppType b(2, 20);
  bool pick_second = false;
  const ::CppType &picked = crate::flip_and_pick(a, b, pick_second);
  return pick_second ? picked.y : -1;
}

int32_t exported_functions::run_callback() {
  auto cb =
      Box<Dyn<Fn<const ::CppType &, bool &, const ::CppType &>>>::make_box(
          [](const ::CppType &item, bool &flag) -> const ::CppType & {
            flag = !flag;
            return item;
          });
  return crate::invoke_cpp_callback(::std::move(cb));
}

int32_t exported_functions::run_rust_nullable_refs() {
  int32_t v = 7;
  int32_t *scaled = crate::scale_if_some(&v, 3);
  int32_t *missing = crate::scale_if_some(nullptr, 3);
  return (scaled == &v && missing == nullptr) ? v : -1;
}

// References to C++ objects are C++ references: `const CppType&` and `int32_t&`.
Unit exported_functions::sum_into(const ::CppType &c, int32_t &out) {
  out = c.x + c.y;
  return {};
}

const ::CppType &exported_functions::larger_x(const ::CppType &a,
                                              const ::CppType &b) {
  return a.x >= b.x ? a : b;
}

const ::CppType *exported_functions::non_negative_x(const ::CppType *p) {
  return (p != nullptr && p->x >= 0) ? p : nullptr;
}

// A raw pointer (`CppPtr<MyCppWrapper>` is `CppType*`).
Unit exported_functions::swap_xy(CppPtr<MyCppWrapper> c) {
  ::std::swap(c->x, c->y);
  return {};
}

Unit exported_functions::invert_bool(CppPtr<Bool> b) {
  *b = !*b;
  return {};
}

const ::CppType &exported_functions::pick_larger_cpp_ref(const ::CppType &c,
                                                         int32_t &out_sum) {
  out_sum = c.x + c.y;
  return c;
}

Ref<CppPtr<int32_t>>
exported_functions::copy_i32_ptr(Ref<CppPtr<int32_t>> src,
                                 RefMut<CppPtr<int32_t>> dst) {
  *dst = *src;
  return src;
}

namespace {
struct CppSelectorImpl : crate::CppSelector {
  const ::CppType &select(const ::CppType &a, const ::CppType &b) override {
    return (a.x + a.y >= b.x + b.y) ? a : b;
  }
};
} // namespace

int32_t exported_functions::run_rust_struct_and_trait() {
  ::CppType a;
  a.x = 3;
  a.y = 4;
  ::CppType b;
  b.x = 10;
  b.y = 20;

  int32_t ctor_out = 0;
  crate::RustBorrow borrowed(a, ctor_out);
  int32_t r0 = borrowed.record_sum();

  crate::RustHolder holder(crate::RustCounter(5));
  int32_t r1 = holder.counter.add_cpp(a);
  Ref<crate::RustHolder> holder_ref = holder;
  int32_t r2 = holder_ref.counter.add_cpp(b);
  int32_t r3 = holder.borrow_counter().add_cpp(a);
  int32_t mut_out1 = 0;
  holder.borrow_counter_mut().add_cpp_mut(a, mut_out1);
  RefMut<crate::RustHolder> holder_mut = holder;
  int32_t mut_out2 = 0;
  holder_mut.counter.add_cpp_mut(b, mut_out2);

  auto sel = Box<Dyn<crate::CppSelector>>::make_box<CppSelectorImpl>();
  const ::CppType &chosen = crate::invoke_cpp_selector(::std::move(sel), a, b);
  return (ctor_out == 7 && r0 == 7 && r1 == 12 && r2 == 35 && r3 == 12 &&
          mut_out1 == 12 && mut_out2 == 42 && &chosen == &b)
             ? 42
             : -1;
}

Unit exported_functions::reset_wrapper_ptr(::CppType **dst,
                                           ::CppType *target) {
  *dst = target;
  return {};
}

Unit exported_functions::read_through_ptr_to_const_ptr(
    int32_t *const *in_ptr, const int32_t **out_ptr) {
  *out_ptr = *in_ptr;
  return {};
}

} // namespace rust
