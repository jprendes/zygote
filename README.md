# `zygote`
`zygote` is a library to create zygote processes on linux.
A zygote process is a small process used primarily to create new processes,
but can be used for anything that requires running things in a separate process.

To learn more about zygote processes check out [these notes on Chromium](https://neugierig.org/software/chromium/notes/2011/08/zygote.html).

## Getting started
```rust
use zygote::Zygote;
fn main() {
    Zygote::init();
    let pid = Zygote::global().run(|_| std::process::id(), ());
    assert_ne!(pid, std::process::id());
}
```