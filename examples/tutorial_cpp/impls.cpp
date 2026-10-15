#include "generated.h"
#include <string>

namespace rust {

using std::ffi::CStr;
using std::fmt::Debug;
using std::fmt::Formatter;
using std::fmt::Result;
using cpp::cpp_inventory::Inventory;
using cpp::cpp_inventory::Item;

// The Rust wrappers of `std::unique_ptr<Inventory>` and `std::unique_ptr<Item>`. (C++ types
// declared in the `.zng` file have their wrappers in `rust::cpp`. Inside `namespace rust`, `std::`
// is the namespace of Rust's standard library types; the C++ standard library is `::std::`.)
using InventoryPtr = cpp::std::unique_ptr<Inventory>;
using ItemPtr = cpp::std::unique_ptr<Item>;

static Ref<Str> rust_str_from_c_str(const char* input) {
  return CStr::from_ptr(reinterpret_cast<const int8_t*>(input)).to_str().expect("invalid_utf8"_rs);
}

InventoryPtr Impl<Inventory>::new_empty(uint32_t space) {
  return InventoryPtr::build(new ::cpp_inventory::Inventory(space));
}

Unit Impl<Inventory>::add_banana(RefMut<Inventory> self, uint32_t count) {
  self.cpp().add_banana(count);
  return {};
}

Unit Impl<Inventory>::add_item(RefMut<Inventory> self, ItemPtr item) {
  self.cpp().add_item(::std::move(*item.cpp()));
  return {};
}

ItemPtr Impl<Item>::new_(Ref<Str> name, uint32_t size) {
  return ItemPtr::build(new ::cpp_inventory::Item{
      .name = ::std::string(reinterpret_cast<const char *>(name.as_ptr()),
                            name.len()),
      .size = size});
}

Result Impl<Inventory, Debug>::fmt(Ref<Inventory> self, RefMut<Formatter> f) {
  ::std::string result = "Inventory { remaining_space: ";
  result += ::std::to_string(self.cpp().remaining_space);
  result += ", items: [";
  bool is_first = true;
  for (const auto &item : self.cpp().items) {
    if (!is_first) {
      result += ", ";
    } else {
      is_first = false;
    }
    result += "Item { name: \"";
    result += item.name;
    result += "\", size: ";
    result += ::std::to_string(item.size);
    result += " }";
  }
  result += "] }";
  return f.write_str(rust_str_from_c_str(result.c_str()));
}

} // namespace rust
