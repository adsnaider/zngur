#[rustfmt::skip]
mod generated;

use generated::{MyConservativeWrapper, MyCppWrapper};
use zngur_lib::CppPtr;

fn main() {
    let mut c = MyCppWrapper::new(5, 6);
    c.print();

    // C++ functions taking and returning C++ pointers and references.
    let mut sum = 0;
    unsafe { generated::sum_into(c.as_ref(), CppPtr::from(&mut sum)) };
    println!("sum = {sum}");
    unsafe { generated::swap_xy(c.as_mut()) };
    c.print();
    let d = MyCppWrapper::new(1, 2);
    let larger = unsafe { generated::larger_x(c.as_ref(), d.as_ref()) };
    assert_eq!(larger, c.as_ref());

    assert_eq!(std::mem::size_of::<MyConservativeWrapper>(), 32);
    assert_eq!(std::mem::align_of::<MyConservativeWrapper>(), 16);

    let mut vec = Vec::new();
    let cons = MyConservativeWrapper::new(10, 20);
    vec.push(cons);
    vec[0].print();
}
