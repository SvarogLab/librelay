//! Every line the server writes reaches the client's stderr once, marked as the server's, with a
//! bare `\n` ending and its text otherwise intact.
#![no_main]

use libfuzzer_sys::fuzz_target;
use librelay::client::log::{SERVER_LINE_PREFIX, forward_server_stderr};

fuzz_target!(|server_stderr: &[u8]| {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let mut out = Vec::new();
    runtime.block_on(forward_server_stderr(server_stderr, &mut out));
    let out = String::from_utf8(out).expect("forwarded lines are valid UTF-8");

    let written: Vec<_> = server_stderr.split_inclusive(|&b| b == b'\n').collect();
    let forwarded: Vec<_> = out.split_inclusive('\n').collect();
    assert_eq!(forwarded.len(), written.len(), "{out:?}");
    for (written, forwarded) in written.into_iter().zip(forwarded) {
        let text = forwarded
            .strip_prefix(SERVER_LINE_PREFIX)
            .and_then(|line| line.strip_suffix('\n'))
            .unwrap_or_else(|| panic!("not a marked, terminated line: {forwarded:?}"));
        assert!(!text.ends_with(['\r', '\n']), "{forwarded:?}");
        assert!(
            String::from_utf8_lossy(written).starts_with(text),
            "{forwarded:?}"
        );
    }
});
