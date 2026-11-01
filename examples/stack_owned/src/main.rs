#[rustfmt::skip]
mod generated;

use std::mem::MaybeUninit;

use generated::{BoxedInt, MyConservativeWrapper, MyCppWrapper, OpaqueRef};
use zngur_lib::{Const, Cpp, CppCtor};

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

#[derive(Clone, Copy)]
pub struct RustTag {
    pub bonus: i32,
}

impl RustTag {
    pub fn bonus(&self) -> i32 {
        self.bonus
    }
}

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

    // C++ member functions, which are methods: `&self` for `const` member functions and
    // `&mut self` for the others. A C++ `const T&` is a `&T<Const>`, and a `const int32_t&` a
    // `&Cpp<i32, Const>`, whose value `get()` reads. Like in C++, an argument that is a `const`
    // object takes a non-`const` one too: `c.add(&d)`.
    println!("sum() = {}", c.sum());
    c.scale(10);
    c.add(&d);
    c.print();
    let x = c.x_ref().get();
    println!("x_ref() = {x}");
    assert_eq!(unsafe { MyCppWrapper::raw_sum(&raw const c) }, 113);
    {
        let (r1, r2) = (&c, &c);
        r1.bump_x(5);
        assert_eq!(r2.x_ref().get(), 66);
        r1.bump_x(-5);
    }
    c.x_ref_mut().set(62);
    assert_eq!(c.x_ref().get(), 62);
    c.x_ref_mut().set(61);
    assert!(c.is_positive(true));
    assert!(!neg.is_positive(true));
    assert!(c.is_positive(false));
    assert_eq!(c.with_rust_tag(&RustTag { bonus: 100 }), 213);
    c.swapped().print();
    MyCppWrapper::diagonal(7).print();

    // C++ constructors, which return the constructor call (a `CppCtor`): `build()` returns the
    // object, and `init()` constructs it in a slot. Calling an `unsafe` constructor is `unsafe`.
    let built = MyCppWrapper::from_xy(3, 4).build();
    built.print();
    let copy = MyCppWrapper::copy(&built).build();
    copy.print();
    let moved = unsafe { MyCppWrapper::shifted(&raw const built, 10) }.build();
    moved.print();
    let mut slot = MaybeUninit::uninit();
    // `MaybeUninit` doesn't run the destructor (which `CppType` doesn't have anyway).
    let constructed: &mut MyCppWrapper = MyCppWrapper::from_xy(8, 9).init(&mut slot);
    constructed.print();
    MyConservativeWrapper::from_values(30, 40).build().print();

    // Free C++ functions.
    let dot = generated::geometry::dot(&built, constructed);
    println!("dot = {dot}");
    generated::geometry::origin().print();
    assert!(std::ptr::eq(
        generated::geometry::pick_first(&built, constructed),
        built.as_const()
    ));
    println!("sum_from_ptr = {}", unsafe {
        generated::geometry::sum_from_ptr(&raw const built)
    });
    let _tag_fn: fn(&generated::geometry::Tag<Const>) -> i32 = generated::geometry::Tag::<Const>;
    println!(
        "twice = {} {}",
        generated::geometry::twice(21),
        generated::geometry::twice_f64(1.25)
    );

    // `const` is part of the type: `MyCppWrapper<Const>` is a `const` object, which only has the
    // `const` member functions. Going from a `T` to a `T<Const>` is explicit, apart from passing
    // it to C++ as a `const` object.
    let mut point = MyCppWrapper::from_xy(2, 3).build();
    let exclusive: &mut MyCppWrapper<Const> = point.as_const_mut();
    println!("const sum() = {}", exclusive.sum());
    assert_eq!(unsafe { MyCppWrapper::raw_sum(exclusive) }, 5);
    let frozen: MyCppWrapper<Const> = MyCppWrapper::from_xy(4, 5).build().into_const();
    println!("frozen sum() = {}", frozen.sum());

    let mut flag = false;
    unsafe { generated::invert_bool(Cpp::from_mut(&mut flag)) };
    assert!(flag);

    let mut ref_sum = 0;
    let echoed_ptr =
        unsafe { generated::pick_larger_cpp_ref(c.as_const(), Cpp::from_mut(&mut ref_sum)) };
    assert!(std::ptr::eq(echoed_ptr, c.as_const()));
    assert_eq!(ref_sum, 113);

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

    // Moving a stack-owned value into a C++ member function: C++ destroys it, not Rust.
    let other = MyConservativeWrapper::new(1, 2);
    let total = vec[0].total_with(other);
    println!("total_with() = {total}");

    let boxed = BoxedInt::new(42);
    assert_eq!(boxed.get(), 42);
}
