#include "impls.h"
#include "generated.h"
#include "task.h"
#include "zngur.h"
#include <new>

namespace rust {

using crate::RustTask;
using cpp::task::CppTask;
using cpp::task::Dispatcher;

// The receivers are C++ references to the C++ objects (here, to uninitialized memory that these
// constructors construct the objects in).
Unit Impl<Dispatcher>::constructor(::task::Dispatcher &dispatcher) {
  new (&dispatcher) ::task::Dispatcher();
  return {};
}

Unit Impl<CppTask>::constructor(::task::CppTask &task) {
  new (&task) ::task::CppTaskForRust();
  return {};
}

Unit Impl<Dispatcher>::run_task(::task::Dispatcher &d, RefMut<RustTask> task) {
  RawMut<RustTask> raw_mut(task);
  RustTask *rust_ptr = from_rust_ptr(raw_mut);
  ::task::CppTaskForRust *task_ref =
      ::task::CppTaskForRust::Inheritance::get_base(as_rust_ptr_mut(rust_ptr));
  d.run_task(task_ref);
  return {};
}

} // namespace rust

task::Poll task::CppTaskForRust::poll() {
  ::rust::RawMut<::rust::crate::RustTask> rust_future =
      Inheritance::get_rust(this);
  ::rust::RefMut<::rust::crate::RustTask> ref_mut =
      rust_future.as_mut_unchecked();
  ::rust::core::task::Poll<::rust::Unit> result =
      ::rust::crate::RustTask::poll(ref_mut);
  return result.is_ready() ? task::Poll::kReady : task::Poll::kPending;
}
