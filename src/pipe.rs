use std::io::{self, Read as _, Write as _};
use std::mem::transmute;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::net::UnixStream;
use std::slice;

use withfd::{WithFd, WithFdExt};

use crate::codec::Codec;
use crate::error::Error;
use crate::fd::swap_fds;

pub struct Pipe(WithFd<UnixStream>, RawFd);

impl Pipe {
    pub(crate) fn new(pipe: UnixStream) -> Self {
        let fd = pipe.as_raw_fd();
        let pipe = pipe.with_fd();
        Pipe(pipe, fd)
    }

    pub fn pair() -> std::io::Result<(Pipe, Pipe)> {
        let (p1, p2) = UnixStream::pair()?;
        Ok((Pipe::new(p1), Pipe::new(p2)))
    }
}

impl AsRawFd for Pipe {
    fn as_raw_fd(&self) -> RawFd {
        self.1
    }
}

impl FromRawFd for Pipe {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        Self::new(UnixStream::from_raw_fd(fd))
    }
}

impl AsFd for Pipe {
    fn as_fd(&self) -> BorrowedFd<'_> {
        unsafe { BorrowedFd::borrow_raw(self.1) }
    }
}

impl Pipe {
    fn write_usize(&mut self, n: usize) -> io::Result<()> {
        let buf = n.to_ne_bytes();
        self.0.write_all(&buf)?;
        Ok(())
    }

    fn read_usize(&mut self) -> io::Result<usize> {
        let mut buf = [0u8; std::mem::size_of::<usize>()];
        self.0.read_exact(&mut buf)?;
        Ok(usize::from_ne_bytes(buf))
    }

    fn write_sized(&mut self, buf: &[u8]) -> io::Result<()> {
        self.write_usize(buf.len())?;
        self.0.write_all(buf)?;
        Ok(())
    }

    fn read_sized(&mut self) -> io::Result<Vec<u8>> {
        let size = self.read_usize()?;
        let mut buf = vec![0; size];
        self.0.read_exact(&mut buf)?;
        Ok(buf)
    }

    fn write_fds(&mut self, fds: &[BorrowedFd]) -> io::Result<()> {
        self.write_usize(fds.len())?;
        self.0.write_with_fd(&[42], fds)?;
        Ok(())
    }

    fn read_fds(&mut self) -> io::Result<Vec<OwnedFd>> {
        let len = self.read_usize()?;
        let mut byte = 0;
        self.0.read_exact(slice::from_mut(&mut byte))?;
        assert_eq!(byte, 42);
        let fds = self.0.take_fds().take(len).collect();
        Ok(fds)
    }

    pub fn send<T: Codec>(&mut self, data: &T) -> Result<(), Error> {
        let n = swap_fds(vec![]).len();
        assert_eq!(n, 0);

        let bytes = Codec::serialize(data)?;

        // safety: BorrowedFd is repr(transparent) over RawFd
        let fds: Vec<BorrowedFd<'_>> = unsafe { transmute(swap_fds(vec![])) };

        self.write_sized(&bytes)?;
        self.write_fds(&fds)?;

        Ok(())
    }

    pub fn recv<T: Codec>(&mut self) -> Result<T, Error> {
        let buffer = self.read_sized()?;
        let fds = self.read_fds()?;

        // safety: OwnedFd is repr(transparent) over RawFd
        let fds: Vec<RawFd> = unsafe { transmute(fds) };

        let n = swap_fds(fds).len();
        assert_eq!(n, 0);

        let res = Codec::deserialize(&buffer)?;

        // safety: OwnedFd is repr(transparent) over RawFd
        let fds: Vec<OwnedFd> = unsafe { transmute(swap_fds(vec![])) };
        assert_eq!(fds.len(), 0);

        Ok(res)
    }
}
