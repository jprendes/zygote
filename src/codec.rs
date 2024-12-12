use serde::{Deserialize, Serialize};

pub trait Codec: Serialize + for<'a> Deserialize<'a> {
    fn deserialize(buf: &[u8]) -> Result<Self, rmp_serde::decode::Error> {
        rmp_serde::from_slice(buf)
    }
    fn serialize(&self) -> Result<Vec<u8>, rmp_serde::encode::Error> {
        rmp_serde::to_vec_named(&self)
    }
}

impl<T: Serialize + for<'a> Deserialize<'a>> Codec for T {}
