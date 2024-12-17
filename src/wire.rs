use std::any::Any;
use std::panic::UnwindSafe;

use serde::de::DeserializeOwned;
use serde::Serialize;

pub trait Wire: Serialize + DeserializeOwned + Any + UnwindSafe {
    fn deserialize(buf: &[u8]) -> Result<Self, rmp_serde::decode::Error> {
        rmp_serde::from_slice(buf)
    }
    fn serialize(&self) -> Result<Vec<u8>, rmp_serde::encode::Error> {
        let mut buf = vec![];
        self.serialize_into(&mut buf)?;
        Ok(buf)
    }
    fn serialize_into(&self, buf: &mut Vec<u8>) -> Result<(), rmp_serde::encode::Error> {
        rmp_serde::encode::write_named(buf,self)
    }
}

impl<T: Serialize + DeserializeOwned + Any + UnwindSafe> Wire for T {}

pub trait AsWire<T: Wire> {
    fn deserialize(buf: &[u8]) -> Result<T, rmp_serde::decode::Error> {
        rmp_serde::from_slice(buf)
    }
    fn serialize(&self) -> Result<Vec<u8>, rmp_serde::encode::Error> {
        let mut buf = vec![];
        self.serialize_into(&mut buf)?;
        Ok(buf)
    }
    fn serialize_into(&self, buf: &mut Vec<u8>) -> Result<(), rmp_serde::encode::Error>;
}

impl<T: Wire> AsWire<T> for T {
    fn serialize_into(&self, buf: &mut Vec<u8>) -> Result<(), rmp_serde::encode::Error> {
        <T as Wire>::serialize_into(self, buf)
    }
}

impl<T: Wire> AsWire<T> for &T {
    fn serialize_into(&self, buf: &mut Vec<u8>) -> Result<(), rmp_serde::encode::Error> {
        <T as Wire>::serialize_into(self, buf)
    }
}

impl AsWire<String> for str {
    fn serialize_into(&self, buf: &mut Vec<u8>) -> Result<(), rmp_serde::encode::Error> {
        rmp_serde::encode::write_named(buf,self)
    }
}

impl AsWire<String> for &str {
    fn serialize_into(&self, buf: &mut Vec<u8>) -> Result<(), rmp_serde::encode::Error> {
        rmp_serde::encode::write_named(buf,self)
    }
}
