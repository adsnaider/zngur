/// Marker trait for C++ objects: the Rust wrappers of `c++::` types (`Foo<C>`, for both
/// constnesses).
///
/// # Safety
///
/// Only generated C++ objects by zngur should implement this trait.
pub unsafe trait ZngCppObject: Sized {}

/// Whether a C++ object is `const` ([`Const`]) or not ([`NonConst`]). Every generated C++ type has
/// a constness parameter, which defaults to `NonConst`: `Foo` is `Foo<NonConst>` (a `Foo` in C++),
/// and `Foo<Const>` is a `const Foo`.
///
/// So a C++ `const Foo&` is a `&Foo<Const>` in Rust, and a `Foo&` a `&Foo` (shared, `&'a var Foo`
/// in the `.zng` file) or a `&mut Foo` (unique, `&'a unique var Foo`). `const` member functions
/// are implemented for `Foo<C>` (both constnesses), and the others only for `Foo` (`Foo<NonConst>`).
/// Going from a `Foo` to a `Foo<Const>` is explicit (`as_const()`), and there's no way back.
/// Builtins have the same parameter through [`Cpp<T, C>`](Cpp).
///
/// Like in C++, passing a non-`const` object where C++ takes a `const` one is implicit: the
/// arguments of C++ functions (member functions, free functions and constructors) that are
/// `const` objects are generic over their constness, e.g. `&'a Foo<C1>` for a `&'a const Foo`.
///
/// ```compile_fail
/// # use core::marker::PhantomData;
/// # use zngur_lib::*;
/// # pub struct Foo<C: Constness = NonConst>(PhantomData<C>);
/// # impl Foo { fn set(&mut self) {} }
/// fn f(foo: &mut Foo<Const>) {
///     foo.set(); // Error: methods on `Foo` (`Foo<NonConst>`) don't exist on `Foo<Const>`.
/// }
/// ```
pub trait Constness: sealed::Sealed + 'static {}

/// The constness of `const` C++ objects (see [`Constness`]).
pub enum Const {}

/// The constness of non-`const` C++ objects (see [`Constness`]), which is the default.
pub enum NonConst {}

impl sealed::Sealed for Const {}
impl sealed::Sealed for NonConst {}
impl Constness for Const {}
impl Constness for NonConst {}

/// The Rust builtins that can be C++ objects ([`Cpp<T, C>`](Cpp)): `bool`, the integers (`i8` to
/// `i64`, `u8` to `u64`, `isize` and `usize`), `f32`, `f64`, and raw pointers (`*const T` and
/// `*mut T`). (Note that `.zng` files only expose `usize` and fixed-width integers `i8`..=`i64` /
/// `u8`..=`u64`, not `isize`.) Not `char`, since C++ can store any value in a `char32_t`,
/// including invalid code points.
///
/// # Safety
///
/// Only zngur implements this trait (it's sealed): every bit pattern that C++ can store in the
/// corresponding C++ type must be a valid `Self`.
pub unsafe trait CppRustBuiltin: Copy + Unpin + sealed::SealedBuiltin {}

macro_rules! cpp_rust_builtins {
    ($($t:ty),*) => {$(
        impl sealed::SealedBuiltin for $t {}
        unsafe impl CppRustBuiltin for $t {}

        impl<C: Constness> From<Cpp<$t, C>> for $t {
            fn from(value: Cpp<$t, C>) -> Self {
                value.into_inner()
            }
        }
    )*};
}

cpp_rust_builtins!(
    bool, i8, i16, i32, i64, isize, u8, u16, u32, u64, usize, f32, f64
);

impl<T: ?Sized> sealed::SealedBuiltin for *const T {}
impl<T: ?Sized> sealed::SealedBuiltin for *mut T {}
unsafe impl<T: ?Sized> CppRustBuiltin for *const T {}
unsafe impl<T: ?Sized> CppRustBuiltin for *mut T {}

impl<T: ?Sized, C: Constness> From<Cpp<*const T, C>> for *const T {
    fn from(value: Cpp<*const T, C>) -> Self {
        value.into_inner()
    }
}

impl<T: ?Sized, C: Constness> From<Cpp<*mut T, C>> for *mut T {
    fn from(value: Cpp<*mut T, C>) -> Self {
        value.into_inner()
    }
}

/// A builtin (`i32`, `bool`, a raw pointer, ...; see [`CppRustBuiltin`]) that is a C++ object:
/// `Cpp<i32>` is a C++ `int32_t`, and `Cpp<i32, Const>` a `const int32_t` (see [`Constness`]).
///
/// References to C++ objects are spelled with `const` or `var` in `.zng` files: `&'a const i32` is
/// a `&'a Cpp<i32, Const>`, `&'a var i32` a `&'a Cpp<i32>`, and `&'a unique var i32` a
/// `&'a mut Cpp<i32>`. (A plain `&'a i32` is a Rust value, which C++ must treat with Rust's rules.)
///
/// C++ may change a C++ object through other references (or pointers) while Rust has a shared
/// reference to it, so `Cpp<T, C>` is an [`UnsafeCell`](core::cell::UnsafeCell), like
/// [`Cell`](core::cell::Cell): its value is read with [`get`](Cpp::get), and a non-`const` one
/// can be [`set`](Cpp::set) through a shared reference. `&mut Cpp<T, C>` is unique, as always in
/// Rust: that's what `unique` promises in the `.zng` file. It has the same layout as `T`.
///
/// ```
/// # use zngur_lib::*;
/// let x: Cpp<i32> = Cpp::new(1);
/// let shared = &x;
/// shared.set(shared.get() + 1);
/// assert_eq!(x, 2);
///
/// let mut y = 5;
/// Cpp::from_mut(&mut y).set(6); // E.g. to pass a Rust `i32` to C++ as an `int32_t*`.
/// assert_eq!(y, 6);
/// ```
///
/// A `const` one can't be changed:
///
/// ```compile_fail
/// # use zngur_lib::*;
/// let x: Cpp<i32, Const> = Cpp::new_const(1);
/// x.set(2); // Error: `set` only exists for `Cpp<i32, NonConst>`.
/// ```
#[repr(transparent)]
pub struct Cpp<T: CppRustBuiltin, C: Constness = NonConst> {
    value: core::cell::UnsafeCell<T>,
    _constness: core::marker::PhantomData<C>,
}

impl<T: CppRustBuiltin, C: Constness> Cpp<T, C> {
    /// The value, consuming the object.
    pub fn into_inner(self) -> T {
        self.value.into_inner()
    }

    /// The current value.
    pub fn get(&self) -> T {
        // SAFETY: `Cpp` isn't `Sync`, so nothing else writes to the value during this read on the
        // Rust side, and C++ keeps its own rules.
        unsafe { *self.value.get() }
    }

    /// A pointer to the value. It's only valid to write through it for non-`const` objects (see
    /// [`as_mut_ptr`](Cpp::as_mut_ptr) and [`set`](Cpp::set)).
    pub fn as_ptr(&self) -> *const T {
        self.value.get()
    }
}

impl<T: CppRustBuiltin> Cpp<T> {
    /// A (non-`const`) C++ object with the value `value`.
    pub const fn new(value: T) -> Self {
        Cpp {
            value: core::cell::UnsafeCell::new(value),
            _constness: core::marker::PhantomData,
        }
    }

    /// Sets the value, through a shared reference (like [`Cell::set`](core::cell::Cell::set)).
    pub fn set(&self, value: T) {
        // SAFETY: Like `Cell::set`: `Cpp` isn't `Sync`, and no reference to the value is handed out
        // from a shared reference.
        unsafe { *self.value.get() = value }
    }

    /// A mutable pointer to the value, through a shared reference (like
    /// [`Cell::as_ptr`](core::cell::Cell::as_ptr)).
    pub fn as_mut_ptr(&self) -> *mut T {
        self.value.get()
    }

    /// A mutable reference to the value, through the unique reference to the object.
    pub fn get_mut(&mut self) -> &mut T {
        self.value.get_mut()
    }

    /// The object as a `const` one, which can't be changed through the returned reference.
    pub fn as_const(&self) -> &Cpp<T, Const> {
        // SAFETY: The constness is only a marker: both have the same layout.
        unsafe { &*(self as *const Self as *const Cpp<T, Const>) }
    }

    /// The object as a `const` one, which can't be changed through the returned reference.
    pub fn as_const_mut(&mut self) -> &mut Cpp<T, Const> {
        // SAFETY: The constness is only a marker: both have the same layout.
        unsafe { &mut *(self as *mut Self as *mut Cpp<T, Const>) }
    }

    /// The object as a `const` one.
    pub fn into_const(self) -> Cpp<T, Const> {
        Cpp::new_const(self.into_inner())
    }

    /// A Rust value as a (non-`const`) C++ object, e.g. to pass it to C++ as a pointer (like
    /// [`Cell::from_mut`](core::cell::Cell::from_mut)).
    pub fn from_mut(value: &mut T) -> &mut Cpp<T> {
        // SAFETY: `Cpp<T>` has the same layout as `T`, and `value` is borrowed uniquely for as
        // long as the returned reference lives.
        unsafe { &mut *(value as *mut T as *mut Cpp<T>) }
    }
}

impl<T: CppRustBuiltin> Cpp<T, Const> {
    /// A `const` C++ object with the value `value`.
    pub const fn new_const(value: T) -> Self {
        Cpp {
            value: core::cell::UnsafeCell::new(value),
            _constness: core::marker::PhantomData,
        }
    }
}

/// Only for non-`const` objects, so that `x.into()` knows which one to build even where either
/// would do (e.g. `f(&x.into())`, where `f` takes a `&'a const i32`).
impl<T: CppRustBuiltin> From<T> for Cpp<T> {
    fn from(value: T) -> Self {
        Cpp::new(value)
    }
}

impl<T: CppRustBuiltin + core::fmt::Debug, C: Constness> core::fmt::Debug for Cpp<T, C> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.get().fmt(f)
    }
}

impl<T: CppRustBuiltin + PartialEq, C: Constness, C2: Constness> PartialEq<Cpp<T, C2>>
    for Cpp<T, C>
{
    fn eq(&self, other: &Cpp<T, C2>) -> bool {
        self.get() == other.get()
    }
}

impl<T: CppRustBuiltin + PartialEq, C: Constness> PartialEq<T> for Cpp<T, C> {
    fn eq(&self, other: &T) -> bool {
        self.get() == *other
    }
}

impl<T: CppRustBuiltin + PartialOrd, C: Constness, C2: Constness> PartialOrd<Cpp<T, C2>>
    for Cpp<T, C>
{
    fn partial_cmp(&self, other: &Cpp<T, C2>) -> Option<core::cmp::Ordering> {
        self.get().partial_cmp(&other.get())
    }
}

impl<T: CppRustBuiltin + PartialOrd, C: Constness> PartialOrd<T> for Cpp<T, C> {
    fn partial_cmp(&self, other: &T) -> Option<core::cmp::Ordering> {
        self.get().partial_cmp(other)
    }
}

impl<T: CppRustBuiltin + Eq, C: Constness> Eq for Cpp<T, C> {}

impl<T: CppRustBuiltin + Ord, C: Constness> Ord for Cpp<T, C> {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.get().cmp(&other.get())
    }
}

impl<T: CppRustBuiltin + Default> Default for Cpp<T> {
    fn default() -> Self {
        Cpp::new(T::default())
    }
}

// Note on `Sized` bound: For now all C++ objects are Sized. Stack objects are by definition Sized.
// Heap and Ref objects aren't as clear but they are currently represented as 0-sized types since
// extern_types don't exist yet. MaybeUninit<T> requires T: Sized for now. Maybe by the time we
// have extern types, this will change, since that could be the first !Sized referenced with a
// thin-pointer and there'll be more of a reason MaybeUninit would need to support T: ?Sized.

/// Trait for representing C++ objects in the Rust stack
///
/// # Safety
///
/// Only generated objects by zngur should implement this trait.
pub unsafe trait ZngCppStackObject: ZngCppObject {}

/// Trait for invoking the C++ destructor
///
/// # Safety
///
/// Implementer must abide to the documentation specified for the trait methods.
pub unsafe trait ZngCppDestruct: ZngCppObject {
    /// Calls the destructor for the C++ object
    ///
    /// # Safety
    ///
    /// * The object must be properly initialized.
    /// * The only things you can do with `self` after this call is
    ///   reinitialize it with a constructor call or drop it.
    unsafe fn destruct(&mut self);
}

/// A C++ constructor call with its arguments, which hasn't run yet: what the functions that zngur
/// generates for C++ constructors (`safe constructor(i32, i32) as from_xy;`) return. Running it
/// constructs the object in place, like in C++: [`build`](CppCtor::build) returns the object (for
/// C++ objects that Rust holds by value, which are trivially relocatable), and
/// [`init`](CppCtor::init) constructs it in a slot.
///
/// For an `unsafe` constructor, the generated function is `unsafe`: calling it is where the
/// constructor's preconditions are promised, and running the returned value is safe.
///
/// ```
/// # use core::mem::MaybeUninit;
/// # use zngur_lib::*;
/// # struct Bar(i32);
/// # unsafe impl ZngCppObject for Bar {}
/// # unsafe impl ZngCppStackObject for Bar {}
/// impl Bar {
///     // zngur generates this for `safe constructor(i32) as new;`, with a closure that calls C++.
///     pub fn new(i0: i32) -> impl CppCtor<Output = Self> {
///         // SAFETY: The closure initializes the slot that it gets.
///         unsafe { CppCtorFn::new(move |slot: *mut Self| slot.write(Bar(i0))) }
///     }
/// }
/// let bar = Bar::new(1).build();
/// let mut slot = MaybeUninit::uninit();
/// let in_slot: &mut Bar = Bar::new(2).init(&mut slot);
/// assert_eq!(bar.0 + in_slot.0, 3);
/// ```
///
/// # Safety
///
/// [`construct_at`](CppCtor::construct_at) must initialize the `Output` that `slot` points to.
#[must_use = "a C++ constructor call does nothing until it's run with `build()` or `init()`"]
pub unsafe trait CppCtor: Sized {
    /// The C++ object that it constructs.
    type Output: ZngCppObject;

    /// Runs the C++ constructor on `slot`, which is initialized afterwards (without dropping
    /// what was there).
    ///
    /// # Safety
    ///
    /// `slot` must be valid for writes and properly aligned.
    unsafe fn construct_at(self, slot: *mut Self::Output);

    /// Runs the C++ constructor on `slot`, and returns the object, which lives there.
    ///
    /// `MaybeUninit` doesn't drop what it contains, so the object's destructor only runs if the
    /// caller drops it (e.g. with [`MaybeUninit::assume_init_drop`]). Otherwise it leaks, which is
    /// safe.
    fn init(self, slot: &mut core::mem::MaybeUninit<Self::Output>) -> &mut Self::Output {
        // SAFETY: `slot` is a valid and aligned `MaybeUninit`, and `construct_at` initializes it.
        unsafe {
            self.construct_at(slot.as_mut_ptr());
            slot.assume_init_mut()
        }
    }

    /// Constructs an object and returns it. Moving it out of the slot it was constructed in is
    /// fine, because C++ objects that live on the Rust stack are trivially relocatable.
    fn build(self) -> Self::Output
    where
        Self::Output: ZngCppStackObject,
    {
        let mut slot = core::mem::MaybeUninit::uninit();
        // SAFETY: `slot` is a valid and aligned `MaybeUninit`, and `construct_at` initializes it.
        unsafe {
            self.construct_at(slot.as_mut_ptr());
            slot.assume_init()
        }
    }
}

/// A [`CppCtor`] that runs a closure, which zngur's generated code uses: the closure calls the C++
/// constructor with the arguments that it captured.
pub struct CppCtorFn<T, F: FnOnce(*mut T)> {
    construct_at: F,
    _output: core::marker::PhantomData<fn() -> T>,
}

impl<T, F: FnOnce(*mut T)> CppCtorFn<T, F> {
    /// A constructor call that runs `construct_at` on the slot.
    ///
    /// # Safety
    ///
    /// `construct_at` must initialize the `T` that it gets a (valid for writes and aligned)
    /// pointer to.
    pub const unsafe fn new(construct_at: F) -> Self {
        CppCtorFn {
            construct_at,
            _output: core::marker::PhantomData,
        }
    }
}

// SAFETY: `construct_at` initializes the slot (see `CppCtorFn::new`).
unsafe impl<T: ZngCppObject, F: FnOnce(*mut T)> CppCtor for CppCtorFn<T, F> {
    type Output = T;

    unsafe fn construct_at(self, slot: *mut T) {
        (self.construct_at)(slot)
    }
}

mod sealed {
    pub trait Sealed {}
    pub trait SealedBuiltin {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpp_builtin_layout() {
        assert_eq!(
            core::mem::size_of::<Cpp<f64>>(),
            core::mem::size_of::<f64>()
        );
        assert_eq!(
            core::mem::align_of::<Cpp<u8, Const>>(),
            core::mem::align_of::<u8>()
        );
        assert_eq!(
            core::mem::size_of::<Cpp<*const str>>(),
            core::mem::size_of::<*const str>()
        );
    }

    #[test]
    fn cpp_builtin_get_and_set() {
        let x: Cpp<i32> = Cpp::new(1);
        let (a, b) = (&x, &x);
        a.set(b.get() + 1);
        assert_eq!(x.get(), 2);
        assert_eq!(unsafe { *x.as_ptr() }, 2);
        let mut x = x;
        *x.get_mut() += 1;
        assert_eq!(x.into_inner(), 3);
    }

    #[test]
    fn cpp_builtin_conversions() {
        let x: Cpp<u64, Const> = Cpp::new_const(7);
        let y: u64 = x.into();
        assert_eq!(y, 7);
        let c: Cpp<bool> = Cpp::from(true);
        assert!(bool::from(c));
        let mut v = 0;
        let p: Cpp<*mut i32> = Cpp::new(&mut v as *mut i32);
        let p: *mut i32 = p.into();
        unsafe { *p = 4 };
        assert_eq!(v, 4);
    }

    /// What the inputs of C++ functions that take `const` objects look like (see `Constness`):
    /// `into()` and `Cpp::new` still know what to build.
    #[test]
    fn cpp_builtin_generic_constness_inference() {
        fn takes_const<C: Constness>(x: &Cpp<i32, C>) -> i32 {
            x.get()
        }
        let x: Cpp<i32> = Cpp::new(1);
        let c: Cpp<i32, Const> = Cpp::new_const(2);
        assert_eq!(takes_const(&x) + takes_const(&c), 3);
        assert_eq!(takes_const(&4.into()), 4);
        assert_eq!(takes_const(&Cpp::new(5)), 5);
    }

    #[test]
    fn cpp_builtin_constness() {
        let mut x: Cpp<i16> = Cpp::new(3);
        assert_eq!(x.as_const().get(), 3);
        assert_eq!(x.as_const_mut().get(), 3);
        let c: Cpp<i16, Const> = x.into_const();
        assert_eq!(c, 3);
    }

    #[test]
    fn cpp_builtin_from_mut() {
        let mut x = 1.5f32;
        let c = Cpp::from_mut(&mut x);
        c.set(c.get() * 2.0);
        assert_eq!(x, 3.0);
    }

    #[test]
    fn cpp_builtin_comparisons() {
        let a: Cpp<i32> = Cpp::new(1);
        let b: Cpp<i32, Const> = Cpp::new_const(2);
        assert!(a < b);
        assert!(a != b);
        assert_eq!(a, 1);
        assert_eq!(format!("{b:?}"), "2");
    }

    #[test]
    fn cpp_ctor_build_init_and_unexecuted_drop() {
        use core::cell::Cell;
        use core::mem::MaybeUninit;

        struct DropCounter<'a>(&'a Cell<usize>);
        impl Drop for DropCounter<'_> {
            fn drop(&mut self) {
                self.0.set(self.0.get() + 1);
            }
        }

        struct StackObj<'a> {
            value: i32,
            drops: &'a Cell<usize>,
        }
        impl Drop for StackObj<'_> {
            fn drop(&mut self) {
                self.drops.set(self.drops.get() + 1);
            }
        }
        unsafe impl ZngCppObject for StackObj<'_> {}
        unsafe impl ZngCppStackObject for StackObj<'_> {}

        fn make_ctor<'a>(
            value: i32,
            arg_drops: &'a Cell<usize>,
            obj_drops: &'a Cell<usize>,
        ) -> impl CppCtor<Output = StackObj<'a>> {
            let captured = DropCounter(arg_drops);
            unsafe {
                CppCtorFn::new(move |slot: *mut StackObj<'a>| {
                    let _ = &captured;
                    slot.write(StackObj {
                        value,
                        drops: obj_drops,
                    });
                })
            }
        }

        let arg_drops = Cell::new(0);
        let obj_drops = Cell::new(0);

        // `build()` constructs and returns by value, dropping captured arguments once run.
        {
            let built = make_ctor(10, &arg_drops, &obj_drops).build();
            assert_eq!(built.value, 10);
            assert_eq!(arg_drops.get(), 1);
            assert_eq!(obj_drops.get(), 0);
        }
        assert_eq!(obj_drops.get(), 1);

        // `init(&mut slot)` constructs in place without moving or dropping until `assume_init_drop`.
        let mut slot = MaybeUninit::uninit();
        let slot_addr = slot.as_ptr() as usize;
        let in_place = make_ctor(20, &arg_drops, &obj_drops).init(&mut slot);
        assert_eq!(in_place as *const StackObj<'_> as usize, slot_addr);
        assert_eq!(in_place.value, 20);
        assert_eq!(arg_drops.get(), 2);
        assert_eq!(obj_drops.get(), 1);
        unsafe { slot.assume_init_drop() };
        assert_eq!(obj_drops.get(), 2);

        // Dropping an un-executed `CppCtorFn` drops its captured non-`Copy` arguments without
        // constructing or dropping `Output`.
        let unexecuted = make_ctor(30, &arg_drops, &obj_drops);
        drop(unexecuted);
        assert_eq!(arg_drops.get(), 3);
        assert_eq!(obj_drops.get(), 2);
    }
}
