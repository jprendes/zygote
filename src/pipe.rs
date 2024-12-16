use std::io::{self, Read as _, Write as _};
use std::mem::transmute;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd, RawFd};
use std::slice;

use withfd::{UnixStream, SCM_MAX_FD};

use crate::wire::{Wire, AsWire};
use crate::error::Error;
use crate::fd::swap_fds;

mod withfd;

pub struct Pipe(UnixStream);

impl Pipe {
    pub(crate) fn new(pipe: UnixStream) -> Self {
        Pipe(pipe)
    }

    pub fn pair() -> std::io::Result<(Pipe, Pipe)> {
        let (p1, p2) = UnixStream::pair()?;
        Ok((Pipe::new(p1), Pipe::new(p2)))
    }
}

impl AsRawFd for Pipe {
    fn as_raw_fd(&self) -> RawFd {
        self.0.as_raw_fd()
    }
}

impl FromRawFd for Pipe {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        Self::new(UnixStream::from_raw_fd(fd))
    }
}

impl AsFd for Pipe {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
        //unsafe { BorrowedFd::borrow_raw(self.1) }
    }
}

impl Pipe {
    fn write_usize(&mut self, n: usize) -> io::Result<()> {
        self.0.write_all(&n.to_ne_bytes())
    }

    fn read_usize(&mut self) -> io::Result<usize> {
        let mut buf = [0u8; std::mem::size_of::<usize>()];
        self.0.read_exact(&mut buf)?;
        Ok(usize::from_ne_bytes(buf))
    }

    fn write_sized(&mut self, buf: &[u8]) -> io::Result<()> {
        self.write_usize(buf.len())?;
        self.0.write_all(buf)
    }

    fn read_sized(&mut self) -> io::Result<Vec<u8>> {
        let size = self.read_usize()?;
        let mut buf = vec![0; size];
        self.0.read_exact(&mut buf)?;
        Ok(buf)
    }

    fn write_fds(&mut self, mut fds: &[BorrowedFd]) -> io::Result<()> {
        while fds.len() > SCM_MAX_FD {
            self.0.write_with_fd(&[255], &fds[..SCM_MAX_FD])?;
            fds = &fds[SCM_MAX_FD..];
        }
        self.0.write_with_fd(&[fds.len() as u8], fds)?;
        Ok(())
    }

    fn read_fds(&mut self) -> io::Result<Vec<OwnedFd>> {
        let mut len = 0usize;
        let mut byte = 255;
        while byte == 255 {
            self.0.read_exact(slice::from_mut(&mut byte))?;
            len += (byte as usize).min(SCM_MAX_FD);
        }
        let fds = self.0.take_fds();
        assert_eq!(fds.len(), len);
        Ok(fds)
    }

    pub fn send<'a, T: Wire>(&mut self, data: impl AsWire<T>) -> Result<(), Error> {
        let n = swap_fds(vec![]).len();
        assert_eq!(n, 0);

        let bytes = data.serialize()?;

        // safety: BorrowedFd is repr(transparent) over RawFd
        let fds: Vec<BorrowedFd<'_>> = unsafe { transmute(swap_fds(vec![])) };

        self.write_sized(&bytes)?;
        self.write_fds(&fds)?;

        Ok(())
    }

    pub fn recv_into<'de, T: Wire>(&mut self, buffer: &'de mut Vec<u8>) -> Result<T, Error> {
        *buffer = self.read_sized()?;
        let fds = self.read_fds()?;

        // safety: OwnedFd is repr(transparent) over RawFd
        let fds: Vec<RawFd> = unsafe { transmute(fds) };

        let n = swap_fds(fds).len();
        assert_eq!(n, 0);

        let res = <T as Wire>::deserialize(buffer)?;

        // safety: OwnedFd is repr(transparent) over RawFd
        let fds: Vec<OwnedFd> = unsafe { transmute(swap_fds(vec![])) };
        assert_eq!(fds.len(), 0);

        Ok(res)
    }

    pub fn recv<T: Wire>(&mut self) -> Result<T, Error> {
        let mut buffer = Vec::default();
        self.recv_into(&mut buffer)
    }
}
