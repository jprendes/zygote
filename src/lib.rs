//! `zygote` is a library to create zygote processes on linux.
//! A zygote process is a small process used primarily to create new processes,
//! but can be used for anything that requires running things in a separate process.
//!
//! To learn more about zygote processes check out [these notes on Chromium](https://neugierig.org/software/chromium/notes/2011/08/zygote.html).
//!
//! # Getting started
//! ```rust
//! # use zygote::Zygote;
//! fn main() {
//!     Zygote::init();
//!     let pid = Zygote::global().run(|_| std::process::id(), ());
//!     assert_ne!(pid, std::process::id());
//! }
//! ```

use std::backtrace::Backtrace;
use std::io::ErrorKind;
use std::panic::{catch_unwind, set_hook};
use std::sync::{LazyLock, Mutex};

#[cfg(feature = "anyhow")]
use anyhow::Error as AnyhowError;
use clone3::Clone3;
use codec::{AsCodecRef, Codec};
use error::{Error, WireError};
use fd::SendableFd;
use pipe::Pipe;
use serde::{Deserialize, Serialize};

mod codec;
pub mod error;
pub mod fd;
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
    /// Initialize a new global zygote child process.
    /// The global zygote can be accessed using [`Zygote::global()`].
    ///
    /// Calling this method multiple times does not change the result
    /// beyond the initial call.
    ///
    /// Usually this method would be called very early on in the process
    /// main function. This is to avoid leaving the new process in an
    /// undefined state. In particular, it is highly recommended to run
    /// this method before creating any new thread, as that could leave
    /// the libc inside the new process in an undefined state.
    ///
    /// ```rust
    /// # use zygote::Zygote;
    /// # fn start_multithreaded_tokio_runtime() {}
    /// fn main() {
    ///     // initialize the zygote
    ///     Zygote::init();
    ///
    ///     start_multithreaded_tokio_runtime();    
    ///     // initializing the zygote after this could lead to undefined behavior.
    ///     
    ///     // safely use the pre-initialized zygote
    ///     Zygote::global();
    /// }
    /// ```
    ///
    /// # Panics
    /// Same panic conditions as [`Zygote::new()`].
    pub fn init() {
        Self::global();
    }

    /// Obtain the global zygote process.
    /// This method initializes the global zygote if needed.
    /// ```rust
    /// # use zygote::Zygote;
    /// Zygote::global().run(|_| std::process::id(), ());
    /// ```
    ///
    /// # Panics
    /// If this calls initializes the global zygote, it shares the same
    /// panic conditions as [`Zygote::new()`].
    pub fn global() -> &'static Zygote {
        static ZYGOTE: LazyLock<Zygote> = LazyLock::new(Zygote::new);
        &*ZYGOTE
    }

    /// Create a new child zygote process.
    ///
    /// ```rust
    /// # use zygote::Zygote;
    /// # fn getppid() -> libc::pid_t { unsafe { libc::getppid() } }
    /// # fn getpid() -> libc::pid_t { unsafe { libc::getpid() } }
    /// let zygote = Zygote::new();
    /// let ppid = zygote.run(|_| getppid(), ());
    /// assert_eq!(ppid, getpid()); // zygote is a child of the current process
    /// ```
    ///
    /// The zygote process will inherit the state of the calling thread at the point
    /// it was created.
    ///
    /// ```rust
    /// # use zygote::Zygote;
    /// # use std::sync::atomic::AtomicU32;
    /// # use std::sync::atomic::Ordering::SeqCst;
    /// static VALUE: AtomicU32 = AtomicU32::new(0);
    ///
    /// VALUE.store(42, SeqCst);
    /// let zygote = Zygote::new(); // inherit thread state at this point
    ///
    /// VALUE.store(123, SeqCst);
    /// let n = zygote.run(|_| VALUE.load(SeqCst), ());
    ///
    /// assert_eq!(n, 42); // changes after creation don't affect the zygote
    /// ```
    ///
    /// Inside the zygote process it would be as if any other thread in the process
    /// has suddenly been terminated. This could leave libc in a bad state.
    /// To avoid this it is best to create the zygote while the application is still
    /// single threaded.
    ///
    /// # Panics
    /// This method panics if any of the syscalls (creating a unix domain socket and
    /// cloning the process) fails.
    pub fn new() -> Zygote {
        Self::new_impl(false)
    }

    /// Create a new sibling zygote process.
    /// Like [`Zygote::new()`], but the new process will have the same parent process
    /// as the calling process.
    ///
    /// ```rust
    /// # use zygote::Zygote;
    /// # fn getppid() -> libc::pid_t { unsafe { libc::getppid() } }
    /// # fn getpid() -> libc::pid_t { unsafe { libc::getpid() } }
    /// let zygote = Zygote::new_sibling();
    ///
    /// let ppid = zygote.run(|_| getppid(), ());
    /// assert_eq!(ppid, getppid()); // same parent pid
    ///
    /// let pid = zygote.run(|_| getpid(), ());
    /// assert_ne!(pid, getpid()); // different pid
    /// ```
    ///
    /// # Panics
    /// Same panic conditions as [`Zygote::new()`].
    pub fn new_sibling() -> Zygote {
        Self::new_impl(true)
    }

    fn new_impl(sibling: bool) -> Zygote {
        let (child_pipe, parent_pipe) = Pipe::pair().unwrap();
        let mut clone3 = Clone3::default();
        if sibling {
            clone3.flag_parent();
        } else {
            clone3.exit_signal(libc::SIGCHLD as _);
        }
        match unsafe { clone3.call() }.unwrap() {
            0 => {
                drop(parent_pipe);
                zygote_start(child_pipe);
                // unreachable
            }
            _child_pid => {
                drop(child_pipe);
                let pipe = Mutex::new(SendableFd::from(parent_pipe));
                return Self { pipe };
            }
        }
    }

    /// Run a task in the zygote process.
    ///
    /// ```rust
    /// # use zygote::Zygote;
    /// let zygote = Zygote::new();
    /// let pid = zygote.run(|_| std::process::id(), ());
    /// assert_ne!(pid, std::process::id());
    /// ```
    ///
    /// The task must receive a single argument.
    /// If you need more than one argument, use a tuple.
    ///
    /// The argument type `Args` and the return type `Ret` must both be serializable
    /// (and deserializable) with [serde].
    /// If you want to run a fallible function that returns a [`Result`](core::result::Result)
    /// consider using [`WireError`] as the error type, which is serializable.
    ///
    /// The arguments can be moved or passed by reference. This means this method accepts
    /// either `Args` or `&Args`.
    /// ```rust
    /// # use zygote::Zygote;
    /// # let zygote = Zygote::new();
    /// let x = zygote.run(|x: u32| x*2, 4); // this is ok
    /// assert_eq!(x, 8);
    ///
    /// let x = zygote.run(|x: u32| x*2, &4); // this is also ok
    /// assert_eq!(x, 8);
    /// ```
    ///
    /// # Panics
    /// This method panics if communication with the zygote fails or
    /// if the task itself panics.
    /// For a non panicing version of this method see [`Zygote::try_run()`].
    pub fn run<Args: Codec, Ret: Codec>(
        &self,
        f: fn(Args) -> Ret,
        args: impl AsCodecRef<Args>,
    ) -> Ret {
        self.try_run(f, args).unwrap()
    }

    /// Run a task in the zygote process.
    /// Like [`Zygote::run()`], but the return value is a [`Result`](core::result::Result)
    /// that will error if the task panics or communication with the zygote fails.
    /// ```rust
    /// # use zygote::Zygote;
    /// # let zygote = Zygote::new();
    /// let res = zygote.try_run(|_| 123, ()).unwrap();
    /// assert_eq!(res, 123);
    ///
    /// let res = zygote.try_run(|_| panic!("oops"), ()).unwrap_err();
    /// assert!(res.to_string().contains("oops"));
    /// ```
    pub fn try_run<Args: Codec, Ret: Codec>(
        &self,
        f: fn(Args) -> Ret,
        args: impl AsCodecRef<Args>,
    ) -> Result<Ret, Error> {
        let mut pipe = self.pipe.lock().unwrap();
        let runner = runner::<Args, Ret> as usize;
        let f = f as usize;
        pipe.send(&[f, runner])?;
        pipe.send(args.as_codec_ref())?;
        Ok(pipe.recv::<Result<_, WireError>>()??)
    }

    /// Create a new zygote process from within this zygote process.
    /// The new zygote process will be a child of this zygote process.
    ///
    /// This is useful when you want to create a new zygote but the current
    /// process is not in a state where doing that would be safe.
    /// The new zygote inherits the state of the main thread of the first
    /// zygote process.
    ///
    /// ```rust
    /// # use zygote::Zygote;
    /// # let zygote = Zygote::new();
    /// # fn getppid() -> libc::pid_t { unsafe { libc::getppid() } }
    /// # fn getpid() -> libc::pid_t { unsafe { libc::getpid() } }
    /// let pid = zygote.run(|_| getpid(), ());
    ///
    /// let zygote2 = zygote.spawn();
    /// let ppid = zygote2.run(|_| getppid(), ());
    ///
    /// assert_eq!(ppid, pid); // zygote2 is a child of zygote
    /// ```
    ///
    /// # Panics
    /// Same panic conditions as [`Zygote::new()`].
    pub fn spawn(&self) -> Zygote {
        self.spawn_impl(false).unwrap()
    }

    /// Create a new zygote process from within this zygote process.
    /// Like [`Zygote::spawn()`], but the new process will have the same parent process
    /// as the current zygote.
    ///
    /// ```rust
    /// # use zygote::Zygote;
    /// # let zygote = Zygote::new();
    /// # fn getppid() -> libc::pid_t { unsafe { libc::getppid() } }
    /// # fn getpid() -> libc::pid_t { unsafe { libc::getpid() } }
    /// let ppid = zygote.run(|_| getppid(), ());
    ///
    /// let zygote2 = zygote.spawn_sibling();
    /// let ppid2 = zygote2.run(|_| getppid(), ());
    ///
    /// assert_eq!(ppid, ppid2); // zygote2 and zygote share the same parent
    /// ```
    ///
    /// # Panics
    /// Same panic conditions as [`Zygote::new()`].
    pub fn spawn_sibling(&self) -> Zygote {
        self.spawn_impl(true).unwrap()
    }

    fn spawn_impl(&self, sibling: bool) -> Result<Zygote, Error> {
        self.try_run(spawner, &sibling)
    }
}

// The pipe used by this zygote process.
// We don't need a mutex as the pipe will always be accessed from one single threaded.
static mut PIPE: Option<Pipe> = None;
fn pipe() -> &'static mut Pipe {
    unsafe { PIPE.as_mut().unwrap() }
}

fn runner<Args: Codec, Ret: Codec>(f: usize) -> Result<(), Error>
where
    Result<Ret, WireError>: Codec,
{
    catch_unwind(|| -> Result<(), Error> {
        let f: fn(Args) -> Ret = unsafe { std::mem::transmute(f) };
        let args = pipe().recv::<Args>()?;
        pipe().send::<Result<Ret, WireError>>(&Ok(f(args)))?;
        Ok(())
    })
    .unwrap_or(Ok(()))
}

fn spawner(sibling: bool) -> Zygote {
    Zygote::new_impl(sibling)
}

fn zygote_start(pipe: Pipe) -> ! {
    match zygote_main(pipe) {
        Ok(()) => std::process::exit(0),
        Err(Error::Io(err)) if err.kind() == ErrorKind::UnexpectedEof => {
            std::process::exit(0);
        }
        Err(error) => {
            #[cfg(feature = "log")]
            log::warn!("zygote exited with error: {error:?}");
            drop(error); // silence warning if log feature is disabled
            std::process::exit(1);
        }
    }
}

fn zygote_main(p: Pipe) -> Result<(), Error> {
    unsafe { PIPE = Some(p) };

    set_hook(Box::new(|info| {
        let backtrace = Backtrace::capture();
        let error = WireError::from_panic(info, &backtrace);
        #[cfg(feature = "log")]
        log::warn!("zygote task panic: {error:?}");
        let _ = pipe().send::<Result<(), WireError>>(&Err(error));
    }));

    loop {
        let [f, runner] = pipe().recv::<[usize; 2]>()?;
        let runner: fn(usize) -> Result<(), Error> = unsafe { std::mem::transmute(runner) };
        runner(f)?;
    }
}
