use std::io;
use std::mem::transmute;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd, RawFd};
use std::os::unix::net::UnixStream as StdUnixStream;

use nix::sys::socket::{recvmsg, sendmsg, ControlMessage, ControlMessageOwned, MsgFlags};

pub(super) const SCM_MAX_FD: usize = 253; // see https://man7.org/linux/man-pages/man7/unix.7.html

pub struct UnixStream {
    inner: StdUnixStream,
    fds: Vec<OwnedFd>,
    cmsg: Vec<u8>,
}

impl UnixStream {
    pub fn new(inner: StdUnixStream) -> UnixStream {
        Self {
            inner,
            fds: vec![],
            cmsg: nix::cmsg_space!([RawFd; SCM_MAX_FD]),
        }
    }

    pub fn pair() -> io::Result<(UnixStream, UnixStream)> {
        let (p1, p2) = StdUnixStream::pair()?;
        Ok((p1.into(), p2.into()))
    }
}

impl From<StdUnixStream> for UnixStream {
    fn from(inner: StdUnixStream) -> Self {
        Self::new(inner)
    }
}

impl UnixStream {
    fn read_with_fd(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let fd = self.inner.as_raw_fd();
        let mut buf = [io::IoSliceMut::new(buf)];
        let recvmsg = recvmsg::<()>(fd, &mut buf, Some(&mut self.cmsg), MsgFlags::empty())?;
        for cmsg in recvmsg.cmsgs()? {
            if let ControlMessageOwned::ScmRights(fds) = cmsg {
                // Safety: OwnedFd is repr(transparent) over RawFd
                let fds: Vec<OwnedFd> = unsafe { transmute(fds) };
                self.fds.extend(fds);
            }
        }
        Ok(recvmsg.bytes)
    }

    pub fn write_with_fd(&mut self, buf: &[u8], fds: &[BorrowedFd<'_>]) -> std::io::Result<usize> {
        let fd = self.inner.as_raw_fd();
        // Safety: BorrowedFd is repr(transparent) over RawFd
        let fds: &[RawFd] = unsafe { transmute(fds) };
        let cmsg: ControlMessage<'_> = ControlMessage::ScmRights(fds);
        let sendmsg = sendmsg::<()>(
            fd,
            &[io::IoSlice::new(buf)],
            &[cmsg],
            MsgFlags::empty(),
            None,
        )?;
        Ok(sendmsg)
    }

    pub fn take_fds(&mut self) -> Vec<OwnedFd> {
        std::mem::take(&mut self.fds)
    }
}

impl io::Read for UnixStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.read_with_fd(buf)
    }
}

impl io::Write for UnixStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.inner.write(buf)
    }

    fn write_vectored(&mut self, bufs: &[io::IoSlice<'_>]) -> io::Result<usize> {
        self.inner.write_vectored(bufs)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

impl AsFd for UnixStream {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.inner.as_fd()
    }
}

impl From<OwnedFd> for UnixStream {
    fn from(fd: OwnedFd) -> Self {
        Self::new(fd.into())
    }
}
