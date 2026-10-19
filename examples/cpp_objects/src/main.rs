#[rustfmt::skip]
mod generated;

use generated::objects::{Counter, Point};
use zngur_lib::{Const, Cpp, ZngCppConstruct};

fn main() {
    let mut first = Counter::build((1,));
    let mut second = Counter::build((10,));

    // A pointer field, set through a shared reference: `next_var()` is a `&Cpp<*mut Counter>`.
    first.next_var().set(&mut second);

    // A non-`const` member function, called through shared references.
    let (a, b) = (&first, &first);
    a.bump();
    b.bump();
    println!("counts = {} {}", first.count().get(), second.count().get());

    // A C++ `int32_t` is a `Cpp<i32>`, which can be set through a shared reference.
    let count: &Cpp<i32> = first.count_var();
    count.set(count.get() + 5);
    first.count_ref().set(first.count_ref().get() * 2);
    println!("count = {}", first.count().get());

    // A `const` member function and field accessor, through a unique reference to a `const`
    // object.
    let frozen: &mut Counter<Const> = first.as_const_mut();
    println!("total = {}", frozen.total());
    let step: &mut Cpp<i32, Const> = frozen.step_unique_const();
    println!("step = {}", step.get());

    // Copies of fields: `const` ones are `T<Const>` / `Cpp<i32, Const>`.
    let copy: i32 = first.count_copy();
    let const_copy: Cpp<i32, Const> = first.count_const_copy();
    let origin: Point = first.origin_copy();
    let origin_const: Point<Const> = first.origin_const_copy();
    println!(
        "copies = {copy} {} ({}, {}) ({}, {})",
        const_copy.get(),
        origin.x(),
        origin.y(),
        origin_const.x(),
        origin_const.y()
    );
    let next: *mut Counter = first.next_copy();
    assert_eq!(next, &raw mut second);

    // `Cpp::from_mut` makes a Rust `i32` a C++ object, for a `&'a unique var i32`...
    let mut sum = 100;
    first.add_to(Cpp::from_mut(&mut sum));
    println!("sum = {sum}");
    // ...while a `&'a i32` is a Rust value, which C++ gets as a `const int32_t&`.
    first.add(&6);
    println!("count = {}", first.count().get());

    // Free functions taking references to C++ objects.
    let shared = &second;
    generated::objects::reset(shared);
    generated::objects::reset(&second);
    generated::objects::double_step(&mut second);
    second.bump();
    println!("peek = {}", generated::objects::peek(second.as_const_mut()));
}
