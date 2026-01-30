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

# Make a small set of test files for exporting
RUN bash -ex -c " \
    mkdir -p /tmp/fs123-export ; \
    cd /tmp/fs123-export ; \
    echo 'hi' > hello.txt ; \
    ln -s hello.txt hello2.txt"

COPY / /src/

WORKDIR /src

RUN cargo build --workspace

ENTRYPOINT ["/usr/bin/tini", "/usr/bin/sleep", "--", "86400"]
