#[rustfmt::skip]
mod generated;

use generated::shapes::Point;
use generated::std::{make_canvas, make_point};
use zngur_lib::CppCtor;

fn main() {
    // A type that Rust can hold by value, constructed on the heap instead. `get_mut()` borrows the
    // object uniquely and `get()` shares it, or they're `None` if the `unique_ptr` is empty.
    let mut boxed_point = make_point(3, 4);
    let point: &mut Point = boxed_point.get_mut().expect("empty std::unique_ptr");
    println!("point = ({}, {})", point.x().get(), point.y().get());
    let point = boxed_point.get().expect("empty std::unique_ptr");

    // A type that Rust can only refer to: `std::make_unique` is how Rust constructs one.
    let mut canvas = make_canvas(640, 480);
    if let Some(canvas) = canvas.get_mut() {
        canvas.Draw(point);
        canvas.Draw(&Point::new(5, 6).build());
    }
    println!("points drawn = {}", canvas.get().map_or(0, |c| c.Points()));

    // Dropping the `std::unique_ptr` destroys the canvas.
    drop(canvas);
    println!("done");
}
