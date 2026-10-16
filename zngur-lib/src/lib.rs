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
/// `const` member functions are available on both, and the others only on `Foo<NonConst>`. So a
/// C++ `const Foo&` is a `&Foo<Const>` in Rust, and a `Foo&` a `&mut Foo`. Going from a `Foo` to a
/// `Foo<Const>` is explicit (`as_const()`), and there's no way back.
///
/// ```compile_fail
/// # use core::marker::PhantomData;
/// # use zngur_lib::*;
/// # pub struct Foo<C: Constness = NonConst>(PhantomData<C>);
/// # impl Foo { fn set(&mut self) {} }
/// fn f(foo: &mut Foo<Const>) {
///     foo.set(); // Error: non-`const` member functions don't exist on `Foo<Const>`.
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

/// A `const` builtin or pointer, where a C++ type must be named as `const`: a C++ template argument
/// (`c++::std::span<const i32>` is `std::span<ConstValue<i32>>`). `const` C++ classes have their
/// own constness parameter instead (`Foo<Const>`).
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct ConstValue<T>(T);

impl<T> ConstValue<T> {
    /// Makes `value` `const`.
    pub const fn new(value: T) -> Self {
        ConstValue(value)
    }

    /// The value.
    pub fn into_inner(self) -> T {
        self.0
    }

    /// A reference to the value, which can't be modified through it.
    pub fn get(&self) -> &T {
        &self.0
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

/// Whether a C++ constructor ([`ZngCppConstruct`]) is safe ([`IsSafe`]) or not ([`IsUnsafe`]).
pub trait Safety: sealed::Sealed + 'static {}

/// Implemented only by [`IsSafe`], to make the safe methods of [`ZngCppConstruct`] available.
///
/// So they don't compile for `unsafe` constructors:
///
/// ```compile_fail
/// # use core::mem::MaybeUninit;
/// # use zngur_lib::*;
/// # struct Bar(i32);
/// # unsafe impl ZngCppObject for Bar {}
/// # unsafe impl ZngCppStackObject for Bar {}
/// unsafe impl ZngCppConstruct<(i32,)> for Bar {
///     type Safety = IsUnsafe;
///     unsafe fn construct_unchecked(slot: &mut MaybeUninit<Self>, args: (i32,)) {
///         slot.write(Bar(args.0));
///     }
/// }
/// let bar = Bar::build((1,)); // Error: `IsUnsafe: Safe` isn't satisfied.
/// ```
pub trait Safe: Safety {}

/// Marks a C++ constructor declared `safe`.
pub enum IsSafe {}

/// Marks a C++ constructor declared `unsafe`.
pub enum IsUnsafe {}

impl sealed::Sealed for IsSafe {}
impl sealed::Sealed for IsUnsafe {}
impl Safety for IsSafe {}
impl Safety for IsUnsafe {}
impl Safe for IsSafe {}

/// A C++ constructor of `Self` that takes `Args`, a tuple of the arguments: `ZngCppConstruct<()>`
/// is the default constructor, `ZngCppConstruct<(i32,)>` one taking an `i32`, and so on. Like C++
/// constructors, it initializes an object in place.
///
/// The `*_unchecked` methods are always available. The safe ones, [`construct`] and [`build`],
/// only exist for constructors declared `safe` (`Self::Safety` is [`IsSafe`]).
///
/// ```
/// # use core::mem::MaybeUninit;
/// # use zngur_lib::*;
/// # struct Bar(i32);
/// # unsafe impl ZngCppObject for Bar {}
/// # unsafe impl ZngCppStackObject for Bar {}
/// // zngur generates this for `safe constructor(i32);`, with a body that calls C++.
/// unsafe impl ZngCppConstruct<(i32,)> for Bar {
///     type Safety = IsSafe;
///     unsafe fn construct_unchecked(slot: &mut MaybeUninit<Self>, args: (i32,)) {
///         slot.write(Bar(args.0));
///     }
/// }
/// let bar = Bar::build((1,));
/// let mut slot = MaybeUninit::uninit();
/// Bar::construct(&mut slot, (2,));
/// assert_eq!(bar.0 + unsafe { slot.assume_init() }.0, 3);
/// ```
///
/// [`construct`]: ZngCppConstruct::construct
/// [`build`]: ZngCppConstruct::build
///
/// # Safety
///
/// [`construct_unchecked`](ZngCppConstruct::construct_unchecked) must initialize `slot`, and
/// `Safety` may only be [`IsSafe`] if the constructor is safe to call with any `args`.
pub unsafe trait ZngCppConstruct<Args>: ZngCppObject {
    /// [`IsSafe`] for a `safe constructor(...)`, and [`IsUnsafe`] for an `unsafe` one.
    type Safety: Safety;

    /// Runs the C++ constructor on `slot`, which is initialized afterwards.
    ///
    /// # Safety
    ///
    /// The preconditions of the C++ constructor must hold.
    unsafe fn construct_unchecked(slot: &mut core::mem::MaybeUninit<Self>, args: Args);

    /// Constructs an object and returns it. Moving it out of the slot it was constructed in is
    /// fine, because C++ objects that live on the Rust stack are trivially relocatable.
    ///
    /// # Safety
    ///
    /// The preconditions of the C++ constructor must hold.
    unsafe fn build_unchecked(args: Args) -> Self
    where
        Self: ZngCppStackObject,
    {
        let mut slot = core::mem::MaybeUninit::uninit();
        unsafe {
            Self::construct_unchecked(&mut slot, args);
            slot.assume_init()
        }
    }

    /// Runs the C++ constructor on `slot`, which is initialized afterwards.
    fn construct(slot: &mut core::mem::MaybeUninit<Self>, args: Args)
    where
        Self::Safety: Safe,
    {
        // SAFETY: The constructor is safe (`Self::Safety` is `IsSafe`).
        unsafe { Self::construct_unchecked(slot, args) }
    }

    /// Constructs an object and returns it.
    fn build(args: Args) -> Self
    where
        Self: ZngCppStackObject,
        Self::Safety: Safe,
    {
        // SAFETY: The constructor is safe (`Self::Safety` is `IsSafe`).
        unsafe { Self::build_unchecked(args) }
    }
}

mod sealed {
    pub trait Sealed {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn const_value_basics() {
        let c = ConstValue::new(5);
        let d = c;
        assert_eq!(*c.get() + d.into_inner(), 10);
        assert_eq!(
            core::mem::size_of::<ConstValue<f64>>(),
            core::mem::size_of::<f64>()
        );
    }
}
