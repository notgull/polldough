// GNU GPL v3 License

#[cfg(unix)]
pub use std::os::raw::unix::io::{AsRawFd as _, RawFd as Raw};
#[cfg(windows)]
pub use std::os::windows::io::{AsRawHandle as _, AsRawSocket as _, RawHandle as Raw};
#[cfg(not(any(unix, windows)))]
compile_error! { "Unsupported platform" }

/// A wrapper around a system-specific file descriptor.
pub unsafe trait Source {
    /// The type of the system-specific file descriptor.
    const SOURCE_TYPE: SourceType;

    /// Get the raw underlying file descriptor.
    fn as_raw(&self) -> Raw;
}

/// Is this source a socket or a file?
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SourceType {
    /// A socket.
    Socket,
    /// A file.
    File,
}

macro_rules! impl_source {
    ($($(#[$meta: meta])* $ty: ty, $name: ident, $as_raw_windows: ident),*) => {
        $(
            $(#[$meta])*
            unsafe impl Source for $ty {
                const SOURCE_TYPE: SourceType = SourceType::$name;

                fn as_raw(&self) -> Raw {
                    cfg_if::cfg_if! {
                        if #[cfg(unix)] {
                            self.as_raw_fd()
                        } else if #[cfg(windows)] {
                            self.$as_raw_windows() as usize as Raw
                        } else {
                            compile_error! { "Unsupported platform" }
                        }
                    }
                }
            }
        )*
    };
}

impl_source! {
    std::net::TcpStream, Socket, as_raw_socket,
    std::net::TcpListener, Socket, as_raw_socket,
    std::net::UdpSocket, Socket, as_raw_socket,
    std::fs::File, File, as_raw_handle,
    std::io::Stderr, File, as_raw_handle,
    std::io::Stdout, File, as_raw_handle,
    std::io::Stdin, File, as_raw_handle,
    std::io::StderrLock<'_>, File, as_raw_handle,
    std::io::StdoutLock<'_>, File, as_raw_handle,
    std::io::StdinLock<'_>, File, as_raw_handle,
    std::process::ChildStdin, File, as_raw_handle,
    std::process::ChildStdout, File, as_raw_handle,
    std::process::ChildStderr, File, as_raw_handle,
    #[cfg(unix)] std::os::unix::net::UnixStream, File, as_raw_fd,
    #[cfg(unix)] std::os::unix::net::UnixListener, File, as_raw_fd,
    #[cfg(unix)] std::os::unix::net::UnixDatagram, File, as_raw_fd
}