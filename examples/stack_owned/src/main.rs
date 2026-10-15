#[rustfmt::skip]
mod generated;

use std::mem::MaybeUninit;

use generated::{MyConservativeWrapper, MyCppWrapper};
use zngur_lib::{CppPtr, ZngCppConstruct};

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

    // C++ member functions, called directly.
    println!("sum() = {}", unsafe { MyCppWrapper::sum(c.as_ref()) });
    unsafe { MyCppWrapper::scale(c.as_mut(), 10) };
    unsafe { MyCppWrapper::add(c.as_mut(), d.as_ref()) };
    c.print();
    let x = unsafe { *MyCppWrapper::x_ref(c.as_ref()).as_ref().unwrap() };
    println!("x_ref() = {x}");
    unsafe { MyCppWrapper::swapped(c.as_ref()) }.print();
    MyCppWrapper::diagonal(7).print();

    // C++ constructors. `build` returns the object, and `construct` constructs it in a slot.
    let built = MyCppWrapper::build((3, 4));
    built.print();
    let copy = unsafe { MyCppWrapper::build_unchecked((built.as_ref(),)) };
    copy.print();
    let mut slot = MaybeUninit::uninit();
    MyCppWrapper::construct(&mut slot, (8, 9));
    let constructed = unsafe { slot.assume_init() };
    constructed.print();
    MyConservativeWrapper::build((30, 40)).print();

    assert_eq!(std::mem::size_of::<MyConservativeWrapper>(), 32);
    assert_eq!(std::mem::align_of::<MyConservativeWrapper>(), 16);

    let mut vec = Vec::new();
    let cons = MyConservativeWrapper::new(10, 20);
    vec.push(cons);
    vec[0].print();

    // Moving a stack-owned value into a C++ member function: C++ destroys it, not Rust.
    let other = MyConservativeWrapper::new(1, 2);
    let total = unsafe { MyConservativeWrapper::total_with(vec[0].as_ref(), other) };
    println!("total_with() = {total}");
}
