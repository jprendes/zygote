use std::cell::RefCell;
use std::io::{Read, Write};
use std::ops::{Deref, DerefMut};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, IntoRawFd, OwnedFd, RawFd};

use serde::{Deserialize, Serialize};

thread_local! {
    static FDS: RefCell<Vec<Option<RawFd>>> = RefCell::default();
}

/// Wrapper type that allows sending file descriptors to and from the zygote.
///
/// The wrapped type must implement the [`AsFd`] and [`FromRawFd`] traits.
///
/// ```rust
/// # use std::os::unix::net::UnixStream;
/// # use std::net::Shutdown;
/// # use std::io::{Write, Read, BufReader, BufRead as _, read_to_string};
/// # use std::ops::DerefMut;
/// # use zygote::Zygote;
/// # use zygote::WireFd;
/// #[derive(serde::Serialize, serde::Deserialize)]
/// struct WriteArgs {
///     writer: WireFd<UnixStream>,
///     content: String,
/// }
///
/// let (writer, mut reader) = UnixStream::pair().unwrap();
///
/// let args = WriteArgs {
///     writer: writer.into(),
///     content: "hello world!".into(),
/// };
///
/// Zygote::global().run(|mut args| {
///     write!(args.writer, "{}", args.content).unwrap();
///     args.writer.shutdown(Shutdown::Both).unwrap();
/// }, args);
///
/// let content = read_to_string(reader).unwrap();
///
/// assert_eq!(content, "hello world!");
/// ```
pub struct WireFd<T>(T);

impl<T> From<T> for WireFd<T> {
    fn from(value: T) -> Self {
        Self(value)
    }
}

impl<T> WireFd<T> {
    pub fn new(val: T) -> WireFd<T> {
        val.into()
    }

    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T: AsFd> AsRawFd for WireFd<T> {
    fn as_raw_fd(&self) -> RawFd {
        self.0.as_fd().as_raw_fd()
    }
}

impl<T: AsFd> AsFd for WireFd<T> {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
    }
}

impl<T: IntoRawFd> IntoRawFd for WireFd<T> {
    fn into_raw_fd(self) -> RawFd {
        self.0.into_raw_fd()
    }
}

impl<T: FromRawFd> FromRawFd for WireFd<T> {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        T::from_raw_fd(fd).into()
    }
}

impl<T> Deref for WireFd<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> DerefMut for WireFd<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<T: AsFd> Serialize for WireFd<T> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let n = push_fd(self);
        Serialize::serialize(&n, serializer)
    }
}

impl<'a, T: FromRawFd> Deserialize<'a> for WireFd<T> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'a>,
    {
        let n = Deserialize::deserialize(deserializer)?;
        let fd = take_fd(n).expect("expected an FD");
        Ok(Self(unsafe { T::from_raw_fd(fd.into_raw_fd()) }))
    }
}

impl<T: Write> Write for WireFd<T> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }

    fn write_vectored(&mut self, bufs: &[std::io::IoSlice<'_>]) -> std::io::Result<usize> {
        self.0.write_vectored(bufs)
    }
}

impl<T: Read> Read for WireFd<T> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }

    fn read_vectored(&mut self, bufs: &mut [std::io::IoSliceMut<'_>]) -> std::io::Result<usize> {
        self.0.read_vectored(bufs)
    }
}

pub(crate) fn swap_fds(fds: Vec<RawFd>) -> Vec<RawFd> {
    let fds = fds.into_iter().map(Option::Some).collect();
    FDS.with_borrow_mut(|old_fds| std::mem::replace(old_fds, fds))
        .into_iter()
        .flatten()
        .collect()
}

fn push_fd(fd: &impl AsFd) -> usize {
    FDS.with_borrow_mut(|fds| {
        let n = fds.len();
        fds.push(Some(fd.as_fd().as_raw_fd()));
        n
    })
}

fn take_fd(n: usize) -> Option<OwnedFd> {
    FDS.with_borrow_mut(|fds| fds.get_mut(n).map(Option::take))
        .flatten()
        .map(|fd| unsafe { OwnedFd::from_raw_fd(fd) })
}
