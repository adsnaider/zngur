#[rustfmt::skip]
mod generated;

use std::mem::MaybeUninit;

use generated::{MyConservativeWrapper, MyCppWrapper};
use zngur_lib::{Const, CppPtr, CppRef, ZngCppConstruct, ZngCppObject};

fn main() {
    let mut c = MyCppWrapper::new(5, 6);
    c.print();

    // C++ functions taking and returning C++ references (`CppRef`) and pointers (`CppPtr`).
    // `c.as_ref()` is a `CppRef<Const<MyCppWrapper>>` (`const T&`), and `c.as_mut()` a
    // `CppRef<MyCppWrapper>` (`T&`).
    let mut sum = 0;
    unsafe { generated::sum_into(c.as_ref(), CppPtr::from(CppRef::from(&mut sum))) };
    println!("sum = {sum}");
    unsafe { generated::swap_xy(c.as_mut().as_ptr()) };
    c.print();
    let d = MyCppWrapper::new(1, 2);
    let larger = unsafe { generated::larger_x(c.as_ref(), d.as_ref()) };
    assert_eq!(larger, c.as_ref().as_ptr());

    // C++ member functions, called directly. They're `safe`, since `this` is a `CppRef`.
    println!("sum() = {}", MyCppWrapper::sum(c.as_ref()));
    MyCppWrapper::scale(c.as_mut(), 10);
    MyCppWrapper::add(c.as_mut(), d.as_ref());
    c.print();
    let x = MyCppWrapper::x_ref(c.as_ref()).read();
    println!("x_ref() = {x}");
    MyCppWrapper::swapped(c.as_ref()).print();
    MyCppWrapper::diagonal(7).print();

    // C++ constructors. `build` returns the object, and `construct` constructs it in a slot.
    let built = MyCppWrapper::build((3, 4));
    built.print();
    let copy = MyCppWrapper::build((built.as_ref(),));
    copy.print();
    let moved = unsafe { MyCppWrapper::build_unchecked((built.as_ref().as_ptr(), 10)) };
    moved.print();
    let mut slot = MaybeUninit::uninit();
    MyCppWrapper::construct(&mut slot, (8, 9));
    let constructed = unsafe { slot.assume_init() };
    constructed.print();
    MyConservativeWrapper::build((30, 40)).print();

    // Free C++ functions.
    let dot = generated::geometry::dot(built.as_ref(), constructed.as_ref());
    println!("dot = {dot}");
    generated::geometry::origin().print();
    println!(
        "twice = {} {}",
        generated::geometry::twice(21),
        generated::geometry::twice_f64(1.25)
    );

    // C++ field accessors, which return a `CppRef` to the field.
    let mut point = MyCppWrapper::build((1, 2));
    let x = MyCppWrapper::x_mut(point.as_mut());
    x.write(x.read() + 100);
    let x = MyCppWrapper::x(point.as_ref()).read();
    let y = MyCppWrapper::y(point.as_ref()).read();
    println!("fields = {x} {y}");
    point.print();

    // Instantiations of C++ class templates.
    let mut ints = generated::Pair::<i32>::build((3, 4));
    let doubles = generated::Pair::<f64>::build((1.5, 2.25));
    generated::Pair::<i32>::first_mut(ints.as_mut()).write(30);
    println!(
        "pairs = {} {}",
        generated::Pair::<i32>::sum(ints.as_ref()),
        generated::Pair::<f64>::sum(doubles.as_ref())
    );
    let boxed: generated::std::unique_ptr<generated::Pair<i32>> = generated::pairs::boxed(5, 6);
    // `get()` returns a C++ pointer, which may be null: turning it into a reference is `unsafe`.
    let pair = generated::std::unique_ptr::<generated::Pair<i32>>::get(boxed.as_ref());
    let pair = unsafe { pair.as_cpp_ref() }.expect("null");
    println!("boxed = {}", generated::Pair::<i32>::sum(pair.as_const()));

    // Fields of pointer type.
    let mut first = generated::Node::build((1,));
    let mut second = generated::Node::build((2,));
    generated::Node::next_mut(first.as_mut()).write(second.as_mut().as_ptr());
    generated::Node::type_mut(second.as_mut()).write(5);
    let next = generated::Node::next(first.as_ref()).read();
    let next = unsafe { next.as_cpp_ref() }.expect("null");
    println!(
        "nodes = {} -> {}, type = {}",
        generated::Node::id(first.as_ref()).read(),
        generated::Node::id(next.as_const()).read(),
        generated::Node::type_mut(next).read()
    );

    assert_eq!(std::mem::size_of::<MyConservativeWrapper>(), 32);
    assert_eq!(std::mem::align_of::<MyConservativeWrapper>(), 16);

    // `const` is part of the type: `Const<T>` is `const T`. Only `const` member functions take a
    // `CppRef<Const<T>>`, and going from a `T` to a `Const<T>` is explicit.
    let squares = generated::views::squares();
    let len = generated::View::<Const<i32>>::size(squares.as_ref());
    let third: i32 = unsafe { generated::View::<Const<i32>>::at(squares.as_ref(), 3) }.read();
    println!("squares = {len} items, squares[3] = {third}");
    let mut point = MyCppWrapper::build((2, 3));
    let exclusive: &mut Const<MyCppWrapper> = point.as_const_mut();
    println!(
        "const sum() = {}",
        MyCppWrapper::sum(CppRef::from(exclusive))
    );
    let frozen = Const::new(MyCppWrapper::build((4, 5)));
    println!(
        "frozen sum() = {}",
        MyCppWrapper::sum(CppRef::from(&frozen))
    );

    let mut vec = Vec::new();
    let cons = MyConservativeWrapper::new(10, 20);
    vec.push(cons);
    vec[0].print();

    // Moving a stack-owned value into a C++ member function: C++ destroys it, not Rust.
    let other = MyConservativeWrapper::new(1, 2);
    let total = MyConservativeWrapper::total_with(vec[0].as_ref(), other);
    println!("total_with() = {total}");
}
