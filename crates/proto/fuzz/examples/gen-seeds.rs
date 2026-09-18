// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Regenerate the seed corpora.
//!
//! Coverage-guided fuzzing from an empty corpus spends most of a short budget
//! rediscovering basic structure — valid protobuf tags and wire types, or the
//! `a/b/c` shape of a name — which matters a lot for a time-boxed CI run. The
//! seeds here are deliberately minimal: they establish valid *structure* and
//! let libFuzzer mutate outwards from it.
//!
//! An example rather than a `[[bin]]` on purpose: `cargo fuzz` builds
//! `--bins`, and a binary without `fuzz_target!` is not a valid fuzz target.
//!
//! Run with `task fuzz:seeds`.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use agntcy_slim_proto::dataplane::proto::v1::{
    message::MessageType, Message, Publish, Subscribe, Unsubscribe,
};
use prost::Message as _;

fn write(dir: &Path, name: &str, bytes: &[u8]) {
    let path = dir.join(name);
    fs::write(&path, bytes).unwrap_or_else(|e| panic!("writing {}: {e}", path.display()));
    println!("  {} ({} bytes)", path.display(), bytes.len());
}

fn encode(msg: &Message) -> Vec<u8> {
    let mut buf = Vec::with_capacity(msg.encoded_len());
    msg.encode(&mut buf).expect("encoding a constructed message");
    buf
}

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("seeds");

    // message-decode: one seed per oneof arm we care about, plus the metadata
    // map, so every top-level tag appears somewhere in the corpus.
    let dir = root.join("message-decode");
    fs::create_dir_all(&dir).expect("creating seed dir");
    println!("message-decode:");

    // No empty seed: a zero-byte file carries no structure for libFuzzer to
    // mutate, and it tries the empty input on its own regardless.
    write(
        &dir,
        "subscribe.bin",
        &encode(&Message {
            metadata: HashMap::new(),
            message_type: Some(MessageType::Subscribe(Subscribe {
                header: None,
                subscription_id: 42,
            })),
        }),
    );

    write(
        &dir,
        "unsubscribe.bin",
        &encode(&Message {
            metadata: HashMap::new(),
            message_type: Some(MessageType::Unsubscribe(Unsubscribe {
                header: None,
                subscription_id: 7,
            })),
        }),
    );

    write(
        &dir,
        "publish.bin",
        &encode(&Message {
            metadata: HashMap::new(),
            message_type: Some(MessageType::Publish(Publish {
                header: None,
                session: None,
                msg: None,
            })),
        }),
    );

    let mut metadata = HashMap::new();
    metadata.insert("trace-id".to_string(), "0123456789abcdef".to_string());
    metadata.insert("k".to_string(), String::new());
    write(
        &dir,
        "metadata.bin",
        &encode(&Message {
            metadata,
            message_type: None,
        }),
    );

    // parse-name: the accepted shape, plus the edges the parser treats
    // specially (surrounding whitespace, non-ASCII components).
    let dir = root.join("parse-name");
    fs::create_dir_all(&dir).expect("creating seed dir");
    println!("parse-name:");

    for (name, content) in [
        ("simple", "org/default/alice"),
        ("padded", "  org / default / bob  "),
        ("unicode", "org/défaut/ålice"),
        ("long", "organisation-with-a-long-name/default/endpoint-0001"),
    ] {
        write(&dir, name, content.as_bytes());
    }
}
