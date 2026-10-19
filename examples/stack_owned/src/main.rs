#[rustfmt::skip]
mod generated;

use std::mem::MaybeUninit;

use generated::{MyConservativeWrapper, MyCppWrapper};
use zngur_lib::{Const, Cpp, ZngCppConstruct};

fn main() {
    let mut c = MyCppWrapper::new(5, 6);
    c.print();

    // C++ functions taking and returning C++-style pointers and references, which are raw
    // pointers in Rust: `*const MyCppWrapper<Const>` for `const T&` / `const T*`, `*mut T` for
    // `T&` / `T*`, and `*mut Cpp<i32>` for `int32_t*`. Rust references coerce to them, and
    // `Cpp::from_mut` makes a Rust `i32` a C++ object.
    let mut sum = 0;
    unsafe { generated::sum_into(c.as_const(), Cpp::from_mut(&mut sum)) };
    println!("sum = {sum}");
    unsafe { generated::swap_xy(&mut c) };
    c.print();
    let d = MyCppWrapper::new(1, 2);
    let larger = unsafe { generated::larger_x(c.as_const(), d.as_const()) };
    assert_eq!(larger, c.as_const() as *const _);

    // C++ member functions, which are methods: `&self` for `const` member functions and
    // `&mut self` for the others. A C++ `const T&` is a `&T<Const>`, and a `const int32_t&` a
    // `&Cpp<i32, Const>`, whose value `get()` reads.
    println!("sum() = {}", c.sum());
    c.scale(10);
    c.add(d.as_const());
    c.print();
    let x = c.x_ref().get();
    println!("x_ref() = {x}");
    c.swapped().print();
    MyCppWrapper::diagonal(7).print();

    // C++ constructors. `build` returns the object, and `construct` constructs it in a slot.
    let built = MyCppWrapper::build((3, 4));
    built.print();
    let copy = MyCppWrapper::build((built.as_const(),));
    copy.print();
    let moved = unsafe { MyCppWrapper::build_unchecked((built.as_const() as *const _, 10)) };
    moved.print();
    let mut slot = MaybeUninit::uninit();
    MyCppWrapper::construct(&mut slot, (8, 9));
    let constructed = unsafe { slot.assume_init() };
    constructed.print();
    MyConservativeWrapper::build((30, 40)).print();

    // Free C++ functions.
    let dot = generated::geometry::dot(built.as_const(), constructed.as_const());
    println!("dot = {dot}");
    generated::geometry::origin().print();
    println!(
        "twice = {} {}",
        generated::geometry::twice(21),
        generated::geometry::twice_f64(1.25)
    );

    // C++ field accessors, which return a reference to the field.
    let mut point = MyCppWrapper::build((1, 2));
    *point.x_mut().get_mut() += 100;
    let x = point.x().get();
    let y = point.y().get();
    println!("fields = {x} {y}");
    point.print();

    // Instantiations of C++ class templates.
    let mut ints = generated::Pair::<i32>::build((3, 4));
    let doubles = generated::Pair::<f64>::build((1.5, 2.25));
    ints.first_mut().set(30);
    println!("pairs = {} {}", ints.sum(), doubles.sum());
    let boxed: generated::std::unique_ptr<generated::Pair<i32>> = generated::pairs::boxed(5, 6);
    // `get()` returns a C++ pointer, which may be null: dereferencing it is `unsafe`.
    let pair = unsafe { boxed.get().as_ref() }.expect("null");
    println!("boxed = {}", pair.sum());

    // Fields of pointer type.
    let mut first = generated::Node::build((1,));
    let mut second = generated::Node::build((2,));
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
    // `const` member functions. Going from a `T` to a `T<Const>` is explicit.
    let squares = generated::views::squares();
    let len = squares.size();
    let third: i32 = unsafe { squares.at(3) }.get();
    println!("squares = {len} items, squares[3] = {third}");
    let _: &generated::View<Cpp<i32, Const>> = &squares;
    let mut point = MyCppWrapper::build((2, 3));
    let exclusive: &mut MyCppWrapper<Const> = point.as_const_mut();
    println!("const sum() = {}", exclusive.sum());
    let frozen: MyCppWrapper<Const> = MyCppWrapper::build((4, 5)).into_const();
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
