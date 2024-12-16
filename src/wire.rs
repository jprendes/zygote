use std::panic::UnwindSafe;

use serde::de::DeserializeOwned;
use serde::Serialize;

pub trait Wire: Serialize + DeserializeOwned + UnwindSafe {
    fn deserialize(buf: &[u8]) -> Result<Self, rmp_serde::decode::Error> {
        rmp_serde::from_slice(buf)
    }
    fn serialize(&self) -> Result<Vec<u8>, rmp_serde::encode::Error> {
        rmp_serde::to_vec_named(&self)
    }
}

impl<T: Serialize + DeserializeOwned + UnwindSafe> Wire for T {}

pub trait AsWire<T: Wire> {
    fn deserialize(buf: &[u8]) -> Result<T, rmp_serde::decode::Error> {
        rmp_serde::from_slice(buf)
    }
    fn serialize(&self) -> Result<Vec<u8>, rmp_serde::encode::Error>;
}

impl<T: Wire> AsWire<T> for T {
    fn serialize(&self) -> Result<Vec<u8>, rmp_serde::encode::Error> {
        <T as Wire>::serialize(self)
    }
}

impl<T: Wire> AsWire<T> for &T {
    fn serialize(&self) -> Result<Vec<u8>, rmp_serde::encode::Error> {
        <T as Wire>::serialize(self)
    }
}

impl AsWire<String> for str {
    fn serialize(&self) -> Result<Vec<u8>, rmp_serde::encode::Error> {
        rmp_serde::to_vec_named(self)
    }
}

impl AsWire<String> for &str {
    fn serialize(&self) -> Result<Vec<u8>, rmp_serde::encode::Error> {
        rmp_serde::to_vec_named(self)
    }
}
