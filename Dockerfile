ARG PG_MAJOR=17
ARG PGVECTOR_VERSION=0.8.1
ARG DEBIAN_SUITE=trixie

FROM pgvector/pgvector:${PGVECTOR_VERSION}-pg${PG_MAJOR}-${DEBIAN_SUITE} AS builder

ENV DEBIAN_FRONTEND=noninteractive \
    CARGO_HOME=/cargo \
    RUSTUP_HOME=/rustup
ENV PATH="$CARGO_HOME/bin:$PATH"

RUN set -eux; \
  apt-get update; \
  apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    build-essential \
    clang \
    llvm \
    pkg-config \
    libssl-dev \
    sudo \
    postgresql-server-dev-${PG_MAJOR}; \
  rm -rf /var/lib/apt/lists/*

RUN set -eux; \
  curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable; \
  rustc --version; \
  cargo --version

# Install cargo-pgrx early (independent of project files)
RUN set -eux; \
  cargo install cargo-pgrx --version 0.16.1

WORKDIR /workspace

# Initialize pgrx (independent of project files)
RUN set -eux; \
  cargo pgrx init --pg${PG_MAJOR} /usr/lib/postgresql/${PG_MAJOR}/bin/pg_config

# Copy dependency files first
COPY Cargo.toml Cargo.lock* ./
COPY pg_s2.control ./

# Build dependencies with dummy source (cached unless Cargo.toml changes)
RUN set -eux; \
  mkdir -p src/bin; \
  echo 'fn main() {}' > src/bin/pgrx_embed.rs; \
  echo 'use pgrx::prelude::*; pgrx::pg_module_magic!();' > src/lib.rs; \
  cargo build --release --lib; \
  rm -rf src

# Copy actual source code
COPY src ./src

# Build actual extension (only this layer rebuilds when src changes)
RUN set -eux; \
  cargo pgrx install --release

FROM pgvector/pgvector:${PGVECTOR_VERSION}-pg${PG_MAJOR}-${DEBIAN_SUITE}

COPY --from=builder /usr/lib/postgresql/${PG_MAJOR}/lib/pg_s2.so /usr/lib/postgresql/${PG_MAJOR}/lib/
COPY --from=builder /usr/share/postgresql/${PG_MAJOR}/extension/pg_s2* /usr/share/postgresql/${PG_MAJOR}/extension/
