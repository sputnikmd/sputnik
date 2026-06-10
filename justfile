default:
    @just --list

# Run the project
run:
    cargo run

# Build the project
build:
    cargo build

# Run tests
test:
    cargo test

# Format the code
fmt:
    cargo fmt

# Run clippy
clippy:
    cargo clippy
