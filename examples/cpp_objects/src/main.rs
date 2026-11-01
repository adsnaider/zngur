#[rustfmt::skip]
mod generated;

use generated::objects::{Counter, Flag, Point};
use zngur_lib::{Const, Cpp, CppCtor};

fn main() {
    let mut first = Counter::new(1).build();
    let mut second = Counter::new(10).build();

    // A pointer field, set through a shared reference: `next_var()` is a `&Cpp<*mut Counter>`.
    unsafe { first.next_var() }.set(&mut second);

    // `bool` and `const bool*` fields work without an explicit `type bool` block.
    let flag = Flag::new().build();
    assert!(!flag.active_copy());
    flag.active().set(true);
    assert!(flag.active_copy());
    unsafe { flag.watched_var() }.set(flag.active().as_const());
    assert_eq!(
        unsafe { (*flag.watched().get()).get() },
        flag.active().get()
    );

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

    // A `const` member function through a unique reference to a `const` object, and a `&unique
    // const` field accessor on a non-`const` object.
    let frozen: &mut Counter<Const> = first.as_const_mut();
    println!("total = {}", frozen.total());
    let step: &mut Cpp<i32, Const> = first.step_unique_const();
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
    // `unique` (`&mut second`) requires no live pointer aliases while active, so clear
    // `first.next` first and store a fresh pointer derived from `&mut second` afterwards.
    unsafe { first.next_var() }.set(std::ptr::null_mut());
    generated::objects::double_step(&mut second);
    second.bump();
    println!("peek = {}", generated::objects::peek(&mut second));
    unsafe { first.next_var() }.set(&mut second);

    // Nullable references (C++ pointers that may be null): `Some(&c)` or `None`, a `Counter*` in
    // C++. `first.next` points to `second`.
    let fallback = -1;
    println!(
        "count_or = {} {}",
        generated::objects::count_or(Some(&first), fallback),
        // `const` inputs take both constnesses, so a bare `None` needs a type.
        generated::objects::count_or(None::<&Counter>, fallback)
    );
    if let Some(next) = generated::objects::next_of(&first) {
        println!("next count = {}", next.count().get());
    }
    assert!(generated::objects::next_of(&second).is_none());
    // C++ calls `bump_chain` (below) with the pointer.
    println!(
        "bumped in Rust = {} {}",
        generated::objects::bump_in_rust(Some(&second)),
        generated::objects::bump_in_rust(None)
    );
}

/// Called from C++, with a nullable reference to a C++ object (a `Counter*` in C++).
pub fn bump_chain(counter: Option<&Counter>) -> i32 {
    match counter {
        Some(counter) => {
            counter.bump();
            counter.count().get()
        }
        None => -1,
    }
}
