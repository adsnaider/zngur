#[rustfmt::skip]
mod generated;

#[derive(Debug, PartialEq, Eq)]
pub enum Unit {
    Unspecified,
    Meter,
}

#[repr(C)]
#[derive(Debug, PartialEq, Eq)]
pub struct Tuple(pub i32, pub i32);

pub mod enums {
    #[derive(Debug, PartialEq, Eq)]
    pub enum Tuple {
        Empty,
        Single,
    }
}

pub mod traits {
    pub trait Tuple {
        fn total(&self) -> i32;
    }
}

pub mod fns {
    use std::sync::atomic::{AtomicUsize, Ordering};

    pub static CALLS: AtomicUsize = AtomicUsize::new(0);

    #[allow(non_snake_case)]
    pub fn Unit() {
        CALLS.fetch_add(1, Ordering::SeqCst);
    }
}

fn main() {
    let _ = Unit::Meter;
    let _ = enums::Tuple::Empty;
    assert_eq!(generated::use_units(()), Unit::Unspecified);
    assert_eq!(generated::make_user_tuple((10, 32)), Tuple(10, 32));
    assert_eq!(generated::make_enum_tuple(), enums::Tuple::Single);
    assert_eq!(generated::make_trait_tuple().total(), 42);
    generated::call_unit_fn();
    assert_eq!(
        fns::CALLS.load(std::sync::atomic::Ordering::SeqCst),
        1
    );
}
