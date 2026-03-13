
#[allow(dead_code)]
mod zngur_types {
    pub struct ZngurCppOpaqueBorrowedObject(());

    #[repr(C)]
    pub struct ZngurCppOpaqueOwnedObject {
        data: *mut u8,
        destructor: extern "C" fn(*mut u8),
    }

    impl ZngurCppOpaqueOwnedObject {
        pub unsafe fn new(
            data: *mut u8,
            destructor: extern "C" fn(*mut u8),            
        ) -> Self {
            Self { data, destructor }
        }

        pub fn ptr(&self) -> *mut u8 {
            self.data
        }
    }

    impl Drop for ZngurCppOpaqueOwnedObject {
        fn drop(&mut self) {
            (self.destructor)(self.data)
        }
    }
}

#[allow(unused_imports)]
pub use zngur_types::ZngurCppOpaqueOwnedObject;
#[allow(unused_imports)]
pub use zngur_types::ZngurCppOpaqueBorrowedObject;
const _: [(); 24] = [(); ::std::mem::size_of::<::std::vec::Vec::<i32>>()];
const _: [(); 8] = [(); ::std::mem::align_of::<::std::vec::Vec::<i32>>()];

#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn _zngur__std_vec_Vec_i32__drop_in_place_s7s11s15m19y23e24(v: *mut u8) { unsafe {
    ::std::ptr::drop_in_place(v as *mut ::std::vec::Vec::<i32>);
} }

#[allow(non_snake_case)]
#[unsafe(no_mangle)]
#[allow(unused_parens)]
pub extern "C" fn _zngur___std_vec_Vec_i32__new___x7s8s12s16m20y24n25m29y30_e3b0c44298(o: *mut u8) { unsafe {
    ::std::ptr::write(o as *mut ::std::vec::Vec::<i32>,  <::std::vec::Vec::<i32>>::new::<>());
 } }
const _: [(); 8] = [(); ::std::mem::size_of::<crate::MainType>()];
const _: [(); 8] = [(); ::std::mem::align_of::<crate::MainType>()];

#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn _zngur_crate_MainType_drop_in_place_s12e21(v: *mut u8) { unsafe {
    ::std::ptr::drop_in_place(v as *mut crate::MainType);
} }

#[allow(non_snake_case)]
#[unsafe(no_mangle)]
#[allow(unused_parens)]
pub extern "C" fn _zngur__crate_MainType_do_something___x7s13n22m35y36_ae7798fde0(i0: *mut u8, i1: *mut u8, o: *mut u8) { unsafe {
    ::std::ptr::write(o as *mut (),  <crate::MainType>::do_something::<>((::std::ptr::read(i0 as *mut &crate::MainType)), (::std::ptr::read(i1 as *mut ::std::option::Option::<&i32>)), ));
 } }
