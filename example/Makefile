#
# Build
#

.PHONY: build
build:
	cargo build

.PHONY: build-examples
build-examples: build
	cargo build --example server
	cargo build --example client
	cargo build --example async-server --features async
	cargo build --example async-client --features async
	cargo build --example async-stream-server --features async
	cargo build --example async-stream-client --features async

.PHONY: deps
deps:
	rustup update stable
	rustup default stable
	rustup component add rustfmt clippy
