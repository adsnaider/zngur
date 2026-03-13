pub struct MainType(pub i64);

impl MainType {
    pub fn do_something(&self, _v: std::option::Option<&i32>) {
        println!("do_something called!");
    }
}

#[rustfmt::skip]
mod main_generated;
#[rustfmt::skip]
mod module_generated;
