FROM ubuntu:26.04

# Install necessary packages (including build tools and OpenSSL development headers)
RUN apt update && apt install -y \
    cargo \
    curl \
    fuse libfuse-dev \
    git \
    build-essential \
    libssl-dev \
    pkg-config \
    tini \
    wget \
    xxd

RUN install -d -m 755 /srv/fs123

COPY / /src/

WORKDIR /src

RUN cargo build --workspace

ENTRYPOINT ["/usr/bin/tini", "/usr/bin/sleep", "--", "86400"]
