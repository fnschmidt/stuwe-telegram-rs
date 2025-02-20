FROM rust:bookworm AS build
COPY ./src ./src
COPY ./Cargo.lock .
COPY ./Cargo.toml .

RUN cargo build --release


FROM debian:bookworm-slim AS stuwe-telegram-rs
RUN apt-get update && \
  apt-get install -y \
  libsqlite3-0 \
  libssl3 \
  ca-certificates \
  && \
  apt-get autoremove -y && \
  apt-get clean -y && \
  rm -rf /var/lib/apt/lists/*
COPY GEANT_OV_RSA_CA_4_tcs-cert3.pem /etc/ssl/certs/GEANT_OV_RSA_CA_4_tcs-cert3.pem
RUN c_rehash
COPY --from=build ./target/release/stuwe-telegram-rs /app/stuwe-telegram-rs
WORKDIR /app/data
ENTRYPOINT ["/app/stuwe-telegram-rs"]

#FROM debian:bookworm-slim AS mensi-telegram-rs
#RUN apt-get update && \
#  apt-get install -y \
#  libsqlite3-0 \
#  libssl3 \
#  ca-certificates \
#  && \
#  apt-get autoremove -y && \
#  apt-get clean -y && \
#  rm -rf /var/lib/apt/lists/*
#COPY GEANT_OV_RSA_CA_4_tcs-cert3.pem /etc/ssl/certs/GEANT_OV_RSA_CA_4_tcs-cert3.pem
#RUN c_rehash
#COPY --from=build ./target/release/mensi-telegram-rs /app/mensi-telegram-rs
#WORKDIR /app/data
#ENTRYPOINT ["/app/mensi-telegram-rs"]
