use std::io::{Read, Write};
use std::os::fd::{FromRawFd, IntoRawFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::sync::LazyLock;

use zygote::error::WireError;
use zygote::fds::SendableFd;
use zygote::Zygote;

static NAME: LazyLock<String> = LazyLock::new(|| String::from("Zygote"));
const HELLO_WORLD: &[u8] = b"hello world!";

fn say_hi(name: String) -> String {
    format!("hello {name}")
}

fn say_hi_panic(_: String) -> String {
    panic!("that didn't work")
}

fn does_error(_: ()) -> Result<(), WireError> {
    Err(std::io::Error::other("some wire error"))?;
    Ok(())
}

fn write_to_fd(p: SendableFd<OwnedFd>) {
    let mut p = unsafe { UnixStream::from_raw_fd(p.0.into_raw_fd()) };
    p.write_all(HELLO_WORLD).unwrap();
}

#[test]
fn task_success() {
    let res = Zygote::global().run(say_hi, &*NAME);
    assert_eq!(res, "hello Zygote");
}

#[test]
fn task_failure() {
    let res = Zygote::global().try_run(say_hi_panic, &*NAME).unwrap_err();
    assert!(res.to_string().contains("panic"));
}

#[test]
fn many_calls() {
    Zygote::global().try_run(say_hi_panic, &*NAME).unwrap_err();
    Zygote::global().try_run(say_hi_panic, &*NAME).unwrap_err();
    Zygote::global().try_run(say_hi, &*NAME).unwrap();
    Zygote::global().try_run(say_hi, &*NAME).unwrap();
    Zygote::global().try_run(say_hi_panic, &*NAME).unwrap_err();
    Zygote::global().try_run(say_hi, &*NAME).unwrap();
}

#[test]
fn large_payload() {
    let payload: Vec<u32> = (0..1024 * 1024).into_iter().collect();
    let res = Zygote::global().run(|v| v, &payload);
    assert_eq!(res, payload);
}

#[test]
fn send_fd() {
    let (p1, mut p2) = UnixStream::pair().unwrap();
    let p1 = SendableFd(p1.into());
    Zygote::global().run(write_to_fd, p1);
    let mut msg = vec![0; size_of_val(HELLO_WORLD)];
    p2.read_exact(&mut msg).unwrap();
    assert_eq!(&msg, HELLO_WORLD);
}

#[test]
fn wire_error() {
    let err = Zygote::global().run(does_error, ()).unwrap_err();
    assert!(err.to_string().contains("some wire error"));
}

#[test]
fn nested_zygote() {
    let pid = std::process::id();
    let zyg_pid = Zygote::global().run(|_| std::process::id(), ());

    assert_ne!(pid, zyg_pid);

    let zygote = Zygote::global().spawn();
    let zygzyg_pid = zygote.run(|_| std::process::id(), ());

    assert_ne!(pid, zygzyg_pid);
    assert_ne!(zyg_pid, zygzyg_pid);
}
