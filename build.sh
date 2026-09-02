#!/bin/sh

# build connector binary
cargo build --target x86_64-unknown-linux-musl --release

# build container
docker build --platform linux/amd64 -t connector:latest .
