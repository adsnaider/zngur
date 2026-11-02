pub use aggregation::StatsAccumulator;
pub use receiver::Receiver;

use generated::cpp_processor::RunConfig;
use generated::std::unique_ptr;
use zngur_lib::Const;

pub struct Processor;

impl Processor {
    pub fn new() -> Self {
        Processor
    }

    pub fn run(
        &self,
        receiver: &mut receiver::Receiver,
        stats: &mut aggregation::StatsAccumulator,
        config: &unique_ptr<RunConfig, Const>,
    ) {
        let config = config.get().expect("no run config");
        let packets = config.packets().get();
        for _ in 0..packets {
            let packet = receiver.next_packet();
            stats.add_packet(packet);
        }
    }
}

#[rustfmt::skip]
#[path = "processor.zng.rs"]
mod generated;
