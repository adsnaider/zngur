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

/// Whether a [`CppRef`] or [`CppPtr`] refers to a `const` object ([`Const`]) or not ([`Mut`]).
pub trait Constness: sealed::Sealed + 'static {}

/// Marks a [`CppRef`] or [`CppPtr`] to a `const` object, like `const T&` or `const T*` in C++.
pub enum Const {}

/// Marks a [`CppRef`] or [`CppPtr`] to a non-`const` object, like `T&` or `T*` in C++.
pub enum Mut {}

impl sealed::Sealed for Const {}
impl sealed::Sealed for Mut {}
impl Constness for Const {}
impl Constness for Mut {}

/// A C++ reference (`const T&` or `T&`) to a `T`.
///
/// Like a C++ reference, it always refers to a valid object: making one from a Rust reference is
/// safe, and making one from a raw pointer is `unsafe`. So C++ functions that only take references
/// can be declared `safe` in the `.zng` file, and called without `unsafe`.
///
/// It doesn't follow Rust's other rules for references, though: it has no lifetime (the caller
/// keeps the object alive), other C++ references can alias it, and `const` only means that the
/// object can't be modified through this particular reference. So getting a Rust reference to the
/// object from it ([`as_ref`](CppRef::as_ref), [`as_mut`](CppRef::as_mut)) is `unsafe`, but copying
/// a `Copy` object in or out ([`read`](CppRef::read), [`write`](CppRef::write)) isn't.
///
/// A `CppRef<T, Mut>` converts to a `CppRef<T, Const>`, and to a [`CppPtr`], but not the other way
/// around.
#[repr(transparent)]
pub struct CppRef<T, C: Constness = Const> {
    ptr: core::ptr::NonNull<T>,
    _constness: core::marker::PhantomData<C>,
}

/// A C++ pointer (`const T*` or `T*`) to a `T`.
///
/// Like a C++ pointer, it can be null or dangling: creating and copying one is safe, but anything
/// that dereferences it is `unsafe`, including turning it into a [`CppRef`]
/// ([`as_cpp_ref`](CppPtr::as_cpp_ref)).
///
/// A `CppPtr<T, Mut>` converts to a `CppPtr<T, Const>`, but not the other way around.
#[repr(transparent)]
pub struct CppPtr<T, C: Constness = Const> {
    ptr: *mut T,
    _constness: core::marker::PhantomData<C>,
}

macro_rules! impl_common {
    ($ty:ident) => {
        impl<T, C: Constness> Clone for $ty<T, C> {
            fn clone(&self) -> Self {
                *self
            }
        }

        impl<T, C: Constness> Copy for $ty<T, C> {}

        impl<T, C: Constness> PartialEq for $ty<T, C> {
            fn eq(&self, other: &Self) -> bool {
                core::ptr::eq(self.raw(), other.raw())
            }
        }

        impl<T, C: Constness> Eq for $ty<T, C> {}

        impl<T, C: Constness> core::fmt::Debug for $ty<T, C> {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                write!(f, concat!(stringify!($ty), "({:p})"), self.raw())
            }
        }

        impl<T> From<$ty<T, Mut>> for $ty<T, Const> {
            fn from(p: $ty<T, Mut>) -> Self {
                p.as_const()
            }
        }
    };
}

impl_common!(CppRef);
impl_common!(CppPtr);

impl<T, C: Constness> CppRef<T, C> {
    fn raw(&self) -> *mut T {
        self.ptr.as_ptr()
    }

    /// Converts to a reference to `const`.
    pub const fn as_const(self) -> CppRef<T, Const> {
        CppRef {
            ptr: self.ptr,
            _constness: core::marker::PhantomData,
        }
    }

    /// Converts to a pointer.
    pub const fn as_ptr(self) -> CppPtr<T, C> {
        CppPtr {
            ptr: self.ptr.as_ptr(),
            _constness: core::marker::PhantomData,
        }
    }

    /// Returns a shared Rust reference to the object.
    ///
    /// # Safety
    ///
    /// The object must not be mutated (except through `UnsafeCell`s) or destroyed while the
    /// returned reference lives. The caller chooses the lifetime, and must make sure it's correct.
    pub unsafe fn as_ref<'a>(self) -> &'a T {
        unsafe { self.ptr.as_ref() }
    }
}

impl<T: Copy, C: Constness> CppRef<T, C> {
    /// Copies the object out, e.g. the value of a field of builtin type.
    pub fn read(self) -> T {
        // SAFETY: The reference is valid, and `T` is `Copy`.
        unsafe { self.ptr.read() }
    }
}

impl<T: Copy> CppRef<T, Mut> {
    /// Overwrites the object, e.g. the value of a field of builtin type.
    pub fn write(self, value: T) {
        // SAFETY: The reference is valid and refers to a non-`const` object, and `T` is `Copy`, so
        // the old value doesn't need to be dropped.
        unsafe { self.ptr.write(value) }
    }
}

impl<T> CppRef<T, Const> {
    /// A reference to `const` from a raw pointer.
    ///
    /// # Safety
    ///
    /// `ptr` must point to a valid `T`, for as long as the reference is used.
    pub const unsafe fn from_raw(ptr: *const T) -> Self {
        Self {
            ptr: unsafe { core::ptr::NonNull::new_unchecked(ptr as *mut T) },
            _constness: core::marker::PhantomData,
        }
    }

    /// The raw pointer.
    pub const fn as_raw(self) -> *const T {
        self.ptr.as_ptr()
    }
}

impl<T> CppRef<T, Mut> {
    /// A reference to non-`const` from a raw pointer.
    ///
    /// # Safety
    ///
    /// `ptr` must point to a valid `T`, for as long as the reference is used.
    pub const unsafe fn from_raw(ptr: *mut T) -> Self {
        Self {
            ptr: unsafe { core::ptr::NonNull::new_unchecked(ptr) },
            _constness: core::marker::PhantomData,
        }
    }

    /// The raw pointer.
    pub const fn as_raw(self) -> *mut T {
        self.ptr.as_ptr()
    }

    /// Returns a mutable Rust reference to the object.
    ///
    /// # Safety
    ///
    /// Nothing else may access the object while the returned reference lives. The caller chooses
    /// the lifetime, and must make sure it's correct.
    pub unsafe fn as_mut<'a>(self) -> &'a mut T {
        unsafe { &mut *self.ptr.as_ptr() }
    }
}

impl<T> From<&T> for CppRef<T, Const> {
    fn from(r: &T) -> Self {
        // SAFETY: A Rust reference points to a valid object.
        unsafe { Self::from_raw(r) }
    }
}

impl<T> From<&mut T> for CppRef<T, Mut> {
    fn from(r: &mut T) -> Self {
        // SAFETY: A Rust reference points to a valid object.
        unsafe { Self::from_raw(r) }
    }
}

impl<T, C: Constness> From<CppRef<T, C>> for CppPtr<T, C> {
    fn from(r: CppRef<T, C>) -> Self {
        r.as_ptr()
    }
}

impl<T, C: Constness> CppPtr<T, C> {
    fn raw(&self) -> *mut T {
        self.ptr
    }

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

    /// Returns a [`CppRef`] to the object, or `None` if the pointer is null.
    ///
    /// # Safety
    ///
    /// The pointer must be null or point to a valid `T`, for as long as the reference is used.
    pub unsafe fn as_cpp_ref(self) -> Option<CppRef<T, C>> {
        Some(CppRef {
            ptr: core::ptr::NonNull::new(self.ptr)?,
            _constness: core::marker::PhantomData,
        })
    }

    /// Returns a shared Rust reference to the object, or `None` if the pointer is null.
    ///
    /// # Safety
    ///
    /// The pointer must be null or valid, and the object must not be mutated (except through
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

    /// Returns a mutable Rust reference to the object, or `None` if the pointer is null.
    ///
    /// # Safety
    ///
    /// The pointer must be null or valid, and nothing else may access the object while the
    /// returned reference lives. The caller chooses the lifetime, and must make sure it's correct.
    pub unsafe fn as_mut<'a>(self) -> Option<&'a mut T> {
        unsafe { self.ptr.as_mut() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpp_ref_basics() {
        let mut x = 5;
        let r: CppRef<i32, Mut> = (&mut x).into();
        unsafe { *r.as_mut() = 6 };
        let c: CppRef<i32> = r.into();
        assert_eq!(unsafe { *c.as_ref() }, 6);
        r.write(7);
        assert_eq!(c.read(), 7);
        assert_eq!(c, unsafe { CppRef::<i32>::from_raw(&x as *const i32) });
        // Same size and ABI as a pointer, and `Option<CppRef>` too.
        assert_eq!(
            core::mem::size_of::<CppRef<i32, Mut>>(),
            core::mem::size_of::<*mut i32>()
        );
        assert_eq!(
            core::mem::size_of::<Option<CppRef<i32>>>(),
            core::mem::size_of::<*mut i32>()
        );
    }

    #[test]
    fn cpp_ptr_basics() {
        let mut x = 5;
        let r: CppRef<i32, Mut> = (&mut x).into();
        let p: CppPtr<i32, Mut> = r.into();
        assert!(!p.is_null());
        unsafe { *p.as_mut().unwrap() = 6 };
        let c: CppPtr<i32> = p.into();
        assert_eq!(unsafe { c.as_ref() }, Some(&6));
        assert_eq!(c, CppPtr::<i32>::from_raw(&x as *const i32));
        assert_eq!(
            unsafe { c.as_cpp_ref() }.map(|r| r.as_raw()),
            Some(c.as_raw())
        );
        assert!(CppPtr::<i32>::null().is_null());
        assert_eq!(unsafe { CppPtr::<i32>::null().as_ref() }, None);
        assert_eq!(unsafe { CppPtr::<i32>::null().as_cpp_ref() }, None);
        assert_eq!(
            core::mem::size_of::<CppPtr<i32, Mut>>(),
            core::mem::size_of::<*mut i32>()
        );
    }
}
