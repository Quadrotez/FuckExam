# Контекст сборки: этот каталог (./relay).
#   docker build -f relay/Dockerfile relay/
#
# Результат: минимальный образ с бинарником relay (ws://0.0.0.0:7444).

FROM rust:1-bookworm AS build
WORKDIR /build
COPY . .
RUN cargo build --release

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /build/target/release/fuck-exam-relay /usr/local/bin/fuck-exam-relay
ENV RELAY_PORT=7444
EXPOSE 7444
USER nobody
CMD ["fuck-exam-relay"]