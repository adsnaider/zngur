#[rustfmt::skip]
mod generated;

use zngur_lib::{Cpp, CppCtor};

fn main() {
    let d: generated::Pair<i32> = Default::default();
    let same = generated::Pair::<i32>::new(10, 20).build();
    let diff = generated::Pair::<i32>::new(10, 21).build();
    assert!(d == same);
    assert!(d != diff);

    let cp = generated::Pair::<Cpp<i32, zngur_lib::Const>>::new(7, 35).build();
    assert_eq!(i32::from(cp), 42);
}
