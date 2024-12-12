use std::backtrace::Backtrace;
use std::io::ErrorKind;
use std::panic::{catch_unwind, set_hook};
use std::sync::{LazyLock, Mutex};

#[cfg(feature = "anyhow")]
use anyhow::Error as AnyhowError;
use clone3::Clone3;
use codec::Codec;
use error::{Error, WireError};
use fds::SendableFd;
use pipe::Pipe;
use serde::{Deserialize, Serialize};

mod codec;
pub mod error;
#[doc(hidden)]
pub mod fds;
mod pipe;

#[cfg(feature = "anyhow")]
type Result<T, E = AnyhowError> = std::result::Result<T, E>;

#[cfg(not(feature = "anyhow"))]
type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Serialize, Deserialize)]
pub struct Zygote {
    pipe: Mutex<SendableFd<Pipe>>,
}

impl Zygote {
    pub fn init() {
        Self::global();
    }

    pub fn global() -> &'static Self {
        static ZYGOTE: LazyLock<Zygote> = LazyLock::new(Zygote::new);
        &*ZYGOTE
    }

    pub fn new() -> Self {
        let (child_pipe, parent_pipe) = Pipe::pair().unwrap();
        let mut clone3 = Clone3::default();
        clone3.exit_signal(libc::SIGCHLD as _);
        match unsafe { clone3.call() }.unwrap() {
            0 => {
                drop(parent_pipe);
                match zygote_main(child_pipe) {
                    Ok(()) => std::process::exit(0),
                    Err(Error::Io(err)) if err.kind() == ErrorKind::UnexpectedEof => {
                        std::process::exit(0);
                    }
                    Err(err) => {
                        log::warn!("zygote exited: {err}");
                        std::process::exit(1);
                    }
                }
            }
            _child_pid => {
                drop(child_pipe);
                return Self {
                    pipe: Mutex::new(SendableFd(parent_pipe)),
                };
            }
        }
    }

    pub fn run<Args: Codec, Res: Codec>(
        &self,
        f: fn(Args) -> Res,
        args: &Args,
    ) -> Result<Res, Error> {
        let mut pipe = self.pipe.lock().unwrap();
        let runner = runner::<Args, Res> as usize;
        let f = f as usize;
        pipe.send(&[f, runner])?;
        pipe.send(args)?;
        Ok(pipe.recv::<Result<_, WireError>>()??)
    }

    pub fn spawn(&self) -> Result<Zygote, Error> {
        fn spawner(_: ()) -> Zygote {
            Zygote::new()
        }
        self.run(spawner, &())
    }
}

/*
impl Serialize for Zygote {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
        where
            S: serde::Serializer {
        let fd = self.pipe.lock().unwrap();
        let fd = *fd;
    }
}
*/

/*
impl<E: Codec> Codec for Result<Zygote, E>
where
    Result<(), E>: Codec
{
    fn serialize(&self) -> Result<(Vec<u8>, Vec<BorrowedFd>), rmp_serde::encode::Error> {
        match self {
            Ok(zygote) => {
                let (bytes, fds) = zygote.serialize()?;
                let bytes = [0].into_iter().chain(bytes.into_iter()).collect();
                Ok((bytes, fds))
            }
            Err(err) => {
                let (bytes, fds) = Result::<(), E>::serialize(&Err(*err))?;
                let bytes = [1].into_iter().chain(bytes.into_iter()).collect();
                Ok((bytes, fds))
            }
        }
    }
}
*/

// We don't need a mutex as the process is single threaded
static mut PIPE: Option<Pipe> = None;
fn pipe() -> &'static mut Pipe {
    unsafe { PIPE.as_mut().unwrap() }
}

fn runner<Args: Codec, Res: Codec>(f: usize) -> Result<(), Error>
where
    Result<Res, WireError>: Codec,
{
    catch_unwind(|| -> Result<(), Error> {
        let f: fn(Args) -> Res = unsafe { std::mem::transmute(f) };
        let args = pipe().recv::<Args>()?;
        pipe().send::<Result<Res, WireError>>(&Ok(f(args)))?;
        Ok(())
    })
    .unwrap_or(Ok(()))
}

fn zygote_main(p: Pipe) -> Result<(), Error> {
    unsafe { PIPE = Some(p) };

    set_hook(Box::new(|info| {
        let backtrace = Backtrace::capture();
        let error = WireError::from_panic(info, &backtrace);
        let _ = pipe().send::<Result<(), WireError>>(&Err(error));
    }));

    loop {
        let [f, runner] = pipe().recv::<[usize; 2]>()?;
        let runner: fn(usize) -> Result<(), Error> = unsafe { std::mem::transmute(runner) };
        runner(f)?;
    }
}
