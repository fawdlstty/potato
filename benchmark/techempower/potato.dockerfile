FROM rust:1.98

ADD ./ /potato
WORKDIR /potato

RUN RUSTFLAGS="-C target-cpu=native" cargo build --release

EXPOSE 8080

CMD ./target/release/potato-tfb
