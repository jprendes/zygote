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
use std::cell::Cell;
use std::io;
use std::io::ErrorKind::{BrokenPipe, UnexpectedEof};
use std::mem::transmute;
use std::os::fd::{AsFd as _, AsRawFd, FromRawFd, OwnedFd};
use std::panic::{catch_unwind, set_hook, take_hook};
use std::sync::{LazyLock, Mutex};

pub use error::{Error, WireError};
pub use fd::WireFd;
use libc::{CLONE_PARENT, SIGCHLD, SIGKILL};
use nix::sched::CloneFlags;
use nix::sys::wait::{waitid, Id, WaitPidFlag};
use pipe::Pipe;
use serde::{Deserialize, Serialize};
use wire::{AsWire, Wire};

mod error;
mod fd;
mod pipe;
mod wire;

/// Representation of a zygote process
///
/// This structure is used to represent zygote process and to run tasks
/// in it.
///
/// Dropping this struct will result in the termination of the zygote
/// process.
#[repr(transparent)]
pub struct Zygote(ZygoteImpl);

#[derive(Serialize, Deserialize)]
struct ZygoteImpl {
    pidfd: WireFd<OwnedFd>,
    pipe: Mutex<WireFd<Pipe>>,
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
        &ZYGOTE
    }

    /// Create a new zygote process. The zygote process will be a child
    /// of the calling process.
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

    fn new_impl(sibling: bool) -> Zygote {
        let (child_pipe, parent_pipe) = Pipe::pair().unwrap();
        let pidfd = if sibling {
            clone3_or_clone(CLONE_PARENT, 0).unwrap()
        } else {
            clone3_or_clone(0, SIGCHLD).unwrap()
        };
        match pidfd {
            None => {
                drop(parent_pipe);
                zygote_start(child_pipe);
                // unreachable
            }
            Some(pidfd) => {
                drop(child_pipe);
                let pidfd = WireFd::new(pidfd);
                let pipe = Mutex::new(WireFd::new(parent_pipe));
                Zygote(ZygoteImpl { pidfd, pipe })
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
    /// If you want to run a fallible function that returns a [`Result`] consider using
    /// [`WireError`] as the error type, which is serializable.
    /// If you want to pass a file descriptor, consider using [`WireFd`].
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
    /// For a non panicking version of this method see [`Zygote::try_run()`].
    pub fn run<Args: Wire, Ret: Wire>(&self, f: fn(Args) -> Ret, args: impl AsWire<Args>) -> Ret {
        self.try_run(f, args).unwrap()
    }

    /// Run a task in the zygote process.
    /// Like [`Zygote::run()`], but the return value is a [`Result`] that will
    /// error if the task panics or communication with the zygote fails.
    /// ```rust
    /// # use zygote::Zygote;
    /// # let zygote = Zygote::new();
    /// let res = zygote.try_run(|_| 123, ()).unwrap();
    /// assert_eq!(res, 123);
    ///
    /// let res = zygote.try_run(|_| panic!("oops"), ()).unwrap_err();
    /// assert!(res.to_string().contains("oops"));
    /// ```
    pub fn try_run<Args: Wire, Ret: for<'b> Wire>(
        &self,
        f: fn(Args) -> Ret,
        args: impl AsWire<Args>,
    ) -> Result<Ret, Error> {
        let mut pipe = self.0.pipe.lock().unwrap();
        let runner = runner::<Args, Ret> as usize;
        let f = f as usize;
        pipe.send([f, runner])?;
        pipe.send(args)?;
        Ok(pipe.recv::<Result<_, WireError>>()??)
    }

    /// Create a new zygote process from within this zygote process.
    /// The new zygote process will be a sibling of the current zygote,
    /// i.e., they will have the same parent process.
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
    /// let ppid = zygote.run(|_| getppid(), ());
    ///
    /// let zygote2 = zygote.spawn();
    /// let ppid2 = zygote2.run(|_| getppid(), ());
    ///
    /// assert_eq!(ppid, ppid2); // zygote2 and zygote share the same parent
    /// ```
    ///
    /// # Panics
    /// Same panic conditions as [`Zygote::new()`].
    pub fn spawn(&self) -> Zygote {
        let inner = self.try_run(spawner, ()).unwrap();
        Zygote(inner)
    }
}

impl Default for Zygote {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Zygote {
    fn drop(&mut self) {
        let pidfd = self.0.pidfd.as_raw_fd();
        unsafe { libc::syscall(libc::SYS_pidfd_send_signal, pidfd, SIGKILL, 0, 0) };
        let pidfd = self.0.pidfd.as_fd();
        let _ = waitid(Id::PIDFd(pidfd), WaitPidFlag::WEXITED);
    }
}

fn clone3_or_clone(flags: i32, exit_signal: i32) -> io::Result<Option<OwnedFd>> {
    #[cfg(feature = "clone3")]
    if let Ok(res) = clone3(flags, exit_signal) {
        return Ok(res);
    }
    clone(flags, exit_signal)
}

#[cfg(feature = "clone3")]
fn clone3(flags: i32, exit_signal: i32) -> io::Result<Option<OwnedFd>> {
    let mut args = [flags as u64, 0, 0, 0, exit_signal as u64, 0, 0, 0, 0, 0, 0];
    let args_ptr = std::ptr::from_mut(&mut args);
    let args_size = std::mem::size_of_val(&args);
    let res = unsafe { libc::syscall(libc::SYS_clone3, args_ptr, args_size) };
    match res {
        0 => Ok(None),
        pid @ 1.. => {
            let pidfd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
            let pidfd = unsafe { OwnedFd::from_raw_fd(pidfd as _) };
            Ok(Some(pidfd))
        }
        -1 => Err(io::Error::last_os_error()),
        _ => Err(io::Error::other("unknown")),
    }
}

fn clone(flags: i32, exit_signal: i32) -> io::Result<Option<OwnedFd>> {
    // For sjlj information see: https://llvm.org/docs/ExceptionHandling.html#llvm-eh-sjlj-setjmp
    let mut jmp_buf = [0u16; 128];
    extern "C" {
        #[link_name = "setjmp"]
        fn setjmp(buf: *mut u16) -> i32;
        #[link_name = "longjmp"]
        fn longjmp(buf: *mut u16, val: i32) -> !;
    }
    let res = unsafe { setjmp(jmp_buf.as_mut_ptr()) };
    match res {
        0 => {}
        1 => return Ok(None),
        _ => return Err(io::Error::other("unknown")),
    }
    let f = Box::new(|| {
        unsafe { longjmp(jmp_buf.as_mut_ptr(), 1) };
    });
    // we need a stack just big enough to reach the longjmp
    let mut stack = [0; 1024];
    let pid = unsafe {
        nix::sched::clone(
            f,
            &mut stack,
            CloneFlags::from_bits(flags).unwrap(),
            Some(exit_signal),
        )
    }?;
    let pidfd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid.as_raw(), 0) };
    let pidfd = unsafe { OwnedFd::from_raw_fd(pidfd as _) };
    return Ok(Some(pidfd));
}

fn spawner(_: ()) -> ZygoteImpl {
    let zygote = Zygote::new_impl(true);
    unsafe { transmute(zygote) }
}

fn zygote_start(pipe: Pipe) -> ! {
    match zygote_main(pipe) {
        Ok(()) => std::process::exit(0),
        Err(Error::Io(err)) if err.kind() == UnexpectedEof || err.kind() == BrokenPipe => {
            std::process::exit(0);
        }
        Err(_) => {
            std::process::exit(1);
        }
    }
}

thread_local! {
    static PANIC_ERROR: Cell<Option<WireError>> = const { Cell::new(None) };
}

fn set_panic(error: WireError) {
    PANIC_ERROR.set(Some(error));
}

fn take_panic() -> WireError {
    PANIC_ERROR
        .take()
        .unwrap_or_else(|| WireError::from_str("panic information not found"))
}

fn zygote_main(mut pipe: Pipe) -> Result<(), Error> {
    let panic_hook = take_hook();
    set_hook(Box::new(move |info| {
        let backtrace = Backtrace::capture();
        let error = WireError::from_panic(info, &backtrace);
        set_panic(error);
        panic_hook(info);
    }));

    loop {
        let [f, runner] = pipe.recv::<[usize; 2]>()?;
        let runner: fn(&mut Pipe, usize) -> Result<(), Error> = unsafe { transmute(runner) };
        runner(&mut pipe, f)?;
    }
}

fn runner<Args: Wire, Ret: Wire>(pipe: &mut Pipe, f: usize) -> Result<(), Error>
where
    Result<Ret, WireError>: Wire,
{
    let f: fn(Args) -> Ret = unsafe { transmute(f) };
    let args = pipe.recv_delayed()?;
    let res = catch_unwind(move || -> Result<Ret, WireError> {
        let args = args.deserialize::<Args>()?;
        Ok(f(args))
    })
    .unwrap_or_else(|_| Err(take_panic()));
    pipe.send(res)?;
    Ok(())
}
