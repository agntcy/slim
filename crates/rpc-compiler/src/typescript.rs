// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! TypeScript / Node.js slimrpc code generator.
//!
//! Emits a `<file_base>_slimrpc.ts` file per proto file that declares services.
//! The generated code targets:
//!   * the `@agntcy/slim-bindings` (uniffi-bindgen-react-native, napi) runtime
//!     for the RPC primitives (`Channel`, `Server`, handler interfaces, ...);
//!   * `@bufbuild/protobuf` (protobuf-es v2) for message (de)serialization,
//!     i.e. it assumes message types come from `protoc-gen-es` output
//!     (`*_pb.ts`, `toBinary()` / `fromBinary()`).
//!
//! For every service it emits:
//!   * a `<Service>Client` class (point-to-point client stub);
//!   * a `<Service>GroupClient` class (multicast / group client stub);
//!   * a `<Service>Servicer` interface (implemented by the user);
//!   * internal `_<Service>_<Method>_Handler` adapter classes implementing the
//!     runtime handler interfaces and delegating to the servicer;
//!   * a `register<Service>Servicer(server, servicer)` function.

use anyhow::{Context, Result};
use prost_types::compiler::{
    CodeGeneratorRequest, CodeGeneratorResponse, code_generator_response::File,
};
use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::common;

/// Plugin parameter: override the module specifier that the *current file's*
/// protobuf message types (and their `*Schema` companions) are imported from.
/// Defaults to a sibling `./<base>_pb.js` (the `protoc-gen-es` default output).
const TYPES_IMPORT: &str = "types_import";

/// Plugin parameter: module specifier for the slimrpc runtime (the
/// `@agntcy/slim-bindings` package by default). Useful for examples/tests that
/// consume a locally generated build via a relative path.
const BINDINGS_IMPORT: &str = "bindings_import";

/// Default module specifier for the runtime import.
const DEFAULT_BINDINGS_IMPORT: &str = "@agntcy/slim-bindings";

// --- TEMPLATE DEFINITIONS ---

/// Runtime prelude: helpers shared by every generated file.
///
/// `_toArrayBuffer` bridges protobuf-es's `Uint8Array` output to the exact
/// `ArrayBuffer` the FFI boundary expects. The runtime's `ArrayBuffer`
/// converter copies `new Uint8Array(buffer)` — i.e. the *whole* backing buffer
/// — so a pooled/oversized buffer (non-zero `byteOffset` or trailing capacity)
/// would corrupt the payload. We hand back an exactly-sized buffer, copying
/// only when the view does not already span its backing buffer.
const PRELUDE: &str = r#"function _toArrayBuffer(bytes: Uint8Array): ArrayBuffer {
  if (bytes.byteOffset === 0 && bytes.byteLength === bytes.buffer.byteLength) {
    return bytes.buffer as ArrayBuffer;
  }
  return bytes.slice().buffer as ArrayBuffer;
}

function _toRpcError(e: unknown): RpcError {
  if (e instanceof RpcError.Rpc || e instanceof RpcError.MulticastRpc) {
    return e;
  }
  const message = e instanceof Error ? e.message : String(e);
  return new RpcError.Rpc({ code: RpcCode.Internal, message, details: undefined });
}"#;

const CLIENT_UNARY_UNARY: &str = r#"  async {{METHOD_NAME}}(request: {{REQ_TYPE}}, timeout?: number, metadata?: Map<string, string>): Promise<{{RESP_TYPE}}> {
    const responseBytes = await this._channel.callUnaryAsync(
      "{{FULL_SERVICE}}",
      "{{METHOD_NAME}}",
      _toArrayBuffer(toBinary({{REQ_SCHEMA}}, request)),
      timeout,
      metadata,
    );
    return fromBinary({{RESP_SCHEMA}}, new Uint8Array(responseBytes));
  }

"#;

const CLIENT_UNARY_STREAM: &str = r#"  async *{{METHOD_NAME}}(request: {{REQ_TYPE}}, timeout?: number, metadata?: Map<string, string>): AsyncGenerator<{{RESP_TYPE}}> {
    const reader = await this._channel.callUnaryStreamAsync(
      "{{FULL_SERVICE}}",
      "{{METHOD_NAME}}",
      _toArrayBuffer(toBinary({{REQ_SCHEMA}}, request)),
      timeout,
      metadata,
    );
    while (true) {
      const msg = await reader.nextAsync();
      if (msg.tag === StreamMessage_Tags.End) break;
      if (msg.tag === StreamMessage_Tags.Error) throw msg.inner[0];
      yield fromBinary({{RESP_SCHEMA}}, new Uint8Array(msg.inner[0]));
    }
  }

"#;

const CLIENT_STREAM_UNARY: &str = r#"  async {{METHOD_NAME}}(requests: AsyncIterable<{{REQ_TYPE}}>, timeout?: number, metadata?: Map<string, string>): Promise<{{RESP_TYPE}}> {
    const writer = this._channel.callStreamUnary(
      "{{FULL_SERVICE}}",
      "{{METHOD_NAME}}",
      timeout,
      metadata,
    );
    for await (const request of requests) {
      await writer.sendAsync(_toArrayBuffer(toBinary({{REQ_SCHEMA}}, request)));
    }
    const responseBytes = await writer.finalizeStreamAsync();
    return fromBinary({{RESP_SCHEMA}}, new Uint8Array(responseBytes));
  }

"#;

const CLIENT_STREAM_STREAM: &str = r#"  async *{{METHOD_NAME}}(requests: AsyncIterable<{{REQ_TYPE}}>, timeout?: number, metadata?: Map<string, string>): AsyncGenerator<{{RESP_TYPE}}> {
    const bidi = this._channel.callStreamStream(
      "{{FULL_SERVICE}}",
      "{{METHOD_NAME}}",
      timeout,
      metadata,
    );
    const sendTask = (async () => {
      for await (const request of requests) {
        await bidi.sendAsync(_toArrayBuffer(toBinary({{REQ_SCHEMA}}, request)));
      }
      await bidi.closeSendAsync();
    })();
    try {
      while (true) {
        const msg = await bidi.recvAsync();
        if (msg.tag === StreamMessage_Tags.End) break;
        if (msg.tag === StreamMessage_Tags.Error) throw msg.inner[0];
        yield fromBinary({{RESP_SCHEMA}}, new Uint8Array(msg.inner[0]));
      }
    } finally {
      await sendTask;
    }
  }

"#;

const GROUP_UNARY_UNARY: &str = r#"  async *{{METHOD_NAME}}(request: {{REQ_TYPE}}, timeout?: number, metadata?: Map<string, string>): AsyncGenerator<{ context: RpcMessageContext; response: {{RESP_TYPE}} }> {
    const reader = await this._channel.callMulticastUnaryAsync(
      "{{FULL_SERVICE}}",
      "{{METHOD_NAME}}",
      _toArrayBuffer(toBinary({{REQ_SCHEMA}}, request)),
      timeout,
      metadata,
    );
    while (true) {
      const msg = await reader.nextAsync();
      if (msg.tag === MulticastStreamMessage_Tags.End) break;
      if (msg.tag === MulticastStreamMessage_Tags.Error) throw msg.inner.error;
      yield { context: msg.inner.item.context, response: fromBinary({{RESP_SCHEMA}}, new Uint8Array(msg.inner.item.message)) };
    }
  }

"#;

const GROUP_UNARY_STREAM: &str = r#"  async *{{METHOD_NAME}}(request: {{REQ_TYPE}}, timeout?: number, metadata?: Map<string, string>): AsyncGenerator<{ context: RpcMessageContext; response: {{RESP_TYPE}} }> {
    const reader = await this._channel.callMulticastUnaryStreamAsync(
      "{{FULL_SERVICE}}",
      "{{METHOD_NAME}}",
      _toArrayBuffer(toBinary({{REQ_SCHEMA}}, request)),
      timeout,
      metadata,
    );
    while (true) {
      const msg = await reader.nextAsync();
      if (msg.tag === MulticastStreamMessage_Tags.End) break;
      if (msg.tag === MulticastStreamMessage_Tags.Error) throw msg.inner.error;
      yield { context: msg.inner.item.context, response: fromBinary({{RESP_SCHEMA}}, new Uint8Array(msg.inner.item.message)) };
    }
  }

"#;

const GROUP_STREAM_UNARY: &str = r#"  async *{{METHOD_NAME}}(requests: AsyncIterable<{{REQ_TYPE}}>, timeout?: number, metadata?: Map<string, string>): AsyncGenerator<{ context: RpcMessageContext; response: {{RESP_TYPE}} }> {
    const handler = this._channel.callMulticastStreamUnary(
      "{{FULL_SERVICE}}",
      "{{METHOD_NAME}}",
      timeout,
      metadata,
    );
    const sendTask = (async () => {
      for await (const request of requests) {
        await handler.sendAsync(_toArrayBuffer(toBinary({{REQ_SCHEMA}}, request)));
      }
      await handler.closeSendAsync();
    })();
    try {
      while (true) {
        const msg = await handler.recvAsync();
        if (msg.tag === MulticastStreamMessage_Tags.End) break;
        if (msg.tag === MulticastStreamMessage_Tags.Error) throw msg.inner.error;
        yield { context: msg.inner.item.context, response: fromBinary({{RESP_SCHEMA}}, new Uint8Array(msg.inner.item.message)) };
      }
    } finally {
      await sendTask;
    }
  }

"#;

const GROUP_STREAM_STREAM: &str = r#"  async *{{METHOD_NAME}}(requests: AsyncIterable<{{REQ_TYPE}}>, timeout?: number, metadata?: Map<string, string>): AsyncGenerator<{ context: RpcMessageContext; response: {{RESP_TYPE}} }> {
    const handler = this._channel.callMulticastStreamStream(
      "{{FULL_SERVICE}}",
      "{{METHOD_NAME}}",
      timeout,
      metadata,
    );
    const sendTask = (async () => {
      for await (const request of requests) {
        await handler.sendAsync(_toArrayBuffer(toBinary({{REQ_SCHEMA}}, request)));
      }
      await handler.closeSendAsync();
    })();
    try {
      while (true) {
        const msg = await handler.recvAsync();
        if (msg.tag === MulticastStreamMessage_Tags.End) break;
        if (msg.tag === MulticastStreamMessage_Tags.Error) throw msg.inner.error;
        yield { context: msg.inner.item.context, response: fromBinary({{RESP_SCHEMA}}, new Uint8Array(msg.inner.item.message)) };
      }
    } finally {
      await sendTask;
    }
  }

"#;

const SERVICER_UNARY_UNARY: &str =
    "  {{METHOD_NAME}}(request: {{REQ_TYPE}}, context: ContextLike): Promise<{{RESP_TYPE}}>;\n";
const SERVICER_UNARY_STREAM: &str = "  {{METHOD_NAME}}(request: {{REQ_TYPE}}, context: ContextLike): AsyncIterable<{{RESP_TYPE}}>;\n";
const SERVICER_STREAM_UNARY: &str = "  {{METHOD_NAME}}(requests: AsyncIterable<{{REQ_TYPE}}>, context: ContextLike): Promise<{{RESP_TYPE}}>;\n";
const SERVICER_STREAM_STREAM: &str = "  {{METHOD_NAME}}(requests: AsyncIterable<{{REQ_TYPE}}>, context: ContextLike): AsyncIterable<{{RESP_TYPE}}>;\n";

const HANDLER_UNARY_UNARY: &str = r#"class _{{SERVICE_NAME}}_{{METHOD_NAME}}_Handler implements UnaryUnaryHandler {
  constructor(private readonly servicer: {{SERVICE_NAME}}Servicer) {}

  async handle(request: ArrayBuffer, context: ContextLike): Promise<ArrayBuffer> {
    try {
      const req = fromBinary({{REQ_SCHEMA}}, new Uint8Array(request));
      const response = await this.servicer.{{METHOD_NAME}}(req, context);
      return _toArrayBuffer(toBinary({{RESP_SCHEMA}}, response));
    } catch (e) {
      throw _toRpcError(e);
    }
  }
}

"#;

const HANDLER_UNARY_STREAM: &str = r#"class _{{SERVICE_NAME}}_{{METHOD_NAME}}_Handler implements UnaryStreamHandler {
  constructor(private readonly servicer: {{SERVICE_NAME}}Servicer) {}

  async handle(request: ArrayBuffer, context: ContextLike, sink: ResponseSinkLike): Promise<void> {
    try {
      const req = fromBinary({{REQ_SCHEMA}}, new Uint8Array(request));
      for await (const response of this.servicer.{{METHOD_NAME}}(req, context)) {
        await sink.sendAsync(_toArrayBuffer(toBinary({{RESP_SCHEMA}}, response)));
      }
      await sink.closeAsync();
    } catch (e) {
      await sink.sendErrorAsync(_toRpcError(e));
    }
  }
}

"#;

const HANDLER_STREAM_UNARY: &str = r#"class _{{SERVICE_NAME}}_{{METHOD_NAME}}_Handler implements StreamUnaryHandler {
  constructor(private readonly servicer: {{SERVICE_NAME}}Servicer) {}

  async handle(stream: RequestStreamLike, context: ContextLike): Promise<ArrayBuffer> {
    const requests = (async function* () {
      while (true) {
        const msg = await stream.nextAsync();
        if (msg.tag === StreamMessage_Tags.End) return;
        if (msg.tag === StreamMessage_Tags.Error) throw msg.inner[0];
        yield fromBinary({{REQ_SCHEMA}}, new Uint8Array(msg.inner[0]));
      }
    })();
    try {
      const response = await this.servicer.{{METHOD_NAME}}(requests, context);
      return _toArrayBuffer(toBinary({{RESP_SCHEMA}}, response));
    } catch (e) {
      throw _toRpcError(e);
    }
  }
}

"#;

const HANDLER_STREAM_STREAM: &str = r#"class _{{SERVICE_NAME}}_{{METHOD_NAME}}_Handler implements StreamStreamHandler {
  constructor(private readonly servicer: {{SERVICE_NAME}}Servicer) {}

  async handle(stream: RequestStreamLike, context: ContextLike, sink: ResponseSinkLike): Promise<void> {
    const requests = (async function* () {
      while (true) {
        const msg = await stream.nextAsync();
        if (msg.tag === StreamMessage_Tags.End) return;
        if (msg.tag === StreamMessage_Tags.Error) throw msg.inner[0];
        yield fromBinary({{REQ_SCHEMA}}, new Uint8Array(msg.inner[0]));
      }
    })();
    try {
      for await (const response of this.servicer.{{METHOD_NAME}}(requests, context)) {
        await sink.sendAsync(_toArrayBuffer(toBinary({{RESP_SCHEMA}}, response)));
      }
      await sink.closeAsync();
    } catch (e) {
      await sink.sendErrorAsync(_toRpcError(e));
    }
  }
}

"#;

const REGISTER_METHOD: &str = "  server.register{{RPC_METHOD}}(\"{{FULL_SERVICE}}\", \"{{METHOD_NAME}}\", new _{{SERVICE_NAME}}_{{METHOD_NAME}}_Handler(servicer));\n";

const SERVICE_TEMPLATE: &str = r#"export class {{SERVICE_NAME}}Client {
  constructor(private readonly _channel: ChannelLike) {}

{{CLIENT_METHODS}}}

export class {{SERVICE_NAME}}GroupClient {
  constructor(private readonly _channel: ChannelLike) {}

{{GROUP_METHODS}}}

export interface {{SERVICE_NAME}}Servicer {
{{SERVICER_METHODS}}}

{{HANDLER_CLASSES}}
export function register{{SERVICE_NAME}}Servicer(server: ServerLike, servicer: {{SERVICE_NAME}}Servicer): void {
{{REGISTER_METHODS}}}
"#;

// --- END TEMPLATE DEFINITIONS ---

/// A proto message type resolved to its TypeScript surface.
struct ResolvedType {
    /// The bare TS type name (e.g. `ExampleRequest`).
    ts_type: String,
    /// The protobuf-es schema companion (e.g. `ExampleRequestSchema`).
    schema: String,
    /// The module specifier the type + schema are imported from.
    module: String,
}

/// Strip a directory + `.proto` suffix, returning the bare file base
/// (e.g. `service/api.proto` -> `api`).
fn file_base(proto_file: &str) -> &str {
    let name = proto_file.rsplit('/').next().unwrap_or(proto_file);
    name.strip_suffix(".proto").unwrap_or(name)
}

/// Compute the relative module specifier (with a trailing `_pb.js`) to import a
/// dependency proto's generated types from the current proto's generated file.
///
/// Both files live at their proto paths (that's how `protoc-gen-es` lays out
/// output), so this is a plain directory-relative path.
fn relative_pb_module(from_file: &str, to_file: &str) -> String {
    let from_parts: Vec<&str> = from_file.split('/').collect();
    let to_parts: Vec<&str> = to_file.split('/').collect();

    let from_dir = &from_parts[..from_parts.len().saturating_sub(1)];
    let to_dir = &to_parts[..to_parts.len().saturating_sub(1)];
    let to_base = file_base(to_file);

    // Strip the shared leading path components.
    let mut common = 0;
    while common < from_dir.len() && common < to_dir.len() && from_dir[common] == to_dir[common] {
        common += 1;
    }

    let ups = from_dir.len() - common;
    let mut segments: Vec<String> = Vec::new();
    for _ in 0..ups {
        segments.push("..".to_string());
    }
    for part in &to_dir[common..] {
        segments.push(part.to_string());
    }
    segments.push(format!("{}_pb.js", to_base));

    let joined = segments.join("/");
    // A same-directory or child path needs an explicit `./` prefix to be a
    // relative ES module specifier.
    if joined.starts_with("..") {
        joined
    } else {
        format!("./{}", joined)
    }
}

/// Resolve a fully-qualified proto type (e.g. `.example_service.ExampleRequest`)
/// to its TypeScript surface.
fn resolve_type(
    qualified: &str,
    current_file: &str,
    current_package: &str,
    types_import_override: &Option<String>,
    package_to_file: &HashMap<String, String>,
) -> ResolvedType {
    let trimmed = qualified.trim_start_matches('.');
    let bare = trimmed.rsplit('.').next().unwrap_or(trimmed).to_string();
    let schema = format!("{}Schema", bare);
    let type_pkg = trimmed.rfind('.').map(|pos| &trimmed[..pos]).unwrap_or("");

    // protobuf-es exports the well-known types (and their schemas) from a
    // dedicated subpath.
    if trimmed.starts_with("google.protobuf.") {
        return ResolvedType {
            ts_type: bare,
            schema,
            module: "@bufbuild/protobuf/wkt".to_string(),
        };
    }

    let local_module = || {
        types_import_override
            .clone()
            .unwrap_or_else(|| format!("./{}_pb.js", file_base(current_file)))
    };

    if type_pkg == current_package {
        return ResolvedType {
            ts_type: bare,
            schema,
            module: local_module(),
        };
    }

    if let Some(dep_file) = package_to_file.get(type_pkg) {
        return ResolvedType {
            ts_type: bare,
            schema,
            module: relative_pb_module(current_file, dep_file),
        };
    }

    // Package not found among the inputs: fall back to the local module.
    ResolvedType {
        ts_type: bare,
        schema,
        module: local_module(),
    }
}

/// Build the runtime import statement for the given `@agntcy/slim-bindings`
/// module specifier.
fn runtime_import(bindings_import: &str) -> String {
    // Type against the `*Like` interfaces rather than the concrete classes:
    // the runtime's factory methods (`Channel.newWithConnection`,
    // `Server.newWithConnection`, ...) and callback-handler arguments are all
    // typed as the interfaces, so a concrete instance and a factory result both
    // satisfy them.
    format!(
        r#"import {{
  RpcError,
  RpcCode,
  StreamMessage_Tags,
  MulticastStreamMessage_Tags,
  type ChannelLike,
  type ServerLike,
  type ContextLike,
  type RequestStreamLike,
  type ResponseSinkLike,
  type RpcMessageContext,
  type UnaryUnaryHandler,
  type UnaryStreamHandler,
  type StreamUnaryHandler,
  type StreamStreamHandler,
}} from "{}";"#,
        bindings_import
    )
}

/// Generate TypeScript slimrpc code from a `CodeGeneratorRequest`.
pub fn generate(request: CodeGeneratorRequest) -> Result<CodeGeneratorResponse> {
    let params = common::parse_parameters(request.parameter.as_deref().unwrap_or(""));

    let types_import_override: Option<String> = params.get(TYPES_IMPORT).cloned();
    let bindings_import = params
        .get(BINDINGS_IMPORT)
        .cloned()
        .unwrap_or_else(|| DEFAULT_BINDINGS_IMPORT.to_string());

    // Map proto package -> proto file name, for cross-package type resolution.
    let mut package_to_file: HashMap<String, String> = HashMap::new();
    for file_desc in &request.proto_file {
        if let (Some(name), Some(pkg)) = (&file_desc.name, &file_desc.package) {
            package_to_file.entry(pkg.clone()).or_insert(name.clone());
        }
    }

    // `supported_features = 1` advertises FEATURE_PROTO3_OPTIONAL, matching the
    // other class-based targets (csharp).
    let mut response = CodeGeneratorResponse {
        supported_features: Some(1),
        ..Default::default()
    };

    for file_name in &request.file_to_generate {
        let Some(file_descriptor) = request
            .proto_file
            .iter()
            .find(|f| f.name.as_ref() == Some(file_name))
        else {
            continue;
        };

        if file_descriptor.service.is_empty() {
            continue;
        }

        let package_name = file_descriptor.package.clone().unwrap_or_default();
        let output_file = format!("{}_slimrpc.ts", strip_proto(file_name));

        // module specifier -> set of import members (types prefixed with `type `).
        let mut type_imports: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut service_definitions = String::new();

        let add_type = |resolved: &ResolvedType,
                        imports: &mut BTreeMap<String, BTreeSet<String>>| {
            let members = imports.entry(resolved.module.clone()).or_default();
            members.insert(format!("type {}", resolved.ts_type));
            members.insert(resolved.schema.clone());
        };

        for service in &file_descriptor.service {
            let service_name = service.name.clone().context("Service name missing")?;
            let full_service = if package_name.is_empty() {
                service_name.clone()
            } else {
                format!("{}.{}", package_name, service_name)
            };

            let mut client_methods = String::new();
            let mut group_methods = String::new();
            let mut servicer_methods = String::new();
            let mut handler_classes = String::new();
            let mut register_methods = String::new();

            for method in &service.method {
                let method_name = method.name.clone().context("Method name missing")?;
                let raw_input = method.input_type.clone().context("Input type missing")?;
                let raw_output = method.output_type.clone().context("Output type missing")?;

                let input = resolve_type(
                    &raw_input,
                    file_name,
                    &package_name,
                    &types_import_override,
                    &package_to_file,
                );
                let output = resolve_type(
                    &raw_output,
                    file_name,
                    &package_name,
                    &types_import_override,
                    &package_to_file,
                );
                add_type(&input, &mut type_imports);
                add_type(&output, &mut type_imports);

                let is_client_streaming = method.client_streaming.unwrap_or(false);
                let is_server_streaming = method.server_streaming.unwrap_or(false);

                let (client_tpl, group_tpl, servicer_tpl, handler_tpl, rpc_method) =
                    match (is_client_streaming, is_server_streaming) {
                        (false, false) => (
                            CLIENT_UNARY_UNARY,
                            GROUP_UNARY_UNARY,
                            SERVICER_UNARY_UNARY,
                            HANDLER_UNARY_UNARY,
                            "UnaryUnary",
                        ),
                        (false, true) => (
                            CLIENT_UNARY_STREAM,
                            GROUP_UNARY_STREAM,
                            SERVICER_UNARY_STREAM,
                            HANDLER_UNARY_STREAM,
                            "UnaryStream",
                        ),
                        (true, false) => (
                            CLIENT_STREAM_UNARY,
                            GROUP_STREAM_UNARY,
                            SERVICER_STREAM_UNARY,
                            HANDLER_STREAM_UNARY,
                            "StreamUnary",
                        ),
                        (true, true) => (
                            CLIENT_STREAM_STREAM,
                            GROUP_STREAM_STREAM,
                            SERVICER_STREAM_STREAM,
                            HANDLER_STREAM_STREAM,
                            "StreamStream",
                        ),
                    };

                let fill = |tpl: &str| -> String {
                    tpl.replace("{{METHOD_NAME}}", &method_name)
                        .replace("{{SERVICE_NAME}}", &service_name)
                        .replace("{{FULL_SERVICE}}", &full_service)
                        .replace("{{REQ_TYPE}}", &input.ts_type)
                        .replace("{{REQ_SCHEMA}}", &input.schema)
                        .replace("{{RESP_TYPE}}", &output.ts_type)
                        .replace("{{RESP_SCHEMA}}", &output.schema)
                };

                client_methods.push_str(&fill(client_tpl));
                group_methods.push_str(&fill(group_tpl));
                servicer_methods.push_str(&fill(servicer_tpl));
                handler_classes.push_str(&fill(handler_tpl));
                register_methods
                    .push_str(&fill(REGISTER_METHOD).replace("{{RPC_METHOD}}", rpc_method));
            }

            let service_block = SERVICE_TEMPLATE
                .replace("{{SERVICE_NAME}}", &service_name)
                .replace("{{CLIENT_METHODS}}", &client_methods)
                .replace("{{GROUP_METHODS}}", &group_methods)
                .replace("{{SERVICER_METHODS}}", &servicer_methods)
                .replace("{{HANDLER_CLASSES}}", handler_classes.trim_end())
                .replace("{{REGISTER_METHODS}}", &register_methods);
            service_definitions.push_str(&service_block);
            service_definitions.push('\n');
        }

        // Emit the type imports, deterministically ordered.
        let mut type_import_lines = String::new();
        for (module, members) in &type_imports {
            let joined: Vec<String> = members.iter().cloned().collect();
            type_import_lines.push_str(&format!(
                "import {{ {} }} from \"{}\";\n",
                joined.join(", "),
                module
            ));
        }

        let content = format!(
            "// Code generated by protoc-gen-slimrpc-node. DO NOT EDIT.\n\
             // source: {source}\n\
             \n\
             {runtime}\n\
             import {{ fromBinary, toBinary }} from \"@bufbuild/protobuf\";\n\
             {types}\n\
             {prelude}\n\
             \n\
             {services}",
            source = file_name,
            runtime = runtime_import(&bindings_import),
            types = type_import_lines,
            prelude = PRELUDE,
            services = service_definitions.trim_end(),
        );

        response.file.push(File {
            name: Some(output_file),
            content: Some(format!("{}\n", content)),
            ..Default::default()
        });
    }

    Ok(response)
}

/// Strip only the `.proto` suffix, preserving directory components.
fn strip_proto(proto_file: &str) -> &str {
    proto_file.strip_suffix(".proto").unwrap_or(proto_file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost_types::{FileDescriptorProto, MethodDescriptorProto, ServiceDescriptorProto};

    fn create_test_file_descriptor(
        name: &str,
        package: &str,
        services: Vec<ServiceDescriptorProto>,
    ) -> FileDescriptorProto {
        FileDescriptorProto {
            name: Some(name.to_string()),
            package: Some(package.to_string()),
            service: services,
            message_type: vec![],
            enum_type: vec![],
            dependency: vec![],
            public_dependency: vec![],
            weak_dependency: vec![],
            extension: vec![],
            options: None,
            source_code_info: None,
            syntax: None,
        }
    }

    fn create_test_method(
        name: &str,
        input_type: &str,
        output_type: &str,
        client_streaming: bool,
        server_streaming: bool,
    ) -> MethodDescriptorProto {
        MethodDescriptorProto {
            name: Some(name.to_string()),
            input_type: Some(input_type.to_string()),
            output_type: Some(output_type.to_string()),
            client_streaming: Some(client_streaming),
            server_streaming: Some(server_streaming),
            options: None,
        }
    }

    fn create_test_service(
        name: &str,
        methods: Vec<MethodDescriptorProto>,
    ) -> ServiceDescriptorProto {
        ServiceDescriptorProto {
            name: Some(name.to_string()),
            method: methods,
            options: None,
        }
    }

    fn generate_single(file: FileDescriptorProto, param: Option<String>) -> CodeGeneratorResponse {
        let name = file.name.clone().unwrap();
        let request = CodeGeneratorRequest {
            file_to_generate: vec![name],
            parameter: param,
            proto_file: vec![file],
            compiler_version: None,
        };
        generate(request).unwrap()
    }

    #[test]
    fn test_generate_no_services() {
        let response = generate_single(
            create_test_file_descriptor("test.proto", "test.package", vec![]),
            None,
        );
        assert_eq!(response.file.len(), 0);
    }

    #[test]
    fn test_generate_unary_unary() {
        let method = create_test_method(
            "GetUser",
            ".test.package.GetUserRequest",
            ".test.package.GetUserResponse",
            false,
            false,
        );
        let service = create_test_service("UserService", vec![method]);
        let file = create_test_file_descriptor("user.proto", "test.package", vec![service]);

        let response = generate_single(file, None);
        assert_eq!(response.file.len(), 1);

        let generated = &response.file[0];
        assert_eq!(generated.name.as_ref().unwrap(), "user_slimrpc.ts");

        let content = generated.content.as_ref().unwrap();
        assert!(content.contains("export class UserServiceClient"));
        assert!(content.contains("export class UserServiceGroupClient"));
        assert!(content.contains("export interface UserServiceServicer"));
        assert!(content.contains("export function registerUserServiceServicer(server: ServerLike, servicer: UserServiceServicer)"));
        // Correct channel variant for unary-unary.
        assert!(content.contains("this._channel.callUnaryAsync"));
        // Correct server registration variant.
        assert!(
            content.contains("server.registerUnaryUnary(\"test.package.UserService\", \"GetUser\"")
        );
        // Handler adapter class.
        assert!(
            content.contains("class _UserService_GetUser_Handler implements UnaryUnaryHandler")
        );
        // Types + schemas imported from the default protoc-gen-es module.
        assert!(content.contains("from \"./user_pb.js\""));
        assert!(content.contains("GetUserRequestSchema"));
        assert!(content.contains("type GetUserResponse"));
        // Serialization boundary helpers.
        assert!(content.contains("toBinary(GetUserRequestSchema, request)"));
        assert!(
            content.contains("fromBinary(GetUserResponseSchema, new Uint8Array(responseBytes))")
        );
        // Runtime import.
        assert!(content.contains("from \"@agntcy/slim-bindings\""));
        assert!(content.contains("import { fromBinary, toBinary } from \"@bufbuild/protobuf\""));
    }

    #[test]
    fn test_generate_streaming_shapes_use_correct_channel_methods() {
        let methods = vec![
            create_test_method(
                "UnaryToStream",
                ".pkg.Request",
                ".pkg.Response",
                false,
                true,
            ),
            create_test_method(
                "StreamToUnary",
                ".pkg.Request",
                ".pkg.Response",
                true,
                false,
            ),
            create_test_method(
                "StreamToStream",
                ".pkg.Request",
                ".pkg.Response",
                true,
                true,
            ),
        ];
        let service = create_test_service("StreamService", methods);
        let file = create_test_file_descriptor("stream.proto", "pkg", vec![service]);

        let content = generate_single(file, None).file[0].content.clone().unwrap();

        // Client dispatches to the matching channel.call* variant per shape.
        assert!(content.contains("async *UnaryToStream("));
        assert!(content.contains("this._channel.callUnaryStreamAsync"));
        assert!(content.contains("async StreamToUnary("));
        assert!(content.contains("this._channel.callStreamUnary"));
        assert!(content.contains("finalizeStreamAsync()"));
        assert!(content.contains("async *StreamToStream("));
        assert!(content.contains("this._channel.callStreamStream"));

        // Server registration variants.
        assert!(content.contains("server.registerUnaryStream("));
        assert!(content.contains("server.registerStreamUnary("));
        assert!(content.contains("server.registerStreamStream("));

        // Handler interfaces implemented per shape.
        assert!(content.contains("implements UnaryStreamHandler"));
        assert!(content.contains("implements StreamUnaryHandler"));
        assert!(content.contains("implements StreamStreamHandler"));

        // Servicer signatures.
        assert!(content.contains(
            "UnaryToStream(request: Request, context: ContextLike): AsyncIterable<Response>;"
        ));
        assert!(content.contains("StreamToUnary(requests: AsyncIterable<Request>, context: ContextLike): Promise<Response>;"));
        assert!(content.contains("StreamToStream(requests: AsyncIterable<Request>, context: ContextLike): AsyncIterable<Response>;"));

        // Client-side stream reading uses the tuple StreamMessage shape.
        assert!(content.contains("StreamMessage_Tags.End"));
        assert!(content.contains("throw msg.inner[0];"));
        assert!(content.contains("new Uint8Array(msg.inner[0])"));
    }

    #[test]
    fn test_group_client_uses_multicast_and_named_field_shape() {
        let methods = vec![
            create_test_method(
                "UnaryUnary",
                ".test.Request",
                ".test.Response",
                false,
                false,
            ),
            create_test_method(
                "UnaryStream",
                ".test.Request",
                ".test.Response",
                false,
                true,
            ),
            create_test_method(
                "StreamUnary",
                ".test.Request",
                ".test.Response",
                true,
                false,
            ),
            create_test_method(
                "StreamStream",
                ".test.Request",
                ".test.Response",
                true,
                true,
            ),
        ];
        let service = create_test_service("Test", methods);
        let file = create_test_file_descriptor("example.proto", "test", vec![service]);

        let content = generate_single(file, None).file[0].content.clone().unwrap();

        assert!(content.contains("export class TestGroupClient"));
        assert!(content.contains("callMulticastUnaryAsync"));
        assert!(content.contains("callMulticastUnaryStreamAsync"));
        assert!(content.contains("callMulticastStreamUnary"));
        assert!(content.contains("callMulticastStreamStream"));

        // Multicast messages use the *named-field* union shape, NOT the tuple
        // shape used by StreamMessage.
        assert!(content.contains("MulticastStreamMessage_Tags.End"));
        assert!(content.contains("throw msg.inner.error;"));
        assert!(content.contains("msg.inner.item.context"));
        assert!(content.contains("msg.inner.item.message"));
        assert!(content.contains("context: RpcMessageContext; response: Response"));
    }

    #[test]
    fn test_types_import_override() {
        let method = create_test_method("Echo", ".test.Echo", ".test.Echo", false, false);
        let service = create_test_service("TestService", vec![method]);
        let file = create_test_file_descriptor("test.proto", "test", vec![service]);

        let content = generate_single(
            file,
            Some("types_import=@myorg/generated/test_pb.js".to_string()),
        )
        .file[0]
            .content
            .clone()
            .unwrap();

        assert!(content.contains("from \"@myorg/generated/test_pb.js\""));
        assert!(!content.contains("from \"./test_pb.js\""));
    }

    #[test]
    fn test_bindings_import_override() {
        let method = create_test_method("Echo", ".test.Echo", ".test.Echo", false, false);
        let service = create_test_service("TestService", vec![method]);
        let file = create_test_file_descriptor("test.proto", "test", vec![service]);

        let content = generate_single(
            file,
            Some("bindings_import=../../../../generated/index.js".to_string()),
        )
        .file[0]
            .content
            .clone()
            .unwrap();

        assert!(content.contains("from \"../../../../generated/index.js\""));
        assert!(!content.contains("from \"@agntcy/slim-bindings\""));
    }

    #[test]
    fn test_google_well_known_types() {
        let method = create_test_method(
            "TestEmpty",
            ".google.protobuf.Empty",
            ".test.Response",
            false,
            false,
        );
        let service = create_test_service("TestService", vec![method]);
        let file = create_test_file_descriptor("test.proto", "test", vec![service]);

        let content = generate_single(file, None).file[0].content.clone().unwrap();

        assert!(content.contains("from \"@bufbuild/protobuf/wkt\""));
        assert!(content.contains("EmptySchema"));
        assert!(content.contains("type Empty"));
        assert!(content.contains("toBinary(EmptySchema, request)"));
    }

    #[test]
    fn test_multiple_services() {
        let s1 = create_test_service(
            "Service1",
            vec![create_test_method("M1", ".pkg.R1", ".pkg.R1", false, false)],
        );
        let s2 = create_test_service(
            "Service2",
            vec![create_test_method("M2", ".pkg.R2", ".pkg.R2", false, false)],
        );
        let file = create_test_file_descriptor("multi.proto", "pkg", vec![s1, s2]);

        let content = generate_single(file, None).file[0].content.clone().unwrap();

        assert!(content.contains("export class Service1Client"));
        assert!(content.contains("export class Service2Client"));
        assert!(content.contains("export function registerService1Servicer"));
        assert!(content.contains("export function registerService2Servicer"));
    }

    #[test]
    fn test_not_in_files_to_generate() {
        let method = create_test_method("Test", ".pkg.Req", ".pkg.Res", false, false);
        let service = create_test_service("TestService", vec![method]);
        let file = create_test_file_descriptor("test.proto", "pkg", vec![service]);

        let request = CodeGeneratorRequest {
            file_to_generate: vec!["other.proto".to_string()],
            parameter: None,
            proto_file: vec![file],
            compiler_version: None,
        };

        let response = generate(request).unwrap();
        assert_eq!(response.file.len(), 0);
    }

    #[test]
    fn test_output_filename_preserves_case_and_path() {
        let method = create_test_method("Test", ".pkg.Req", ".pkg.Res", false, false);
        let service = create_test_service("TestService", vec![method]);
        let file = create_test_file_descriptor("api/CamelCase.proto", "pkg", vec![service]);

        let response = generate_single(file, None);
        assert_eq!(
            response.file[0].name.as_ref().unwrap(),
            "api/CamelCase_slimrpc.ts"
        );
    }

    #[test]
    fn test_cross_package_dependency() {
        let dep = create_test_file_descriptor("common/types.proto", "common.types", vec![]);

        let method = create_test_method(
            "ProcessRequest",
            ".common.types.CommonRequest",
            ".service.pkg.ServiceResponse",
            false,
            false,
        );
        let service = create_test_service("MyService", vec![method]);
        let main = create_test_file_descriptor("service/api.proto", "service.pkg", vec![service]);

        let request = CodeGeneratorRequest {
            file_to_generate: vec!["service/api.proto".to_string()],
            parameter: None,
            proto_file: vec![dep, main],
            compiler_version: None,
        };

        let response = generate(request).unwrap();
        assert_eq!(response.file.len(), 1);
        let content = response.file[0].content.as_ref().unwrap();

        // Cross-package dependency resolves to a relative _pb.js module.
        assert!(content.contains("from \"../common/types_pb.js\""));
        assert!(content.contains("CommonRequestSchema"));
        // Local package type resolves to the sibling _pb.js.
        assert!(content.contains("from \"./api_pb.js\""));
        assert!(content.contains("ServiceResponseSchema"));
    }

    #[test]
    fn test_cross_package_type_not_found_falls_back_local() {
        let method = create_test_method(
            "ProcessData",
            ".unknown.package.Request",
            ".test.package.Response",
            false,
            false,
        );
        let service = create_test_service("DataService", vec![method]);
        let file = create_test_file_descriptor("data.proto", "test.package", vec![service]);

        let content = generate_single(file, None).file[0].content.clone().unwrap();

        // Unknown package falls back to the local module.
        assert!(content.contains("from \"./data_pb.js\""));
        assert!(content.contains("RequestSchema"));
    }

    #[test]
    fn test_empty_package() {
        let method = create_test_method("Test", ".Req", ".Res", false, false);
        let service = create_test_service("TestService", vec![method]);
        let file = create_test_file_descriptor("test.proto", "", vec![service]);

        let content = generate_single(file, None).file[0].content.clone().unwrap();

        assert!(content.contains("export class TestServiceClient"));
        // No package -> unqualified service name in the registration string.
        assert!(content.contains("server.registerUnaryUnary(\"TestService\", \"Test\""));
    }

    #[test]
    fn test_relative_pb_module() {
        assert_eq!(
            relative_pb_module("example.proto", "example.proto"),
            "./example_pb.js"
        );
        assert_eq!(
            relative_pb_module("service/api.proto", "common/types.proto"),
            "../common/types_pb.js"
        );
        assert_eq!(
            relative_pb_module("a/b/c.proto", "a/b/d.proto"),
            "./d_pb.js"
        );
        assert_eq!(
            relative_pb_module("a/b/c.proto", "a/x/d.proto"),
            "../x/d_pb.js"
        );
    }
}
