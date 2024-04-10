# nara

A minimal, single-threaded async i/o runtime for Rust. Only has
three dependencies (libc for the `poll` system call, futures-util and futures-io
for the read/write traits).

Current status:

- tested on linux and macos
- executor: `spawn`, `spawn_blocking`
- net: TcpStream

Nara Lines of code:  600
Tokio lines of code: 82513

