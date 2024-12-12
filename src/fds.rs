use std::cell::RefCell;
use std::collections::VecDeque;
use std::ops::{Deref, DerefMut};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, IntoRawFd, OwnedFd, RawFd};

use serde::{Deserialize, Serialize};

thread_local! {
    static FDS: RefCell<VecDeque<RawFd>> = RefCell::default();
}

pub struct SendableFd<T: AsFd + FromRawFd>(pub T);

impl<T: AsFd + FromRawFd> AsRawFd for SendableFd<T> {
    fn as_raw_fd(&self) -> RawFd {
        self.as_fd().as_raw_fd()
    }
}

impl<T: AsFd + FromRawFd> AsFd for SendableFd<T> {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
    }
}

impl<T: AsFd + FromRawFd> FromRawFd for SendableFd<T> {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        Self(T::from_raw_fd(fd))
    }
}

impl<T: AsFd + FromRawFd> Deref for SendableFd<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T: AsFd + FromRawFd> DerefMut for SendableFd<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<T: AsFd + FromRawFd> Serialize for SendableFd<T> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        push_fd(self);
        Serialize::serialize(&(), serializer)
    }
}

impl<'a, T: AsFd + FromRawFd> Deserialize<'a> for SendableFd<T> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'a>,
    {
        let _: () = Deserialize::deserialize(deserializer)?;
        let fd = pop_fd().expect("expected an FD");
        Ok(Self(unsafe { T::from_raw_fd(fd.into_raw_fd()) }))
    }
}

pub(crate) fn swap_fds(fds: Vec<RawFd>) -> Vec<RawFd> {
    FDS.with_borrow_mut(|old_fds| std::mem::replace(old_fds, fds.into()))
        .into()
}

fn push_fd(fd: &impl AsFd) {
    FDS.with_borrow_mut(|fds| fds.push_back(fd.as_fd().as_raw_fd()));
}

fn pop_fd() -> Option<OwnedFd> {
    FDS.with_borrow_mut(|fds| fds.pop_front())
        .map(|fd| unsafe { OwnedFd::from_raw_fd(fd) })
}
