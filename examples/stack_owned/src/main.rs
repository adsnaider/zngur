#[rustfmt::skip]
mod generated;

use std::mem::MaybeUninit;

use generated::{MyConservativeWrapper, MyCppWrapper};
use zngur_lib::{Const, Cpp, CppCtor};

fn main() {
    let mut c = MyCppWrapper::new(5, 6);
    c.print();

    // C++ functions taking and returning references to C++ objects, which are safe: Rust
    // references in Rust (`Cpp::from_mut` makes a Rust `i32` a C++ object), C++ references in C++.
    let mut sum = 0;
    generated::sum_into(&c, Cpp::from_mut(&mut sum));
    println!("sum = {sum}");
    // A C++-style pointer, which is a raw pointer in Rust (`*mut T`): Rust references coerce to it,
    // but calling the function is `unsafe`.
    unsafe { generated::swap_xy(&mut c) };
    c.print();
    let d = MyCppWrapper::new(1, 2);
    let larger: &MyCppWrapper<Const> = generated::larger_x(&c, &d);
    assert!(std::ptr::eq(larger, c.as_const()));

    // C++ member functions, which are methods: `&self` for `const` member functions and
    // `&mut self` for the others. A C++ `const T&` is a `&T<Const>`, and a `const int32_t&` a
    // `&Cpp<i32, Const>`, whose value `get()` reads. Like in C++, an argument that is a `const`
    // object takes a non-`const` one too: `c.add(&d)`.
    println!("sum() = {}", c.sum());
    c.scale(10);
    c.add(&d);
    c.print();
    let x = c.x_ref().get();
    println!("x_ref() = {x}");
    c.swapped().print();
    MyCppWrapper::diagonal(7).print();

    // C++ constructors, which return the constructor call (a `CppCtor`): `build()` returns the
    // object, and `init()` constructs it in a slot. Calling an `unsafe` constructor is `unsafe`.
    let built = MyCppWrapper::from_xy(3, 4).build();
    built.print();
    let copy = MyCppWrapper::copy(&built).build();
    copy.print();
    let moved = unsafe { MyCppWrapper::shifted(&raw const built, 10) }.build();
    moved.print();
    let mut slot = MaybeUninit::uninit();
    // `MaybeUninit` doesn't run the destructor (which `CppType` doesn't have anyway).
    let constructed: &mut MyCppWrapper = MyCppWrapper::from_xy(8, 9).init(&mut slot);
    constructed.print();
    MyConservativeWrapper::from_values(30, 40).build().print();

    // Free C++ functions.
    let dot = generated::geometry::dot(&built, constructed);
    println!("dot = {dot}");
    generated::geometry::origin().print();
    println!(
        "twice = {} {}",
        generated::geometry::twice(21),
        generated::geometry::twice_f64(1.25)
    );

    // C++ field accessors, which return a reference to the field.
    let mut point = MyCppWrapper::from_xy(1, 2).build();
    *point.x_mut().get_mut() += 100;
    let x = point.x().get();
    let y = point.y().get();
    println!("fields = {x} {y}");
    point.print();

    // Instantiations of C++ class templates.
    let mut ints = generated::Pair::<i32>::new(3, 4).build();
    let doubles = generated::Pair::<f64>::new(1.5, 2.25).build();
    ints.first_mut().set(30);
    println!("pairs = {} {}", ints.sum(), doubles.sum());
    let mut boxed: generated::std::unique_ptr<generated::Pair<i32>> = generated::pairs::boxed(5, 6);
    // `get()` returns the owned object, or `None` if the `unique_ptr` is empty (a null C++
    // pointer), borrowed from the `unique_ptr`.
    let pair = boxed.get().expect("empty std::unique_ptr");
    println!("boxed = {}", pair.sum());
    if let Some(pair) = boxed.get_mut() {
        pair.first_mut().set(50);
    }

    // Fields of pointer type.
    let mut first = generated::Node::new(1).build();
    let mut second = generated::Node::new(2).build();
    first.next_mut().set(&mut second);
    second.type_mut().set(5);
    let next = unsafe { first.next().get().as_mut() }.expect("null");
    let next_type = next.type_mut().get();
    println!(
        "nodes = {} -> {}, type = {}",
        first.id().get(),
        next.id().get(),
        next_type
    );

    assert_eq!(std::mem::size_of::<MyConservativeWrapper>(), 32);
    assert_eq!(std::mem::align_of::<MyConservativeWrapper>(), 16);

    // `const` is part of the type: `MyCppWrapper<Const>` is a `const` object, which only has the
    // `const` member functions. Going from a `T` to a `T<Const>` is explicit, apart from passing
    // it to C++ as a `const` object.
    let squares = generated::views::squares();
    let len = squares.size();
    let third: i32 = unsafe { squares.at(3) }.get();
    println!("squares = {len} items, squares[3] = {third}");
    let _: &generated::View<Cpp<i32, Const>> = &squares;
    let mut point = MyCppWrapper::from_xy(2, 3).build();
    let exclusive: &mut MyCppWrapper<Const> = point.as_const_mut();
    println!("const sum() = {}", exclusive.sum());
    let frozen: MyCppWrapper<Const> = MyCppWrapper::from_xy(4, 5).build().into_const();
    println!("frozen sum() = {}", frozen.sum());

    let mut vec = Vec::new();
    let cons = MyConservativeWrapper::new(10, 20);
    vec.push(cons);
    vec[0].print();

    // Moving a stack-owned value into a C++ member function: C++ destroys it, not Rust.
    let other = MyConservativeWrapper::new(1, 2);
    let total = vec[0].total_with(other);
    println!("total_with() = {total}");
}
