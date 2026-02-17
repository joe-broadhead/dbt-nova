SHELL := /bin/bash

.PHONY: build install release-dry-run test lint bench coverage clean ci

build:
	cargo build --release --all-features

install:
	cargo install --path . --all-features --locked --force

release-dry-run:
	@echo "Release dry-run (local build + package)"
	cargo build --release --all-features

test:
	cargo test --all-features

lint:
	cargo clippy --all-targets --all-features -- -D warnings

bench:
	cargo bench --all-features

coverage:
	cargo llvm-cov --all-features --workspace --summary-only

clean:
	cargo clean

ci:
	cargo fmt
	cargo test --all-features
	cargo clippy --all-targets --all-features -- -D warnings
