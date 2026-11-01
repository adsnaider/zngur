#[rustfmt::skip]
mod generated;

pub struct Parser<'a> {
    input: &'a str,
    pos: usize,
}

#[repr(C)]
pub struct Pair<'a, 'b> {
    pub first: &'a str,
    pub second: &'b str,
}

impl<'a> Parser<'a> {
    fn new(input: &'a str) -> Self {
        Parser { input, pos: 0 }
    }

    fn rest(&self) -> &'a str {
        &self.input[self.pos..]
    }

    fn advance(&mut self, n: usize) {
        self.pos = (self.pos + n).min(self.input.len());
    }
}

pub trait Chooser {
    fn choose<'a>(&self, a: &'a str, b: &'a str) -> &'a str;
}

fn longest<'a>(a: &'a str, b: &'a str) -> &'a str {
    if a.len() >= b.len() { a } else { b }
}

fn identity<T>(x: T) -> T {
    x
}

fn print_pair<'a, 'b>(pair: Pair<'a, 'b>) {
    println!("Rust: pair = ({}, {})", pair.first, pair.second);
}

fn run_demo() {
    let hello = String::from("hello");
    let greetings = String::from("greetings");
    // `shortest` is implemented in C++, and its Rust signature says that the result borrows from
    // the arguments.
    let short = generated::shortest(&hello, &greetings);
    println!("Rust: shortest = {short}");

    let input = String::from("hello world");
    let rest = {
        let mut parser = Parser::new(&input);
        // Implemented in C++. The result borrows from `input`, so it outlives `parser`.
        parser.skip_and_rest(6)
    };
    println!("Rust: rest = {rest}");
}

fn use_chooser(chooser: Box<dyn Chooser>) {
    println!("Rust: chooser picked {}", chooser.choose("one", "three"));
}

fn use_chooser_fn(chooser: Box<dyn for<'a> Fn(&'a str, &'a str) -> &'a str>) {
    println!("Rust: chooser fn picked {}", chooser("one", "three"));
}
