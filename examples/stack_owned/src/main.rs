#[rustfmt::skip]
mod generated;

use generated::{BoxedInt, MyConservativeWrapper, MyCppWrapper, OpaqueRef};

pub fn read_opaque(r: &OpaqueRef) -> i32 {
    r.read()
}

fn main() {
    let c = MyCppWrapper::new(5, 6);
    c.print();

    assert_eq!(std::mem::size_of::<MyConservativeWrapper>(), 24);
    assert_eq!(std::mem::align_of::<MyConservativeWrapper>(), 8);
    assert_eq!(std::mem::size_of::<OpaqueRef>(), 0);
    assert_eq!(generated::run_with_opaque_ref(), 7);

    let mut vec = Vec::new();
    let cons = MyConservativeWrapper::new(10, 20);
    vec.push(cons);
    vec[0].print();

    let boxed = BoxedInt::new(42);
    assert_eq!(boxed.get(), 42);
}
