all: debug test

#
# Build
#

.PHONY: debug
debug:
	cargo build --verbose --all-targets

.PHONY: release
release:
	cargo build --release

.PHONY: build
build: debug

#
# Tests and linters
#

.PHONY: test
test:
ifeq ($OS,Windows_NT)
	# async isn't enabled for windows, don't test that feature
	cargo test --verbose
else
	cargo test --all-features --verbose
endif
	

.PHONY: check
check:
	cargo fmt --all -- --check
	cargo clippy --all-targets --all-features -- -D warnings

.PHONY: deps
deps:
	rustup update stable
	rustup default stable
	rustup component add rustfmt clippy
