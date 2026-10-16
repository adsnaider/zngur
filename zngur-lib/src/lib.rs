/// Marker trait for C++ objects: the Rust wrappers of `c++::` types, and the builtins that are C++
/// types too (`i32`, `f64`, `bool`, ...). These are the types that can be `const` ([`Const`]).
///
/// # Safety
///
/// Only generated C++ objects by zngur (and the builtins in this crate) should implement this
/// trait.
pub unsafe trait ZngCppObject: Sized {
    /// The object, as a `const` object.
    fn as_const(&self) -> &Const<Self> {
        // SAFETY: `Const<Self>` is `#[repr(transparent)]` around `Self`.
        unsafe { &*(self as *const Self as *const Const<Self>) }
    }

    /// The object, as a `const` object that nothing else can access while the reference lives.
    fn as_const_mut(&mut self) -> &mut Const<Self> {
        // SAFETY: `Const<Self>` is `#[repr(transparent)]` around `Self`.
        unsafe { &mut *(self as *mut Self as *mut Const<Self>) }
    }
}

macro_rules! impl_cpp_object_for_builtins {
    ($($ty:ty),*) => {
        $(unsafe impl ZngCppObject for $ty {})*
    };
}

impl_cpp_object_for_builtins!(i8, i16, i32, i64, u8, u16, u32, u64, usize, f32, f64, bool);

/// A `const` C++ object: `Const<T>` is `const T` in C++, e.g. `CppRef<Const<T>>` is `const T&`,
/// and `c++::std::span<const i32>` is `std::span<Const<i32>>`.
///
/// It has the same layout as `T`, but only the `const` member functions of a C++ type are
/// available on it (and on `CppRef<Const<T>>`). There's no `Deref` to `T`, which would make the
/// others available too. Going the other way, from a `T` to a `const T`, is explicit:
/// [`ZngCppObject::as_const`] or `CppRef::as_const`.
///
/// ```compile_fail
/// # use zngur_lib::*;
/// let x = 5;
/// let r: CppRef<Const<i32>> = CppRef::from(&x);
/// r.write(6); // Error: a reference to `const` can't be written through.
/// ```
#[repr(transparent)]
pub struct Const<T: ZngCppObject>(T);

impl<T: ZngCppObject> Const<T> {
    /// Makes `value` `const`.
    pub const fn new(value: T) -> Self {
        Const(value)
    }
}

impl<T: ZngCppObject + Clone> Clone for Const<T> {
    fn clone(&self) -> Self {
        Const(self.0.clone())
    }
}

impl<T: ZngCppObject + Copy> Copy for Const<T> {}

impl<T: ZngCppObject + core::fmt::Debug> core::fmt::Debug for Const<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Const({:?})", self.0)
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

/// A C++ reference to a `T`: `CppRef<T>` is `T&`, and `CppRef<Const<T>>` is `const T&`.
///
/// Like a C++ reference, it always refers to a valid object: making one from a Rust reference is
/// safe, and making one from a raw pointer is `unsafe`. So C++ functions that only take references
/// can be declared `safe` in the `.zng` file, and called without `unsafe`.
///
/// It doesn't follow Rust's other rules for references, though: it has no lifetime (the caller
/// keeps the object alive), other C++ references can alias it, and `const` only means that the
/// object can't be modified through this particular reference. So getting a Rust reference to the
/// object from it ([`as_ref`](CppRef::as_ref), [`as_mut`](CppRef::as_mut)) is `unsafe`, but copying
/// a builtin in or out ([`read`](CppRef::read), [`write`](CppRef::write)) isn't.
///
/// A `CppRef<T>` converts to a `CppRef<Const<T>>` ([`as_const`](CppRef::as_const)), and to a
/// [`CppPtr`], but not the other way around.
#[repr(transparent)]
pub struct CppRef<T> {
    ptr: core::ptr::NonNull<T>,
}

/// A C++ pointer to a `T`: `CppPtr<T>` is `T*`, and `CppPtr<Const<T>>` is `const T*`.
///
/// Like a C++ pointer, it can be null or dangling: creating and copying one is safe, but anything
/// that dereferences it is `unsafe`, including turning it into a [`CppRef`]
/// ([`as_cpp_ref`](CppPtr::as_cpp_ref)).
///
/// A `CppPtr<T>` converts to a `CppPtr<Const<T>>`, but not the other way around.
#[repr(transparent)]
pub struct CppPtr<T> {
    ptr: *mut T,
}

macro_rules! impl_common {
    ($ty:ident) => {
        impl<T> Clone for $ty<T> {
            fn clone(&self) -> Self {
                *self
            }
        }

        impl<T> Copy for $ty<T> {}

        impl<T> PartialEq for $ty<T> {
            fn eq(&self, other: &Self) -> bool {
                core::ptr::eq(self.raw(), other.raw())
            }
        }

        impl<T> Eq for $ty<T> {}

        impl<T> core::fmt::Debug for $ty<T> {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                write!(f, concat!(stringify!($ty), "({:p})"), self.raw())
            }
        }

        impl<T: ZngCppObject> From<$ty<T>> for $ty<Const<T>> {
            fn from(p: $ty<T>) -> Self {
                p.as_const()
            }
        }
    };
}

impl_common!(CppRef);
impl_common!(CppPtr);

/// The types that [`CppRef::read`] can copy out of a C++ object: builtins (`i32`, ...), C++
/// pointers ([`CppPtr`]), and `const` ones, whose value is the non-`const` one.
///
/// # Safety
///
/// `Self` must have the same layout as `Value`, and copying it must be fine.
pub unsafe trait CppCopy {
    /// The value that is copied out.
    type Value: Copy;
}

unsafe impl<T: ZngCppObject + Copy> CppCopy for T {
    type Value = T;
}

unsafe impl<T: ZngCppObject + Copy> CppCopy for Const<T> {
    type Value = T;
}

// C++ pointers are C++ objects too, which can be `const` (`T* const`, e.g. a `const` field of
// pointer type).
unsafe impl<T> ZngCppObject for CppPtr<T> {}

impl<T> CppRef<T> {
    fn raw(&self) -> *mut T {
        self.ptr.as_ptr()
    }

    /// A reference from a raw pointer.
    ///
    /// # Safety
    ///
    /// `ptr` must point to a valid `T`, for as long as the reference is used.
    pub const unsafe fn from_raw(ptr: *mut T) -> Self {
        Self {
            ptr: unsafe { core::ptr::NonNull::new_unchecked(ptr) },
        }
    }

    /// The raw pointer.
    pub const fn as_raw(self) -> *mut T {
        self.ptr.as_ptr()
    }

    /// Converts to a pointer.
    pub const fn as_ptr(self) -> CppPtr<T> {
        CppPtr {
            ptr: self.ptr.as_ptr(),
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

    /// Returns a mutable Rust reference to the object. For a `CppRef<Const<T>>`, that's a
    /// `&mut Const<T>`, which still only gives access to the `const` member functions.
    ///
    /// # Safety
    ///
    /// Nothing else may access the object while the returned reference lives. The caller chooses
    /// the lifetime, and must make sure it's correct.
    pub unsafe fn as_mut<'a>(self) -> &'a mut T {
        unsafe { &mut *self.ptr.as_ptr() }
    }
}

impl<T: ZngCppObject> CppRef<T> {
    /// Converts to a reference to `const`.
    pub const fn as_const(self) -> CppRef<Const<T>> {
        CppRef {
            ptr: self.ptr.cast(),
        }
    }
}

impl<T: ZngCppObject> CppRef<Const<T>> {
    /// A reference to `const` from a raw pointer.
    ///
    /// # Safety
    ///
    /// `ptr` must point to a valid `T`, for as long as the reference is used.
    pub const unsafe fn from_const_raw(ptr: *const T) -> Self {
        unsafe { Self::from_raw(ptr as *mut Const<T>) }
    }

    /// The raw pointer, to `T`.
    pub const fn as_const_raw(self) -> *const T {
        self.ptr.as_ptr() as *const T
    }
}

impl<T: CppCopy> CppRef<T> {
    /// Copies the object out, e.g. the value of a field of builtin type.
    pub fn read(self) -> T::Value {
        // SAFETY: The reference is valid, and `T` is a `Value` that can be copied.
        unsafe { self.ptr.cast::<T::Value>().read() }
    }
}

impl<T: CppCopy<Value = T>> CppRef<T> {
    /// Overwrites the object, e.g. the value of a field of builtin type. References to `const`
    /// don't have it.
    pub fn write(self, value: T) {
        // SAFETY: The reference is valid and refers to a non-`const` object, and `T` is `Copy`, so
        // the old value doesn't need to be dropped.
        unsafe { self.ptr.write(value) }
    }
}

impl<T: ZngCppObject> From<&T> for CppRef<Const<T>> {
    fn from(r: &T) -> Self {
        CppRef {
            ptr: core::ptr::NonNull::from(r).cast(),
        }
    }
}

impl<T: ZngCppObject> From<&Const<T>> for CppRef<Const<T>> {
    fn from(r: &Const<T>) -> Self {
        CppRef {
            ptr: core::ptr::NonNull::from(r),
        }
    }
}

/// Also makes a `CppRef<Const<T>>` from a `&mut Const<T>`.
impl<T> From<&mut T> for CppRef<T> {
    fn from(r: &mut T) -> Self {
        CppRef {
            ptr: core::ptr::NonNull::from(r),
        }
    }
}

impl<T> From<CppRef<T>> for CppPtr<T> {
    fn from(r: CppRef<T>) -> Self {
        r.as_ptr()
    }
}

impl<T> CppPtr<T> {
    fn raw(&self) -> *mut T {
        self.ptr
    }

    /// A null pointer.
    pub const fn null() -> Self {
        Self {
            ptr: core::ptr::null_mut(),
        }
    }

    /// Whether the pointer is null.
    pub fn is_null(self) -> bool {
        self.ptr.is_null()
    }

    /// A pointer from a raw pointer.
    pub const fn from_raw(ptr: *mut T) -> Self {
        Self { ptr }
    }

    /// The raw pointer.
    pub const fn as_raw(self) -> *mut T {
        self.ptr
    }

    /// Returns a [`CppRef`] to the object, or `None` if the pointer is null.
    ///
    /// # Safety
    ///
    /// The pointer must be null or point to a valid `T`, for as long as the reference is used.
    pub unsafe fn as_cpp_ref(self) -> Option<CppRef<T>> {
        Some(CppRef {
            ptr: core::ptr::NonNull::new(self.ptr)?,
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

impl<T: ZngCppObject> CppPtr<T> {
    /// Converts to a pointer to `const`.
    pub const fn as_const(self) -> CppPtr<Const<T>> {
        CppPtr {
            ptr: self.ptr as *mut Const<T>,
        }
    }
}

impl<T: ZngCppObject> CppPtr<Const<T>> {
    /// A pointer to `const` from a raw pointer.
    pub const fn from_const_raw(ptr: *const T) -> Self {
        Self {
            ptr: ptr as *mut Const<T>,
        }
    }

    /// The raw pointer, to `T`.
    pub const fn as_const_raw(self) -> *const T {
        self.ptr as *const T
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn const_basics() {
        let mut x = 5;
        assert_eq!(
            x.as_const() as *const Const<i32> as *const i32,
            &x as *const i32
        );
        let c: &mut Const<i32> = x.as_const_mut();
        let r: CppRef<Const<i32>> = c.into();
        assert_eq!(r.read(), 5);
        let copy = Const::new(7);
        let copy2 = copy;
        assert_eq!(CppRef::from(&copy).read() + CppRef::from(&copy2).read(), 14);
        assert_eq!(
            core::mem::size_of::<Const<f64>>(),
            core::mem::size_of::<f64>()
        );
    }

    #[test]
    fn cpp_ref_basics() {
        let mut x = 5;
        let r: CppRef<i32> = (&mut x).into();
        unsafe { *r.as_mut() = 6 };
        let c: CppRef<Const<i32>> = r.into();
        assert_eq!(c.read(), 6);
        r.write(7);
        assert_eq!(c.read(), 7);
        assert_eq!(c, r.as_const());
        assert_eq!(c, unsafe { CppRef::from_const_raw(&x as *const i32) });
        assert_eq!(
            CppRef::<Const<i32>>::from(&x).as_const_raw(),
            &x as *const i32
        );
        // Same size and ABI as a pointer, and `Option<CppRef>` too.
        assert_eq!(
            core::mem::size_of::<CppRef<i32>>(),
            core::mem::size_of::<*mut i32>()
        );
        assert_eq!(
            core::mem::size_of::<Option<CppRef<Const<i32>>>>(),
            core::mem::size_of::<*mut i32>()
        );
    }

    #[test]
    fn cpp_ptr_basics() {
        let mut x = 5;
        let r: CppRef<i32> = (&mut x).into();
        let p: CppPtr<i32> = r.into();
        assert!(!p.is_null());
        unsafe { *p.as_mut().unwrap() = 6 };
        let c: CppPtr<Const<i32>> = p.into();
        assert_eq!(unsafe { c.as_cpp_ref() }.map(|r| r.read()), Some(6));
        assert_eq!(c, CppPtr::from_const_raw(&x as *const i32));
        assert_eq!(
            unsafe { c.as_cpp_ref() }.map(|r| r.as_raw()),
            Some(c.as_raw())
        );
        assert!(CppPtr::<i32>::null().is_null());
        assert_eq!(unsafe { CppPtr::<i32>::null().as_ref() }, None);
        assert_eq!(unsafe { CppPtr::<i32>::null().as_cpp_ref() }, None);
        // Pointers to pointers: a field of pointer type.
        let mut ptr = CppPtr::<i32>::null();
        let field: CppRef<CppPtr<i32>> = (&mut ptr).into();
        field.write(p);
        assert_eq!(field.read(), p);
        assert_eq!(
            core::mem::size_of::<CppPtr<i32>>(),
            core::mem::size_of::<*mut i32>()
        );
    }
}
