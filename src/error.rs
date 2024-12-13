use std::backtrace::{Backtrace, BacktraceStatus};
use std::error::Error as StdError;
use std::fmt::{Debug, Display, Formatter, Result as FmtResult};
use std::ops::{Deref, DerefMut};
use std::panic::PanicHookInfo;

use serde::{Deserialize, Serialize};

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("decode error: {0}")]
    Decode(#[from] rmp_serde::decode::Error),

    #[error("encode error: {0}")]
    Encode(#[from] rmp_serde::encode::Error),

    #[error("wire error: {0}")]
    Wire(#[from] WireError),
}

#[derive(Serialize, Deserialize, Clone)]
#[repr(transparent)]
pub struct WireError(WireErrorInner);

#[derive(Serialize, Deserialize, Clone)]
struct WireErrorInner {
    pub(crate) description: String,
    pub(crate) source: Option<Box<WireErrorInner>>,
    pub(crate) backtrace: Option<String>,
}

impl WireErrorInner {
    fn as_wire_error(&self) -> &WireError {
        unsafe { std::mem::transmute(self) }
    }

    fn into_wire_error(self) -> WireError {
        unsafe { std::mem::transmute(self) }
    }
}

impl WireError {
    pub fn source(&self) -> Option<&WireError> {
        self.0.source.as_ref().map(|s| s.as_wire_error())
    }

    pub fn description(&self) -> &str {
        &self.0.description
    }

    pub fn backtrace(&self) -> Option<&str> {
        self.0.backtrace.as_ref().map(|s| s.as_str())
    }
}

impl WireError {
    pub(crate) fn from_str(err: impl AsRef<str>) -> Self {
        WireErrorInner {
            description: err.as_ref().to_owned(),
            source: None,
            backtrace: None,
        }
        .into_wire_error()
    }

    pub(crate) fn from_err(err: &(impl StdError + ?Sized)) -> Self {
        WireErrorInner {
            description: err.to_string(),
            source: err.source().map(|src| Box::new(Self::from_err(src).0)),
            backtrace: None,
        }
        .into_wire_error()
    }

    pub(crate) fn from_panic(info: &PanicHookInfo, backtrace: &Backtrace) -> Self {
        WireErrorInner {
            description: info.to_string(),
            source: None,
            backtrace: (backtrace.status() == BacktraceStatus::Captured)
                .then_some(backtrace.to_string()),
        }
        .into_wire_error()
    }
}

impl AsRef<dyn StdError> for WireError {
    fn as_ref(&self) -> &(dyn StdError + 'static) {
        &self.0
    }
}

impl AsRef<dyn StdError + Send + Sync> for WireError {
    fn as_ref(&self) -> &(dyn StdError + Send + Sync + 'static) {
        &self.0
    }
}

impl Deref for WireError {
    type Target = dyn StdError + Send + Sync;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for WireError {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<E: StdError> From<E> for WireError {
    fn from(err: E) -> Self {
        Self::from_err(&err)
    }
}

impl From<WireError> for Box<dyn StdError + 'static> {
    fn from(value: WireError) -> Self {
        Box::new(value.0)
    }
}

impl From<WireError> for Box<dyn StdError + Send + 'static> {
    fn from(value: WireError) -> Self {
        Box::new(value.0)
    }
}

impl From<WireError> for Box<dyn StdError + Send + Sync + 'static> {
    fn from(value: WireError) -> Self {
        Box::new(value.0)
    }
}

impl StdError for WireErrorInner {
    fn source(&self) -> Option<&(dyn 'static + StdError)> {
        self.source.as_ref().map(|s| s.as_wire_error().as_ref())
    }

    fn description(&self) -> &str {
        &self.description
    }
}

impl Display for WireErrorInner {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        write!(f, "{}", self.description)
    }
}

impl Debug for WireErrorInner {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        write!(f, "{}", self.description)?;
        if let Some(backtrace) = &self.backtrace {
            write!(f, "\n\n{backtrace}")?
        }
        Ok(())
    }
}

impl Display for WireError {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        write!(f, "{}", self.0)
    }
}

impl Debug for WireError {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        write!(f, "{}", self.0)
    }
}
