default:
  @just --list

build:
  cargo build

run *args:
  cargo run -- {{args}}

test:
  cargo test

clippy:
  cargo clippy

install:
  cargo install --path .

check:
  cargo check
