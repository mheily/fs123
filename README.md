# fs123 (Rust rewrite)

*WARNING* THIS IS AN UNFINISHED EXPERIMENTAL WORK IN PROGRESS -- DO NOT USE WITH ANY IMPORTANT DATA.
See [LICENSE.txt](./LICENSE.txt) for the full details, but TL;DR **USE THIS AT YOUR OWN RISK**

## Overview

This is a complete rewrite of the fs123 client and server in Rust, using AI to generate most of the code.

## Development HOWTO

Requirements to build this software:
* MacOS with MacFUSE installed, or Linux (see the Dockerfile for required packages).
* Docker

### Build instructions

* Run the `./configure` script to setup the working copy.

```
./configure
```

Start the containers.

```
docker-compose up -d
```

Login

```
docker-compose exec client bash
```

View the contents of the mounted fs123 filesystem.

```
# ls /mnt
hello.txt  hello2.txt
```

Run all tests:

```
docker-compose exec client cargo test --workspace -- --include-ignored --test-threads=1
```
