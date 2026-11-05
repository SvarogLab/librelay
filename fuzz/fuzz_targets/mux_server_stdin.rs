//! Whatever arrives on the server's stdin, `serve` ends once stdin does — with `Ok` or a
//! connection error, never a panic, a hang, or `MuxError::Closed`, which only the client side
//! reports.
#![no_main]

use futures::io::AsyncReadExt;
use libfuzzer_sys::fuzz_target;
use librelay::mux::MuxError;

fuzz_target!(|stdin: &[u8]| {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let io = tokio::io::join(stdin, tokio::io::sink());
    let served = runtime.block_on(librelay::server::mux::serve(io, |mut stream| async move {
        let _ = stream.read_to_end(&mut Vec::new()).await;
    }));
    assert!(!matches!(served, Err(MuxError::Closed)), "{served:?}");
});
