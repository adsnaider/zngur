EXAMPLES_CARGO_PROFILE ?= debug

a.out: main.cpp generated.h generated.cpp src/generated.rs src/lib.rs ../../target/$(EXAMPLES_CARGO_PROFILE)/libexample_lifetimes.a
	${CXX} -std=c++20 -Werror main.cpp generated.cpp -g -L ../../target/$(EXAMPLES_CARGO_PROFILE)/ -l example_lifetimes

../../target/$(EXAMPLES_CARGO_PROFILE)/libexample_lifetimes.a:
	cargo build --profile $(EXAMPLES_CARGO_PROFILE:debug=dev)

generated.h generated.cpp ./src/generated.rs: main.zng
	cd ../../zngur-cli && cargo run g -i ../examples/lifetimes/main.zng --crate-name "crate"

.PHONY: ../../target/$(EXAMPLES_CARGO_PROFILE)/libexample_lifetimes.a generated.h clean

clean:
	rm -f generated.h generated.cpp src/generated.rs a.out actual_output.txt
