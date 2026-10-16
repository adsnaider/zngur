#[rustfmt::skip]
mod generated;

use generated::shapes::{Canvas, Point};
use generated::std::{make_canvas, make_point, unique_ptr};
use zngur_lib::ZngCppConstruct;

fn main() {
    // A type that Rust can hold by value, constructed on the heap instead.
    let boxed_point = make_point(3, 4);
    // SAFETY: `boxed_point` owns a `Point`, which lives as long as it does.
    let point =
        unsafe { unique_ptr::<Point>::get(boxed_point.as_ref()).as_cpp_ref() }.expect("null");
    println!(
        "point = ({}, {})",
        Point::x(point.as_const()).read(),
        Point::y(point.as_const()).read()
    );

    // A type that Rust can only refer to: `std::make_unique` is how Rust constructs one.
    let canvas = make_canvas(640, 480);
    // SAFETY: `canvas` owns a `Canvas`, which lives as long as it does.
    let canvas_ref =
        unsafe { unique_ptr::<Canvas>::get(canvas.as_ref()).as_cpp_ref() }.expect("null");
    Canvas::Draw(canvas_ref, point.as_const());
    Canvas::Draw(canvas_ref, Point::build((5, 6)).as_ref());
    println!("points drawn = {}", Canvas::Points(canvas_ref.as_const()));

    // Dropping the `std::unique_ptr` destroys the canvas.
    drop(canvas);
    println!("done");
}
