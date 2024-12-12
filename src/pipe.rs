use std::io::{Read as _, Write as _};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, IntoRawFd, OwnedFd, RawFd};
use std::os::unix::net::UnixStream;

use byteorder::{NativeEndian, ReadBytesExt, WriteBytesExt};
use withfd::{WithFd, WithFdExt};

use crate::codec::Codec;
use crate::error::Error;
use crate::fds::swap_fds;

pub struct Pipe(pub(crate) WithFd<UnixStream>, pub(crate) RawFd);

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
    pub fn send<T: Codec>(&mut self, data: &T) -> Result<(), Error> {
        let n = swap_fds(vec![]).len();
        assert_eq!(n, 0);

        let bytes = Codec::serialize(data)?;

        let fds: Vec<_> = swap_fds(vec![])
            .into_iter()
            .map(|fd| unsafe { BorrowedFd::borrow_raw(fd) })
            .collect();

        self.0.write_u64::<NativeEndian>(bytes.len() as _)?;
        self.0.write_all(&bytes)?;

        self.0.write_u64::<NativeEndian>(fds.len() as _)?;
        self.0.write_with_fd(&[42], &fds)?;
        Ok(())
    }

    pub fn recv<T: Codec>(&mut self) -> Result<T, Error> {
        let len = self.0.read_u64::<NativeEndian>()? as usize;
        let mut buffer = vec![0; len];
        self.0.read_exact(&mut buffer)?;

        let len = self.0.read_u64::<NativeEndian>()? as usize;
        let byte = self.0.read_u8()?;
        assert_eq!(byte, 42);
        let fds = self
            .0
            .take_fds()
            .take(len)
            .map(|fd| fd.into_raw_fd())
            .collect();

        let n = swap_fds(fds).len();
        assert_eq!(n, 0);

        let res = Codec::deserialize(&buffer)?;

        let n = swap_fds(vec![])
            .into_iter()
            .map(|fd| unsafe { OwnedFd::from_raw_fd(fd) })
            .count();
        assert_eq!(n, 0);

        Ok(res)
    }
}

/*
impl Codec for Pipe {
    fn serialize(&self) -> Result<(Vec<u8>, Vec<BorrowedFd>), rmp_serde::encode::Error> {
        Ok((vec![], vec![self.as_fd()]))
    }

    fn deserialize(
        buf: impl AsRef<[u8]>,
        mut fds: Vec<OwnedFd>,
    ) -> Result<Self, rmp_serde::decode::Error> {
        if !buf.as_ref().is_empty() {
            return Err(rmp_serde::decode::Error::InvalidMarkerRead(
                std::io::Error::other("expected no bytes"),
            ));
        }
        if fds.len() != 1 {
            return Err(rmp_serde::decode::Error::InvalidMarkerRead(
                std::io::Error::other("expected only one fd"),
            ));
        }
        let fd = fds.remove(0);
        let pipe = unsafe { UnixStream::from_raw_fd(fd.into_raw_fd()) };
        Ok(Pipe::new(pipe))
    }
}
*/
