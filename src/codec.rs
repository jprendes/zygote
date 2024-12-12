use serde::{Deserialize, Serialize};

mod private {
    pub trait SealedCodec {}

    pub trait SealedCodecAsRef<T> {}
}

pub trait Codec: Serialize + for<'a> Deserialize<'a> + private::SealedCodec {
    fn deserialize(buf: &[u8]) -> Result<Self, rmp_serde::decode::Error> {
        rmp_serde::from_slice(buf)
    }
    fn serialize(&self) -> Result<Vec<u8>, rmp_serde::encode::Error> {
        rmp_serde::to_vec_named(&self)
    }
}

impl<T: Serialize + for<'a> Deserialize<'a>> private::SealedCodec for T {}
impl<T: Serialize + for<'a> Deserialize<'a> + private::SealedCodec> Codec for T {}

pub trait AsCodecRef<T: Codec + ?Sized>: private::SealedCodecAsRef<T> {
    fn as_codec_ref(&self) -> &T;
}

impl<T: Codec + ?Sized> private::SealedCodecAsRef<T> for T {}

impl<T: Codec + ?Sized + private::SealedCodecAsRef<T>> AsCodecRef<T> for T {
    fn as_codec_ref(&self) -> &T {
        self
    }
}

impl<T: Codec + ?Sized> private::SealedCodecAsRef<T> for &T {}

impl<'a, T: Codec + ?Sized> AsCodecRef<T> for &'a T
where
    &'a T: private::SealedCodecAsRef<T>,
{
    fn as_codec_ref(&self) -> &T {
        self
    }
}
