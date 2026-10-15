/// Marker trait for C++ objects
///
/// # Safety
///
/// Only generated C++ objects by zngur should implement this trait.
pub unsafe trait ZngCppObject: Sized {}
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

/// Whether a [`CppPtr`] is a pointer (or reference) to `const` ([`Const`]) or not ([`Mut`]).
pub trait Constness: sealed::Sealed + 'static {}

/// Marks a [`CppPtr`] to a `const` object, like `const T*` or `const T&` in C++.
pub enum Const {}

/// Marks a [`CppPtr`] to a non-`const` object, like `T*` or `T&` in C++.
pub enum Mut {}

impl sealed::Sealed for Const {}
impl sealed::Sealed for Mut {}
impl Constness for Const {}
impl Constness for Mut {}

/// A C++ pointer or reference (`const T*`, `T*`, `const T&` or `T&`) to a `T`.
///
/// C++ pointers and references don't follow Rust's rules: they can be null or dangling, other C++
/// pointers can alias them, and `const` only means that the object can't be modified through this
/// particular pointer. So unlike Rust references, a `CppPtr` makes no promises, and has no
/// lifetime. Creating and copying one is safe, but anything that dereferences it (including
/// calling C++ functions through it) is `unsafe`.
///
/// A `CppPtr<T, Mut>` converts to a `CppPtr<T, Const>`, but not the other way around.
#[repr(transparent)]
pub struct CppPtr<T, C: Constness = Const> {
    ptr: *mut T,
    _constness: core::marker::PhantomData<C>,
}

impl<T, C: Constness> Clone for CppPtr<T, C> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T, C: Constness> Copy for CppPtr<T, C> {}

impl<T, C: Constness> PartialEq for CppPtr<T, C> {
    fn eq(&self, other: &Self) -> bool {
        core::ptr::eq(self.ptr, other.ptr)
    }
}

impl<T, C: Constness> Eq for CppPtr<T, C> {}

impl<T, C: Constness> core::fmt::Debug for CppPtr<T, C> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "CppPtr({:p})", self.ptr)
    }
}

impl<T, C: Constness> CppPtr<T, C> {
    /// A null pointer.
    pub const fn null() -> Self {
        Self {
            ptr: core::ptr::null_mut(),
            _constness: core::marker::PhantomData,
        }
    }

    /// Whether the pointer is null.
    pub fn is_null(self) -> bool {
        self.ptr.is_null()
    }

    /// Converts to a pointer to `const`.
    pub const fn as_const(self) -> CppPtr<T, Const> {
        CppPtr {
            ptr: self.ptr,
            _constness: core::marker::PhantomData,
        }
    }

    /// Returns a shared reference to the object, or `None` if the pointer is null.
    ///
    /// # Safety
    ///
    /// The pointer must be valid, and the object must not be mutated (except through
    /// `UnsafeCell`s) or destroyed while the returned reference lives. The caller chooses the
    /// lifetime, and must make sure it's correct.
    pub unsafe fn as_ref<'a>(self) -> Option<&'a T> {
        unsafe { self.ptr.as_ref() }
    }
}

impl<T> CppPtr<T, Const> {
    /// A pointer to `const` from a raw pointer.
    pub const fn from_raw(ptr: *const T) -> Self {
        Self {
            ptr: ptr as *mut T,
            _constness: core::marker::PhantomData,
        }
    }

    /// The raw pointer.
    pub const fn as_raw(self) -> *const T {
        self.ptr
    }
}

impl<T> CppPtr<T, Mut> {
    /// A pointer to non-`const` from a raw pointer.
    pub const fn from_raw(ptr: *mut T) -> Self {
        Self {
            ptr,
            _constness: core::marker::PhantomData,
        }
    }

    /// The raw pointer.
    pub const fn as_raw(self) -> *mut T {
        self.ptr
    }

    /// Returns a mutable reference to the object, or `None` if the pointer is null.
    ///
    /// # Safety
    ///
    /// The pointer must be valid, and nothing else may access the object while the returned
    /// reference lives. The caller chooses the lifetime, and must make sure it's correct.
    pub unsafe fn as_mut<'a>(self) -> Option<&'a mut T> {
        unsafe { self.ptr.as_mut() }
    }
}

impl<T> From<CppPtr<T, Mut>> for CppPtr<T, Const> {
    fn from(ptr: CppPtr<T, Mut>) -> Self {
        ptr.as_const()
    }
}

impl<T> From<&T> for CppPtr<T, Const> {
    fn from(r: &T) -> Self {
        Self::from_raw(r)
    }
}

impl<T> From<&mut T> for CppPtr<T, Mut> {
    fn from(r: &mut T) -> Self {
        Self::from_raw(r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpp_ptr_basics() {
        let mut x = 5;
        let p: CppPtr<i32, Mut> = (&mut x).into();
        assert!(!p.is_null());
        unsafe { *p.as_mut().unwrap() = 6 };
        let c: CppPtr<i32> = p.into();
        assert_eq!(unsafe { c.as_ref() }, Some(&6));
        assert_eq!(c, CppPtr::<i32>::from_raw(&x as *const i32));
        assert!(CppPtr::<i32>::null().is_null());
        assert_eq!(unsafe { CppPtr::<i32>::null().as_ref() }, None);
        // Same size and ABI as a pointer.
        assert_eq!(
            core::mem::size_of::<CppPtr<i32, Mut>>(),
            core::mem::size_of::<*mut i32>()
        );
    }
}
