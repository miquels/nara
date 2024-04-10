# nara

A minimal, single-threaded async i/o runtime for Rust. Only has
three dependencies (libc for the `poll` system call, futures-util and futures-io
for the read/write traits).

Current status:

- tested on linux, macos and freebsd
- executor: `spawn`, `spawn_blocking`
- net: TcpStream

There are 5 'unsafe' blocks, all in src/syscall.rs, implementing
the poll(2), pipe(2) and write(2) system calls.

Nara Lines of code:  600  
Tokio lines of code: 82513

