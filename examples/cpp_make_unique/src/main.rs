#[rustfmt::skip]
mod generated;

use generated::shapes::{Canvas, Point};
use generated::std::{make_canvas, make_point};
use zngur_lib::ZngCppConstruct;

fn main() {
    // A type that Rust can hold by value, constructed on the heap instead.
    let boxed_point = make_point(3, 4);
    // SAFETY: `boxed_point` owns a `Point`, which lives as long as it does.
    let point = unsafe { boxed_point.get().as_ref() }.expect("null");
    println!("point = ({}, {})", point.x(), point.y());

    // A type that Rust can only refer to: `std::make_unique` is how Rust constructs one.
    let canvas = make_canvas(640, 480);
    // SAFETY: `canvas` owns a `Canvas`, which lives as long as it does.
    let canvas_ref: &mut Canvas = unsafe { canvas.get().as_mut() }.expect("null");
    canvas_ref.Draw(point.as_const());
    canvas_ref.Draw(Point::build((5, 6)).as_const());
    println!("points drawn = {}", canvas_ref.Points());

    // Dropping the `std::unique_ptr` destroys the canvas.
    drop(canvas);
    println!("done");
}
