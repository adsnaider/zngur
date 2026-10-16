pub use aggregation::StatsAccumulator;
pub use receiver::Receiver;

use generated::cpp_processor::RunConfig;
use generated::std::unique_ptr;

pub struct Processor;

impl Processor {
    pub fn new() -> Self {
        Processor
    }

    pub fn run(
        &self,
        receiver: &mut receiver::Receiver,
        stats: &mut aggregation::StatsAccumulator,
        config: &unique_ptr<RunConfig>,
    ) {
        // SAFETY: `config` owns a valid `RunConfig`, which outlives this call.
        let config = unsafe { config.get().as_ref() }.expect("null");
        let packets = *config.packets();
        for _ in 0..packets {
            let packet = receiver.next_packet();
            stats.add_packet(packet);
        }
    }
}

#[rustfmt::skip]
#[path = "processor.zng.rs"]
mod generated;
