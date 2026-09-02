FROM alpine:latest

WORKDIR /app
COPY ./target/x86_64-unknown-linux-musl/release/dsp-rs .

# schemas
COPY third_party/dsp-spec/artifacts/src/main/resources ./third_party/dsp-spec/artifacts/src/main/resources

ENV RUST_BACKTRACE=full
ENV RUST_LOG=debug

CMD ["./dsp-rs"]
