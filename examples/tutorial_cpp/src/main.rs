mod generated {
    include!(concat!(env!("OUT_DIR"), "/generated.rs"));
}

use generated::cpp_inventory::{Inventory, Item};
use generated::std::unique_ptr;

// `std::unique_ptr<Inventory>` owns the C++ inventory, which Rust refers to as `&Inventory` /
// `&mut Inventory`. These impls make it work like a `Box<Inventory>`.
impl std::ops::Deref for unique_ptr<Inventory> {
    type Target = Inventory;

    fn deref(&self) -> &Inventory {
        self.get().expect("empty std::unique_ptr")
    }
}

impl std::ops::DerefMut for unique_ptr<Inventory> {
    fn deref_mut(&mut self) -> &mut Inventory {
        self.get_mut().expect("empty std::unique_ptr")
    }
}

fn main() {
    let mut inventory = Inventory::new_empty(1000);
    inventory.add_banana(3);
    inventory.add_item(Item::new("apple", 5));
    dbg!(&*inventory);
}
