// This module contains the interface to unsafe system calls.
use std::fs::File;
use std::io;
use std::os::fd::{FromRawFd, RawFd};

fn result(val: isize) -> io::Result<usize> {
    match val {
        -1 => Err(std::io::Error::last_os_error()),
        v => Ok(v as usize),
    }
}

pub fn poll(pollfds: &mut [libc::pollfd], timeout: i32) -> io::Result<usize> {

    // The types are the same, so the try_into().unwrap() should get optimized out.
    let timeout: libc::c_int = timeout.try_into().unwrap();
    let nfds = pollfds.len() as libc::nfds_t;

    // SAFETY: very basic linux system call.
    let res = unsafe {
        libc::poll(pollfds.as_mut_ptr(), nfds, timeout)
    };
    result(res as isize)
}

fn non_blocking(fd: RawFd) {
    // SAFETY: very basic linux system calls, no pointers.
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL, 0);
        libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
    }
}

// Note that we change this pipe to non-blocking on the read side,
// but leave it as _blocking_ on the write side!
pub fn pipe() -> io::Result<(File, File)> {
    let mut fds: [libc::c_int; 2] = [0; 2];
    // SAFETY: very basic linux system call.
    let res = unsafe {
        libc::pipe(fds.as_mut_ptr())
    };
    non_blocking(fds[0]);
    // SAFETY: constructing a File from fd we just opened.
    let files = unsafe {
        (File::from_raw_fd(fds[0]), File::from_raw_fd(fds[1]))
    };
    result(res as isize).map(|_| files)
}

pub fn write(fd: RawFd, buf: &[u8]) -> io::Result<usize> {
    // SAFETY: very basic linux system call.
    let res = unsafe {
        libc::write(fd, buf.as_ptr() as *const libc::c_void, buf.len() as libc::size_t)
    };
    result(res).map(|v| v as usize)
}
