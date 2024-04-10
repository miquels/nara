# nara

A minimal, single-threaded async i/o runtime for Rust.

Only has three dependencies (libc for the `poll` system call, futures-util and futures-io
for the read/write traits). Can run Send and non-Send futures.

Current status:

- tested on linux, macos and freebsd
- executor: `block_on`.
- task: `spawn`, `spawn_blocking`, `JoinHandle`
- reactor: `AsyncRead` / `AsyncWrite`, etc
- timer: `sleep`, `sleep_until`.
- net: `TcpStream`

There are 5 'unsafe' blocks, all in src/syscall.rs, implementing
the poll(2), pipe(2) and write(2) system calls.

Nara Lines of code:  600  
Tokio lines of code: 82513

