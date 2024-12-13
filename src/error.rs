use std::backtrace::{Backtrace, BacktraceStatus};
use std::error::Error as StdError;
use std::fmt::{Debug, Display, Formatter, Result as FmtResult};
use std::mem::transmute;
use std::ops::{Deref, DerefMut};
use std::panic::PanicHookInfo;

use serde::{Deserialize, Serialize};

/// Error type used by [`Zygote::try_run()`](crate::Zygote::try_run) when running a task.
#[derive(thiserror::Error, Debug)]
pub enum Error {
    /// Error during an IO operation
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// Error deserializing the task result
    #[error("decode error: {0}")]
    Decode(#[from] rmp_serde::decode::Error),

    /// Error serializing the task arguments
    #[error("encode error: {0}")]
    Encode(#[from] rmp_serde::encode::Error),

    /// Error originating in the zygote process, including task panics.
    #[error("wire error: {0}")]
    Wire(#[from] WireError),
}

/// A serializable error type.
///
/// To run a fallible task that returns a [`Result`], you need to make
/// sure that both variants ([`Ok`] and [`Err`]) are serializable.
/// For example, [`std::io::Error`] is not serializable.
///
/// ```rust,compile_fail
/// # use std::fs;
/// # use std::io;
/// # use zygote::Zygote;
/// # use zygote::error::WireError;
/// Zygote::global().run(|_| -> Result<String, io::Error> {
///     let msg = fs::read_to_string("message.txt")?;
///     Ok(msg)
/// }, ());
/// ```
///
/// [`WireError`] is an error type that can be serialized.
/// It can be converted from any type that implements the
/// [`core::error::Error`] trait (similar to `anyhow::Error`),
/// making it a good catch-all error type.
///
/// ```rust
/// # use std::fs;
/// # use zygote::Zygote;
/// # use zygote::error::WireError;
/// Zygote::global().run(|_| -> Result<String, WireError> {
///     let msg = fs::read_to_string("message.txt")?;
///     Ok(msg)
/// }, ());
/// ```
///
/// This is also the error type used by [`Zygote::try_run()`](crate::Zygote::try_run) to
/// signal panics during the task execution.
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
        unsafe { transmute(self) }
    }

    fn into_wire_error(self) -> WireError {
        unsafe { transmute(self) }
    }
}

impl WireError {
    /// Returns the lower-level source of this error, if any.
    pub fn source(&self) -> Option<&WireError> {
        self.0.source.as_ref().map(|s| s.as_wire_error())
    }

    /// Get the description of this error.
    pub fn description(&self) -> &str {
        &self.0.description
    }

    /// Get the backtrace for this error, if any.
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
