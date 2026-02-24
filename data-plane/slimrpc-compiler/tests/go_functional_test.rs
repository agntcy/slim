// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Functional tests for the Go slimrpc code generator.
//!
//! These tests invoke `buf generate` with the real `protoc-gen-slimrpc-go`
//! binary against the fixture proto files in `tests/testdata/`, exactly as
//! end users would.  Each test writes a temporary `buf.gen.yaml` pointing at
//! the compiled binary, runs `buf generate`, reads the output file, and
//! asserts on its content.
//!
//! Requirements: `buf` must be installed and available on PATH.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

// Cargo sets this env var to the path of the compiled binary under test.
const PLUGIN_BIN: &str = env!("CARGO_BIN_EXE_protoc-gen-slimrpc-go");

// The buf module root containing the fixture proto files.
const TESTDATA_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/testdata");

// Counter for unique temporary output directories (avoids collisions when
// tests run in parallel).
static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Create a unique temporary output directory for one test run.
fn make_out_dir() -> PathBuf {
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("slimrpc_go_test_{}", id));
    fs::create_dir_all(&dir).expect("Failed to create temp output dir");
    dir
}

/// Run `buf generate` against the testdata module, writing output to `out_dir`.
///
/// `extra_opts` is appended to the plugin's `opt:` list in the generated
/// `buf.gen.yaml`, e.g. `&["types_import=github.com/foo/bar"]`.
///
/// Returns the content of the generated `example_with_imports_slimrpc.pb.go`.
fn buf_generate(out_dir: &Path, extra_opts: &[&str]) -> String {
    // Build the opt block.
    let mut opts = vec!["paths=source_relative".to_string()];
    opts.extend(extra_opts.iter().map(|s| s.to_string()));
    let opt_lines: String = opts.iter().map(|o| format!("      - {}\n", o)).collect();

    let buf_gen_yaml = format!(
        "version: v2\nplugins:\n  - local: {plugin}\n    out: {out}\n    opt:\n{opts}",
        plugin = PLUGIN_BIN,
        out = out_dir.display(),
        opts = opt_lines,
    );

    // Write the temporary buf.gen.yaml into the output dir (not the module
    // root) so it does not affect other tests running concurrently.
    let template_path = out_dir.join("buf.gen.yaml");
    fs::write(&template_path, &buf_gen_yaml).expect("Failed to write buf.gen.yaml");

    let result = Command::new("buf")
        .args(["generate", "--template", template_path.to_str().unwrap()])
        .current_dir(TESTDATA_DIR)
        .output()
        .expect("Failed to spawn buf — is it installed and on PATH?");

    assert!(
        result.status.success(),
        "buf generate failed\n--- stdout ---\n{}\n--- stderr ---\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr),
    );

    let generated_path = out_dir.join("example_with_imports_slimrpc.pb.go");
    fs::read_to_string(&generated_path).unwrap_or_else(|e| {
        panic!(
            "Could not read generated file {}: {}\n\
             Files in out_dir:\n{}",
            generated_path.display(),
            e,
            fs::read_dir(out_dir)
                .map(|rd| rd
                    .filter_map(|e| e.ok())
                    .map(|e| format!("  {}", e.path().display()))
                    .collect::<Vec<_>>()
                    .join("\n"))
                .unwrap_or_default()
        )
    })
}

/// Assert that `content` contains `needle`, printing the full file on failure.
fn assert_contains(content: &str, needle: &str) {
    assert!(
        content.contains(needle),
        "expected to find {:?} in generated file:\n{}",
        needle,
        content,
    );
}

/// Assert that `content` does NOT contain `needle`.
fn assert_not_contains(content: &str, needle: &str) {
    assert!(
        !content.contains(needle),
        "unexpected {:?} found in generated file:\n{}",
        needle,
        content,
    );
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Without `types_import`: types from the current package are bare (unqualified)
/// and `google.protobuf.Empty` is auto-detected and imported as `emptypb`.
#[test]
fn test_imported_proto_types_are_auto_detected() {
    let out_dir = make_out_dir();
    let content = buf_generate(&out_dir, &[]);

    // Auto-detected import for the google well-known type.
    assert_contains(
        &content,
        "\"google.golang.org/protobuf/types/known/emptypb\"",
    );

    // The return type of Delete() must be qualified.
    assert_contains(&content, "*emptypb.Empty");

    // Types from the current proto package must remain bare (same Go package).
    assert_contains(&content, "*ExampleRequest");
    assert_contains(&content, "*ExampleResponse");
    assert_contains(&content, "*DeleteRequest");

    // No spurious package qualifier on same-package types.
    assert_not_contains(&content, "*example_service.ExampleRequest");
    assert_not_contains(&content, "*types.ExampleRequest");
}

/// With `types_import` set: same-package types get the `types.` prefix AND
/// `google.protobuf.Empty` still gets its own `emptypb` import and prefix.
#[test]
fn test_types_import_with_imported_proto_types() {
    let out_dir = make_out_dir();
    let content = buf_generate(
        &out_dir,
        &["types_import=github.com/agntcy/slim/data-plane/slimrpc-compiler/tests/testdata/types"],
    );

    // Both imports must be present.
    assert_contains(
        &content,
        "\"github.com/agntcy/slim/data-plane/slimrpc-compiler/tests/testdata/types\"",
    );
    assert_contains(
        &content,
        "\"google.golang.org/protobuf/types/known/emptypb\"",
    );

    // Same-package types use the types alias.
    assert_contains(&content, "*types.ExampleRequest");
    assert_contains(&content, "*types.ExampleResponse");
    assert_contains(&content, "*types.DeleteRequest");

    // Imported type still correctly qualified.
    assert_contains(&content, "*emptypb.Empty");

    // No bare same-package references.
    assert_not_contains(&content, "*ExampleRequest");
}

/// With an explicit `types_alias`: the import line uses the given alias rather
/// than deriving it from the last path component.
#[test]
fn test_explicit_types_alias_with_imported_proto_types() {
    let out_dir = make_out_dir();
    let content = buf_generate(
        &out_dir,
        &[
            "types_import=github.com/agntcy/slim/data-plane/slimrpc-compiler/tests/testdata/types",
            "types_alias=pb",
        ],
    );

    // Import line must carry the explicit alias.
    assert_contains(
        &content,
        "pb \"github.com/agntcy/slim/data-plane/slimrpc-compiler/tests/testdata/types\"",
    );

    // Same-package types use the explicit alias.
    assert_contains(&content, "*pb.ExampleRequest");
    assert_contains(&content, "*pb.DeleteRequest");

    // google.protobuf.Empty still auto-detected.
    assert_contains(&content, "*emptypb.Empty");
}
