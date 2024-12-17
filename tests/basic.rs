use std::io::{read_to_string, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;

use zygote::error::WireError;
use zygote::fd::SendableFd;
use zygote::Zygote;

fn getppid() -> u32 {
    unsafe { libc::getppid() as u32 }
}

fn getpid() -> u32 {
    unsafe { libc::getpid() as u32 }
}

fn say_hi(name: String) -> String {
    format!("hello {name}")
}

fn does_panic(name: String) -> String {
    panic!("sorry {name}, that didn't work")
}

fn does_error(_: ()) -> Result<(), WireError> {
    Err(std::io::Error::other("some wire error"))?;
    Ok(())
}

fn write_to_pipes(pipes: Vec<SendableFd<UnixStream>>) {
    for (i, mut pipe) in pipes.into_iter().enumerate() {
        write!(pipe, "hello world {i}!").unwrap();
        pipe.shutdown(Shutdown::Both).unwrap();
    }
}

#[test]
fn task_success() {
    let res = Zygote::global().run(say_hi, "Zygote");
    assert_eq!(res, "hello Zygote");
}

#[test]
#[should_panic]
fn task_failure() {
    Zygote::global().run(does_panic, "Zygote");
}

#[test]
fn many_calls() {
    Zygote::global().try_run(does_panic, "Zygote").unwrap_err();
    Zygote::global().try_run(does_panic, "Zygote").unwrap_err();
    Zygote::global().try_run(say_hi, "Zygote").unwrap();
    Zygote::global().try_run(say_hi, "Zygote").unwrap();
    Zygote::global().try_run(does_panic, "Zygote").unwrap_err();
    Zygote::global().try_run(say_hi, "Zygote").unwrap();
}

#[test]
fn large_payload() {
    let payload: Vec<u32> = (0..1024 * 1024).into_iter().collect();
    let res = Zygote::global().run(|v: Vec<_>| v, &payload);
    assert_eq!(res, payload);
}

#[test]
fn send_fd() {
    let (p1, p2) = UnixStream::pair().unwrap();
    Zygote::global().run(write_to_pipes, vec![SendableFd::from(p1)]);
    let msg = read_to_string(p2).unwrap();
    assert_eq!(msg, "hello world 0!");
}

#[test]
fn send_many_fd() {
    // less than SCM_MAX_FD
    let mut p1 = vec![];
    let mut p2 = vec![];
    for _ in 0..100 {
        let (pp1, pp2) = UnixStream::pair().unwrap();
        p1.push(SendableFd::from(pp1));
        p2.push(pp2);
    }
    Zygote::global().run(write_to_pipes, p1);
    for (i, pp2) in p2.into_iter().enumerate() {
        let msg = read_to_string(pp2).unwrap();
        assert_eq!(msg, format!("hello world {i}!"));
    }
}

#[test]
fn send_too_many_fd() {
    // more than SCM_MAX_FD
    let mut p1 = vec![];
    let mut p2 = vec![];
    for _ in 0..300 {
        let (pp1, pp2) = UnixStream::pair().unwrap();
        p1.push(SendableFd::from(pp1));
        p2.push(pp2);
    }
    Zygote::global().run(write_to_pipes, p1);
    for (i, pp2) in p2.into_iter().enumerate() {
        let msg = read_to_string(pp2).unwrap();
        assert_eq!(msg, format!("hello world {i}!"));
    }
}

#[test]
fn wire_error() {
    let err = Zygote::global().run(does_error, ()).unwrap_err();
    assert!(err.to_string().contains("some wire error"));
}

#[test]
fn nested_zygote() {
    let pid = getpid();
    let zyg_pid = Zygote::global().run(|_| getpid(), ());
    let zyg_ppid = Zygote::global().run(|_| getppid(), ());

    assert_ne!(pid, zyg_pid);
    assert_eq!(pid, zyg_ppid);

    let zygote = Zygote::global().spawn();
    let zygzyg_pid = zygote.run(|_| getpid(), ());
    let zygzyg_ppid = zygote.run(|_| getppid(), ());

    assert_ne!(pid, zygzyg_pid);
    assert_ne!(zyg_pid, zygzyg_pid);
    assert_eq!(zyg_pid, zygzyg_ppid);
}

#[test]
fn nested_sibling_zygote() {
    let pid = getpid();
    let zyg_pid = Zygote::global().run(|_| getpid(), ());
    let zyg_ppid = Zygote::global().run(|_| getppid(), ());

    assert_ne!(pid, zyg_pid);
    assert_eq!(pid, zyg_ppid);

    let zygote = Zygote::global().spawn_sibling();
    let zygzyg_pid = zygote.run(|_| getpid(), ());
    let zygzyg_ppid = zygote.run(|_| getppid(), ());

    assert_ne!(pid, zygzyg_pid);
    assert_ne!(zyg_pid, zygzyg_pid);
    assert_eq!(zyg_ppid, zygzyg_ppid);
    assert_eq!(pid, zygzyg_ppid);
}
