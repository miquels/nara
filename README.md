# nara

A minimal, single-threaded async i/o runtime for Rust.

Only has four dependencies (libc for the `poll` system call, futures-util and futures-io
for the read/write traits, and socket2 for sockets). Can run Send and non-Send futures.

## Current status

- tested on linux, macos and freebsd
- executor: `block_on`.
- task: `spawn`, `spawn_blocking` (threadpool), `JoinHandle`
- reactor: `AsyncRead` / `AsyncWrite`, etc
- timer: `sleep`, `sleep_until`.
- net: `TcpStream`
- sync: `mpsc::channel()`
- unsync: `mpsc::channel()` (not Send + 'static but 3x faster)

There are 5 'unsafe' blocks, all in src/syscall.rs, implementing
the poll(2), pipe(2) and write(2) system calls.

## Example.

There's an example in examples/naratest.rs:

```
cargo run --example naratest
```

## Size
Lines of code, counted by `cloc src`

Tokio: 82513  
Nara:  1147
