EXAMPLES_CARGO_PROFILE ?= debug

a.out: src/main.rs build.rs main.zng shapes.h
	cargo build --profile $(EXAMPLES_CARGO_PROFILE:debug=dev)
	cp ../../target/$(EXAMPLES_CARGO_PROFILE)/example-cpp-make-unique a.out

.PHONY: clean
clean:
	rm -f a.out actual_output.txt generated.h generated.cpp src/generated.rs
