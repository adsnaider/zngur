#include <iostream>

#include <aggregation.zng.h>
#include <packet.zng.h>
#include <processor.zng.h>
#include <receiver.zng.h>

int main() {
  rust::std::option::Option<rust::processor::Processor> processor = rust::std::option::Option<rust::processor::Processor>::Some(
      rust::processor::Processor::new_());
  rust::std::option::Option<rust::receiver::Receiver> receiver = rust::std::option::Option<rust::receiver::Receiver>::Some(
      rust::receiver::Receiver::new_());
  auto stats =
      rust::Impl<rust::cpp::cpp_stats::StatsAccumulator, rust::Inherent>::create();
  // Rust reads the config through `std::unique_ptr<RunConfig>`, an instantiation declared in
  // processor.zng with the members of aggregation.zng's `type<T> c++::std::unique_ptr<T>`.
  auto config =
      rust::cpp::std::unique_ptr<rust::cpp::cpp_processor::RunConfig>::build(
          new ::cpp_processor::RunConfig{5});

  std::cout << "Starting LatencyAnalysis simulation..." << std::endl;

  // `stats` is a `std::unique_ptr`; Rust takes a C++ reference to the object it owns, and to the
  // `std::unique_ptr<RunConfig>` (the C++ object in `config`).
  processor.unwrap().run(receiver.unwrap(), *stats.cpp(), config.cpp());

  rust::Impl<rust::cpp::cpp_stats::StatsAccumulator, rust::Inherent>::print_report(
      *stats.cpp());

  return 0;
}
