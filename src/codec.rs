use std::panic::UnwindSafe;

use serde::{Deserialize, Serialize};

mod sealed {
    pub trait Codec {}
    pub trait AsCodecRef<T> {}
}

pub trait Codec: Serialize + for<'a> Deserialize<'a> + UnwindSafe + sealed::Codec {
    fn deserialize(buf: &[u8]) -> Result<Self, rmp_serde::decode::Error> {
        rmp_serde::from_slice(buf)
    }
    fn serialize(&self) -> Result<Vec<u8>, rmp_serde::encode::Error> {
        rmp_serde::to_vec_named(&self)
    }
}

impl<T: Serialize + for<'a> Deserialize<'a>> sealed::Codec for T {}
impl<T: Serialize + for<'a> Deserialize<'a> + UnwindSafe + sealed::Codec> Codec for T {}

pub trait AsCodecRef<T: Codec + ?Sized>: sealed::AsCodecRef<T> {
    fn as_codec_ref(&self) -> &T;
}

impl<T: Codec + ?Sized> sealed::AsCodecRef<T> for T {}

impl<T: Codec + ?Sized + sealed::AsCodecRef<T>> AsCodecRef<T> for T {
    fn as_codec_ref(&self) -> &T {
        self
    }
}

impl<T: Codec + ?Sized> sealed::AsCodecRef<T> for &T {}

impl<'a, T: Codec + ?Sized> AsCodecRef<T> for &'a T
where
    &'a T: sealed::AsCodecRef<T>,
{
    fn as_codec_ref(&self) -> &T {
        self
    }
}
