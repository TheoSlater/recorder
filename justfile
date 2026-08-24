check:
    cargo check

run:
    cargo run

release:
    cargo run --release

fmt:
    cargo fmt

lint:
    cargo clippy --all-targets --all-features -- -D warnings

test:
    cargo test

verify: fmt lint test
