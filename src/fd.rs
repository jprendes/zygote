use std::cell::RefCell;
use std::ops::{Deref, DerefMut};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, IntoRawFd, OwnedFd, RawFd};

use serde::{Deserialize, Serialize};

thread_local! {
    static FDS: RefCell<Vec<Option<RawFd>>> = RefCell::default();
}

pub struct SendableFd<T>(T);

impl<T> From<T> for SendableFd<T> {
    fn from(value: T) -> Self {
        Self(value)
    }
}

impl<T> SendableFd<T> {
    pub fn new(val: T) -> SendableFd<T> {
        val.into()
    }

    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T: AsFd> AsRawFd for SendableFd<T> {
    fn as_raw_fd(&self) -> RawFd {
        self.0.as_fd().as_raw_fd()
    }
}

impl<T: AsFd> AsFd for SendableFd<T> {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
    }
}

impl<T: IntoRawFd> IntoRawFd for SendableFd<T> {
    fn into_raw_fd(self) -> RawFd {
        self.0.into_raw_fd()
    }
}

impl<T: FromRawFd> FromRawFd for SendableFd<T> {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        T::from_raw_fd(fd).into()
    }
}

impl<T> Deref for SendableFd<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> DerefMut for SendableFd<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<T: AsFd> Serialize for SendableFd<T> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let n = push_fd(self);
        Serialize::serialize(&n, serializer)
    }
}

impl<'a, T: FromRawFd> Deserialize<'a> for SendableFd<T> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'a>,
    {
        let n = Deserialize::deserialize(deserializer)?;
        let fd = take_fd(n).expect("expected an FD");
        Ok(Self(unsafe { T::from_raw_fd(fd.into_raw_fd()) }))
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
