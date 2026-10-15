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
      rust::Impl<rust::cpp_stats::StatsAccumulator, rust::Inherent>::create();

  std::cout << "Starting LatencyAnalysis simulation..." << std::endl;

  // `stats` is a `std::unique_ptr`; Rust takes a reference to the object it owns.
  processor.unwrap().run(receiver.unwrap(), *stats.cpp(), 5);

  rust::Impl<rust::cpp_stats::StatsAccumulator, rust::Inherent>::print_report(
      *stats.cpp());

  return 0;
}
