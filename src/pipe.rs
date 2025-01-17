use std::any::{type_name, TypeId};
use std::io::{self, Read as _, Write as _};
use std::mem::transmute;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd, RawFd};
use std::slice;

use stream::{UnixStream, SCM_MAX_FD};

use crate::error::Error;
use crate::fd::swap_fds;
use crate::wire::{AsWire, Wire};

mod stream;

#[repr(transparent)]
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

impl From<OwnedFd> for Pipe {
    fn from(fd: OwnedFd) -> Self {
        Self::new(fd.into())
    }
}

impl AsFd for Pipe {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
    }
}

impl Pipe {
    fn write_type_id<T: ?Sized + 'static>(&mut self) -> io::Result<()> {
        let tag: [u8; size_of::<TypeId>()] = unsafe { transmute(TypeId::of::<T>()) };
        self.0.write_all(&tag)
    }

    fn read_type_id(&mut self) -> io::Result<TypeId> {
        let mut tag = [0u8; size_of::<TypeId>()];
        self.0.read_exact(&mut tag)?;
        let tag: TypeId = unsafe { transmute(tag) };
        Ok(tag)
    }

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

    pub fn send<T: Wire>(&mut self, data: impl AsWire<T>) -> Result<(), Error> {
        let n = swap_fds(vec![]).len();
        assert_eq!(n, 0, "orphaned file descriptors in channel");

        let bytes = data.serialize()?;

        // safety: BorrowedFd is repr(transparent) over RawFd
        let fds: Vec<BorrowedFd<'_>> = unsafe { transmute(swap_fds(vec![])) };

        self.write_type_id::<T>()?;
        self.write_sized(&bytes)?;
        self.write_fds(&fds)?;

        Ok(())
    }

    pub fn recv_parts_into(
        &mut self,
        type_id: &mut TypeId,
        buffer: &mut Vec<u8>,
        fds: &mut Vec<OwnedFd>,
    ) -> Result<(), Error> {
        *type_id = self.read_type_id()?;
        *buffer = self.read_sized()?;
        *fds = self.read_fds()?;
        Ok(())
    }

    pub fn recv_delayed(&mut self) -> Result<DelayedRecv, Error> {
        let mut delayed_recv = DelayedRecv {
            type_id: TypeId::of::<u8>(),
            buffer: vec![],
            fds: vec![],
        };

        self.recv_parts_into(
            &mut delayed_recv.type_id,
            &mut delayed_recv.buffer,
            &mut delayed_recv.fds,
        )?;

        Ok(delayed_recv)
    }

    pub fn recv<T: Wire>(&mut self) -> Result<T, Error> {
        self.recv_delayed()?.deserialize::<T>()
    }
}

pub struct DelayedRecv {
    type_id: TypeId,
    buffer: Vec<u8>,
    fds: Vec<OwnedFd>,
}

impl DelayedRecv {
    pub fn deserialize<T: Wire>(self) -> Result<T, Error> {
        if self.type_id != TypeId::of::<T>() {
            return Err(Error::Decode(rmp_serde::decode::Error::Uncategorized(
                format!("Invalid type id for {}", type_name::<T>()),
            )));
        }

        // safety: OwnedFd is repr(transparent) over RawFd
        let fds: Vec<RawFd> = unsafe { transmute(self.fds) };

        let n = swap_fds(fds).len();
        assert_eq!(n, 0, "orphaned file descriptors in channel");

        let res = <T as Wire>::deserialize(&self.buffer)?;

        // safety: OwnedFd is repr(transparent) over RawFd
        let fds: Vec<OwnedFd> = unsafe { transmute(swap_fds(vec![])) };
        assert_eq!(fds.len(), 0, "orphaned file descriptors in channel");

        Ok(res)
    }
}

#[cfg(test)]
mod test {
    use super::Pipe;

    #[test]
    fn basic() {
        let (mut s, mut d) = Pipe::pair().unwrap();

        s.send::<String>("hello world!").unwrap();
        let msg = d.recv::<String>().unwrap();

        assert_eq!(msg, "hello world!");
    }

    #[test]
    fn validate_type() {
        let (mut s, mut d) = Pipe::pair().unwrap();

        s.send::<String>("hello world!").unwrap();
        d.recv::<Vec<u8>>().unwrap_err();
    }
}
