
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
const _: [(); 8] = [(); ::std::mem::size_of::<::std::option::Option::<&i32>>()];
const _: [(); 8] = [(); ::std::mem::align_of::<::std::option::Option::<&i32>>()];

#[allow(non_snake_case)]
#[unsafe(no_mangle)]
pub extern "C" fn _zngur__std_option_Option__i32__drop_in_place_s7s11s18m25r26y30e31(v: *mut u8) { unsafe {
    ::std::ptr::drop_in_place(v as *mut ::std::option::Option::<&i32>);
} }

#[allow(non_snake_case)]
#[unsafe(no_mangle)]
#[allow(unused_parens)]
pub extern "C" fn _zngur___std_option_Option__i32__unwrap___x7s8s12s19m26r27y31n32m39y40_4b7fe64936(i0: *mut u8, o: *mut u8) { unsafe {
    ::std::ptr::write(o as *mut &i32,  <::std::option::Option::<&i32>>::unwrap::<>((::std::ptr::read(i0 as *mut ::std::option::Option::<&i32>)), ));
 } }
