#[rustfmt::skip]
mod generated;

use generated::{BoxedInt, MyConservativeWrapper, MyCppWrapper, OpaqueRef};
use zngur_lib::{Const, Cpp};

trait CheckNotSend<M> {
    fn check() {}
}
impl<T: ?Sized> CheckNotSend<()> for T {}
struct IsSend;
impl<T: ?Sized + Send> CheckNotSend<IsSend> for T {}

trait CheckNotSync<M> {
    fn check() {}
}
impl<T: ?Sized> CheckNotSync<()> for T {}
struct IsSync;
impl<T: ?Sized + Sync> CheckNotSync<IsSync> for T {}

const _: fn() = <OpaqueRef as CheckNotSend<_>>::check;
const _: fn() = <OpaqueRef as CheckNotSync<_>>::check;

pub fn read_opaque(r: &OpaqueRef<Const>) -> i32 {
    let _ = r;
    7
}

pub fn inspect_mut_opaque(r: &mut OpaqueRef) -> i32 {
    let _shared_const: &OpaqueRef<Const> = r.as_const();
    let _unique_const: &mut OpaqueRef<Const> = r.as_const_mut();
    r.read()
}

#[repr(C)]
pub struct RustBorrow<'a>(pub &'a MyCppWrapper<Const>, pub &'a mut Cpp<i32>);

impl RustBorrow<'_> {
    pub fn record_sum(&mut self) -> i32 {
        let mut sum = 0;
        generated::sum_into(self.0, Cpp::from_mut(&mut sum));
        self.1.set(sum);
        sum
    }
}

#[repr(C)]
pub struct RustCounter(pub i32);

impl RustCounter {
    pub fn add_cpp(&self, w: &MyCppWrapper<Const>) -> i32 {
        let mut sum = 0;
        generated::sum_into(w, Cpp::from_mut(&mut sum));
        self.0 + sum
    }

    pub fn add_cpp_mut(&mut self, w: &MyCppWrapper<Const>, out: &mut Cpp<i32>) {
        let mut sum = 0;
        generated::sum_into(w, Cpp::from_mut(&mut sum));
        self.0 += sum;
        out.set(self.0);
    }
}

#[repr(C)]
pub struct RustHolder {
    pub counter: RustCounter,
}

impl RustHolder {
    pub fn borrow_counter(&self) -> &RustCounter {
        &self.counter
    }

    pub fn borrow_counter_mut(&mut self) -> &mut RustCounter {
        &mut self.counter
    }
}

pub trait CppSelector {
    fn select<'a>(
        &self,
        a: &'a MyCppWrapper<Const>,
        b: &'a MyCppWrapper<Const>,
    ) -> &'a MyCppWrapper<Const>;
}

pub fn invoke_cpp_selector<'a>(
    sel: Box<dyn CppSelector>,
    a: &'a MyCppWrapper<Const>,
    b: &'a MyCppWrapper<Const>,
) -> &'a MyCppWrapper<Const> {
    sel.select(a, b)
}

pub fn flip_and_pick<'a>(
    a: &'a MyCppWrapper<Const>,
    b: &'a MyCppWrapper<Const>,
    flag: &'a mut Cpp<bool>,
) -> &'a MyCppWrapper<Const> {
    flag.set(!flag.get());
    if flag.get() { b } else { a }
}

pub fn invoke_cpp_callback(
    cb: Box<dyn for<'a> Fn(&'a MyCppWrapper<Const>, &'a mut Cpp<bool>) -> &'a MyCppWrapper<Const>>,
) -> i32 {
    let item = MyCppWrapper::new(3, 30);
    let mut flag = true;
    let out = cb(item.as_const(), Cpp::from_mut(&mut flag));
    assert!(std::ptr::eq(out, item.as_const()));
    assert!(!flag);
    30
}

pub fn scale_if_some<'a>(v: Option<&'a mut Cpp<i32>>, factor: i32) -> Option<&'a mut Cpp<i32>> {
    let v = v?;
    v.set(v.get() * factor);
    Some(v)
}

fn main() {
    let mut c = MyCppWrapper::new(5, 6);
    c.print();

    // C++ functions taking and returning references to C++ objects, which are safe: Rust
    // references in Rust (`Cpp::from_mut` makes a Rust `i32` a C++ object), C++ references in C++.
    // Like in C++, an argument that is a `const` object takes a non-`const` one too.
    let mut sum = 0;
    generated::sum_into(&c, Cpp::from_mut(&mut sum));
    println!("sum = {sum}");
    // A C++-style pointer, which is a raw pointer in Rust (`*mut T`): Rust references coerce to it,
    // but calling the function is `unsafe`.
    unsafe { generated::swap_xy(&mut c) };
    c.print();
    let d = MyCppWrapper::new(1, 2);
    let larger: &MyCppWrapper<Const> = generated::larger_x(&c, &d);
    assert!(std::ptr::eq(larger, c.as_const()));
    // Each `const` object input has its own constness type parameter (`__C1`, `__C2`).
    let larger_mixed: &MyCppWrapper<Const> = generated::larger_x(&c, d.as_const());
    assert!(std::ptr::eq(larger_mixed, c.as_const()));
    assert!(std::ptr::eq(
        generated::non_negative_x(Some(&c)).unwrap(),
        c.as_const()
    ));
    let neg = MyCppWrapper::new(-1, 2);
    assert!(generated::non_negative_x(Some(&neg)).is_none());
    assert!(generated::non_negative_x(None::<&MyCppWrapper>).is_none());
    let mut point = MyCppWrapper::new(2, 3);
    let _exclusive: &mut MyCppWrapper<Const> = point.as_const_mut();
    let _frozen: MyCppWrapper<Const> = point.into_const();

    let mut flag = false;
    unsafe { generated::invert_bool(Cpp::from_mut(&mut flag)) };
    assert!(flag);

    let mut ref_sum = 0;
    let echoed_ptr =
        unsafe { generated::pick_larger_cpp_ref(c.as_const(), Cpp::from_mut(&mut ref_sum)) };
    assert!(std::ptr::eq(echoed_ptr, c.as_const()));
    assert_eq!(ref_sum, 11);

    let mut target_val = 99;
    let src_ptr: *mut Cpp<i32> = Cpp::from_mut(&mut target_val);
    let mut dst_ptr: *mut Cpp<i32> = std::ptr::null_mut();
    let returned_ref = unsafe { generated::copy_i32_ptr(&src_ptr, &mut dst_ptr) };
    assert_eq!(*returned_ref, src_ptr);
    assert_eq!(dst_ptr, src_ptr);

    let mut w_ptr: Cpp<*mut MyCppWrapper> = Cpp::new(std::ptr::null_mut());
    unsafe { generated::reset_wrapper_ptr(&mut w_ptr, &mut c) };
    assert_eq!(w_ptr.get(), &mut c as *mut MyCppWrapper);

    let inner_ptr: Cpp<*mut Cpp<i32>> = Cpp::new(src_ptr);
    let mut out_const_ptr: Cpp<*const Cpp<i32, Const>> = Cpp::new(std::ptr::null());
    unsafe {
        generated::read_through_ptr_to_const_ptr(inner_ptr.as_const(), &mut out_const_ptr);
    }
    assert_eq!(unsafe { (*out_const_ptr.get()).get() }, 99);
    assert_eq!(generated::run_rust_struct_and_trait(), 42);

    assert_eq!(std::mem::size_of::<MyConservativeWrapper>(), 24);
    assert_eq!(std::mem::align_of::<MyConservativeWrapper>(), 8);
    assert_eq!(std::mem::size_of::<OpaqueRef>(), 0);
    assert_eq!(generated::run_with_opaque_ref(), 14);
    assert_eq!(generated::run_rust_cpp_refs(), 20);
    assert_eq!(generated::run_callback(), 30);
    assert_eq!(generated::run_rust_nullable_refs(), 21);

    let mut vec = Vec::new();
    let cons = MyConservativeWrapper::new(10, 20);
    vec.push(cons);
    vec[0].print();

    let boxed = BoxedInt::new(42);
    assert_eq!(boxed.get(), 42);
}
