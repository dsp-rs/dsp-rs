# DSP Implementation in Rust

## Building

Use the provided [build.sh](build.sh) script for compiling the connector binary and creating a local Docker container.

## Setup and usage instructions

The `docker-compose` folder contains a demo setup with two dataspace participants:

- [Setting up SSI wallets and connector](docker-compose/SETUP.md)
- [Negotiating a contract and accessing a dataset](docker-compose/USAGE.md)

## Run connector in TCK mode

```bash
RUST_LOG=debug CONFIG_PATH="tck/config.json" cargo run --bin dsp-rs --features tck
```

The TCK configuration is provided in [dsp-rs.tck.properties](dsp-rs.tck.properties). Further information about running
the test suite can be found on the official [repo](https://github.com/eclipse-dataspacetck/dsp-tck).
