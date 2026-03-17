// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use anyhow::{Context, Result};
use heck::ToUpperCamelCase;
use prost_types::compiler::{
    CodeGeneratorRequest, CodeGeneratorResponse, code_generator_response::File,
};
use std::collections::HashMap;

use crate::common;

/// Supported parameters for the C# code generator plugin.
const BASE_NAMESPACE: &str = "base_namespace";
const TYPES_NAMESPACE: &str = "types_namespace";

/// Convert proto package (e.g. "example_service") to C# namespace (e.g. "ExampleService").
fn package_to_namespace(pkg: &str) -> String {
    pkg.split('.')
        .map(|s| s.to_upper_camel_case())
        .collect::<Vec<_>>()
        .join(".")
}

/// Get the bare type name from a fully-qualified proto type.
fn bare_type_name(qualified: &str) -> String {
    qualified
        .trim_start_matches('.')
        .split('.')
        .next_back()
        .unwrap_or(qualified)
        .to_string()
}

/// Resolve a fully-qualified proto type to C# type expression.
/// Returns (type_expression, using_directive_if_needed).
fn resolve_csharp_type(
    qualified_type: &str,
    current_proto_pkg: &str,
    types_namespace: &Option<String>,
    type_to_namespace: &HashMap<String, String>,
) -> (String, Option<String>) {
    let trimmed = qualified_type.trim_start_matches('.');
    let bare = bare_type_name(trimmed);
    let type_proto_pkg = trimmed.rfind('.').map(|pos| &trimmed[..pos]).unwrap_or("");

    if type_proto_pkg == current_proto_pkg || type_proto_pkg.is_empty() {
        let ns = types_namespace
            .as_ref()
            .map(|s| format!("{}.{}", s, bare))
            .unwrap_or_else(|| bare.clone());
        (ns, None)
    } else if let Some(ns) = type_to_namespace.get(trimmed) {
        (format!("{}.{}", ns, bare), Some(format!("using {};", ns)))
    } else if trimmed.starts_with("google.protobuf.") {
        let ns = match bare.as_str() {
            "Empty" => "Google.Protobuf.WellKnownTypes",
            "Timestamp" => "Google.Protobuf.WellKnownTypes",
            "Duration" => "Google.Protobuf.WellKnownTypes",
            _ => "Google.Protobuf",
        };
        (format!("{}.{}", ns, bare), Some(format!("using {};", ns)))
    } else {
        (bare, None)
    }
}

/// Generate C# slimrpc code from a CodeGeneratorRequest
pub fn generate(request: CodeGeneratorRequest) -> Result<CodeGeneratorResponse> {
    let params = common::parse_parameters(request.parameter.as_deref().unwrap_or(""));

    let base_namespace: Option<String> = params
        .get(BASE_NAMESPACE)
        .map(|v| v.trim_matches('"').to_string());

    let types_namespace: Option<String> = params
        .get(TYPES_NAMESPACE)
        .map(|v| v.trim_matches('"').to_string());

    // Build map: fully-qualified proto type -> C# namespace
    let mut type_to_namespace: HashMap<String, String> = HashMap::new();
    for file_desc in &request.proto_file {
        let Some(proto_pkg) = &file_desc.package else {
            continue;
        };
        let ns = package_to_namespace(proto_pkg);
        for msg in &file_desc.message_type {
            if let Some(msg_name) = &msg.name {
                let fqn = format!("{}.{}", proto_pkg, msg_name);
                type_to_namespace.insert(fqn, ns.clone());
            }
        }
    }

    let mut response = CodeGeneratorResponse {
        supported_features: Some(1),
        ..Default::default()
    };

    for file_name in &request.file_to_generate {
        let file_descriptor = request
            .proto_file
            .iter()
            .find(|f| f.name.as_ref() == Some(file_name))
            .context("File descriptor not found")?;

        let package_name = file_descriptor.package.clone().unwrap_or_default();
        let ns = base_namespace
            .clone()
            .unwrap_or_else(|| package_to_namespace(&package_name));
        let types_ns = types_namespace
            .clone()
            .unwrap_or_else(|| package_to_namespace(&package_name));

        let file_base = file_name.strip_suffix(".proto").unwrap_or(file_name);
        let output_file = format!("{}_slimrpc.cs", file_base);

        let mut usings: HashMap<String, String> = HashMap::new();
        usings.insert(
            "Agntcy.Slim.SlimRpc".to_string(),
            "using Agntcy.Slim.SlimRpc;".to_string(),
        );
        usings.insert(
            "uniffi.slim_bindings".to_string(),
            "using uniffi.slim_bindings;".to_string(),
        );
        usings.insert(
            "Google.Protobuf".to_string(),
            "using Google.Protobuf;".to_string(),
        );
        usings.insert("System".to_string(), "using System;".to_string());
        usings.insert(
            "System.Collections.Generic".to_string(),
            "using System.Collections.Generic;".to_string(),
        );
        usings.insert(
            "System.Threading".to_string(),
            "using System.Threading;".to_string(),
        );
        usings.insert(
            "System.Threading.Tasks".to_string(),
            "using System.Threading.Tasks;".to_string(),
        );
        usings.insert(
            "System.Runtime.CompilerServices".to_string(),
            "using System.Runtime.CompilerServices;".to_string(),
        );

        let mut services_found = false;
        let mut service_definitions = String::new();

        for service in &file_descriptor.service {
            services_found = true;
            let service_name = service.name.clone().context("Service name missing")?;

            let mut client_methods = String::new();
            let mut client_impls = String::new();
            let mut server_methods = String::new();
            let mut unimplemented_methods = String::new();
            let mut handler_impls = String::new();
            let mut register_methods = String::new();

            for method in &service.method {
                let method_name = method.name.clone().context("Method name missing")?;
                let raw_input = method.input_type.clone().context("Input type missing")?;
                let raw_output = method.output_type.clone().context("Output type missing")?;

                let (input_type, input_using) = resolve_csharp_type(
                    &raw_input,
                    &package_name,
                    &Some(types_ns.clone()),
                    &type_to_namespace,
                );
                if let Some(u) = input_using {
                    usings.insert(u.clone(), u);
                }

                let (output_type, output_using) = resolve_csharp_type(
                    &raw_output,
                    &package_name,
                    &Some(types_ns.clone()),
                    &type_to_namespace,
                );
                if let Some(u) = output_using {
                    usings.insert(u.clone(), u);
                }

                let is_client_streaming = method.client_streaming.unwrap_or(false);
                let is_server_streaming = method.server_streaming.unwrap_or(false);

                match (is_client_streaming, is_server_streaming) {
                    (false, false) => {
                        client_methods.push_str(&format!(
                            r#"        Task<{}> {}Async({} request, TimeSpan? timeout = null, IReadOnlyDictionary<string, string>? metadata = null, CancellationToken cancellationToken = default);
"#,
                            output_type, method_name, input_type
                        ));
                        client_impls.push_str(&generate_unary_unary_client(
                            &service_name,
                            &method_name,
                            &input_type,
                            &output_type,
                            &package_name,
                        ));
                        server_methods.push_str(&format!(
                            r#"        Task<{}> {}({} request, SlimRpcContext context);
"#,
                            output_type, method_name, input_type
                        ));
                        unimplemented_methods.push_str(&format!(
                            r#"        public Task<{}> {}({} request, SlimRpcContext context) =>
            Task.FromException<{}>(new NotImplementedException("Method {} not implemented"));
"#,
                            output_type, method_name, input_type, output_type, method_name
                        ));
                        handler_impls.push_str(&generate_unary_unary_handler(
                            &service_name,
                            &method_name,
                            &input_type,
                            &output_type,
                        ));
                        register_methods.push_str(&format!(
                            r#"            server.RegisterUnaryUnary("{}.{}", "{}", new {}_{}_Handler(impl));
"#,
                            package_name, service_name, method_name, service_name, method_name
                        ));
                    }
                    (false, true) => {
                        client_methods.push_str(&format!(
                            r#"        IAsyncEnumerable<{}> {}Async({} request, TimeSpan? timeout = null, IReadOnlyDictionary<string, string>? metadata = null, CancellationToken cancellationToken = default);
"#,
                            output_type, method_name, input_type
                        ));
                        client_impls.push_str(&generate_unary_stream_client(
                            &service_name,
                            &method_name,
                            &input_type,
                            &output_type,
                            &package_name,
                        ));
                        server_methods.push_str(&format!(
                            r#"        IAsyncEnumerable<{}> {}({} request, SlimRpcContext context);
"#,
                            output_type, method_name, input_type
                        ));
                        unimplemented_methods.push_str(&format!(
                            r#"        public async IAsyncEnumerable<{}> {}({} request, SlimRpcContext context)
        {{
            await Task.CompletedTask;
            yield break;
        }}
"#,
                            output_type, method_name, input_type
                        ));
                        handler_impls.push_str(&generate_unary_stream_handler(
                            &service_name,
                            &method_name,
                            &input_type,
                            &output_type,
                        ));
                        register_methods.push_str(&format!(
                            r#"            server.RegisterUnaryStream("{}.{}", "{}", new {}_{}_Handler(impl));
"#,
                            package_name, service_name, method_name, service_name, method_name
                        ));
                    }
                    (true, false) => {
                        client_methods.push_str(&format!(
                            r#"        Task<{}> {}Async(IAsyncEnumerable<{}> requestStream, TimeSpan? timeout = null, IReadOnlyDictionary<string, string>? metadata = null, CancellationToken cancellationToken = default);
"#,
                            output_type, method_name, input_type
                        ));
                        client_impls.push_str(&generate_stream_unary_client(
                            &service_name,
                            &method_name,
                            &input_type,
                            &output_type,
                            &package_name,
                        ));
                        server_methods.push_str(&format!(
                            r#"        Task<{}> {}(IAsyncEnumerable<{}> requestStream, SlimRpcContext context);
"#,
                            output_type, method_name, input_type
                        ));
                        unimplemented_methods.push_str(&format!(
                            r#"        public Task<{}> {}(IAsyncEnumerable<{}> requestStream, SlimRpcContext context) =>
            Task.FromException<{}>(new NotImplementedException("Method {} not implemented"));
"#,
                            output_type, method_name, input_type, output_type, method_name
                        ));
                        handler_impls.push_str(&generate_stream_unary_handler(
                            &service_name,
                            &method_name,
                            &input_type,
                            &output_type,
                        ));
                        register_methods.push_str(&format!(
                            r#"            server.RegisterStreamUnary("{}.{}", "{}", new {}_{}_Handler(impl));
"#,
                            package_name, service_name, method_name, service_name, method_name
                        ));
                    }
                    (true, true) => {
                        client_methods.push_str(&format!(
                            r#"        IAsyncEnumerable<{}> {}Async(IAsyncEnumerable<{}> requestStream, TimeSpan? timeout = null, IReadOnlyDictionary<string, string>? metadata = null, CancellationToken cancellationToken = default);
"#,
                            output_type, method_name, input_type
                        ));
                        client_impls.push_str(&generate_stream_stream_client(
                            &service_name,
                            &method_name,
                            &input_type,
                            &output_type,
                            &package_name,
                        ));
                        server_methods.push_str(&format!(
                            r#"        IAsyncEnumerable<{}> {}(IAsyncEnumerable<{}> requestStream, SlimRpcContext context);
"#,
                            output_type, method_name, input_type
                        ));
                        unimplemented_methods.push_str(&format!(
                            r#"        public async IAsyncEnumerable<{}> {}(IAsyncEnumerable<{}> requestStream, SlimRpcContext context)
        {{
            await Task.CompletedTask;
            yield break;
        }}
"#,
                            output_type, method_name, input_type
                        ));
                        handler_impls.push_str(&generate_stream_stream_handler(
                            &service_name,
                            &method_name,
                            &input_type,
                            &output_type,
                        ));
                        register_methods.push_str(&format!(
                            r#"            server.RegisterStreamStream("{}.{}", "{}", new {}_{}_Handler(impl));
"#,
                            package_name, service_name, method_name, service_name, method_name
                        ));
                    }
                }
            }

            let client_interface = format!(
                r#"
    public sealed class {}Client
    {{
        private readonly Channel _channel;

        internal {}Client(Channel channel) => _channel = channel;

{}
    }}
"#,
                service_name,
                service_name,
                client_impls.trim_end()
            );

            let server_interface = format!(
                r#"
    public interface I{}Server
    {{
{}
    }}

    public abstract class Unimplemented{}Server : I{}Server
    {{
{}
    }}

    public static class {}ServerRegistration
    {{
        internal static void Register{}Server(Server server, I{}Server impl)
        {{
{}
        }}
    }}
{}
"#,
                service_name,
                server_methods.trim_end(),
                service_name,
                service_name,
                unimplemented_methods.trim_end(),
                service_name,
                service_name,
                service_name,
                register_methods.trim_end(),
                handler_impls.trim_end()
            );

            service_definitions.push_str(&client_interface);
            service_definitions.push_str(&server_interface);
        }

        if services_found {
            let mut using_lines: Vec<_> = usings.values().cloned().collect();
            using_lines.sort();
            let usings_block = using_lines.join("\n");

            let content = format!(
                r#"// Code generated by protoc-gen-slimrpc-csharp. DO NOT EDIT.
// source: {}

{}
namespace {};

{}
"#,
                file_name,
                usings_block,
                ns,
                service_definitions.trim_end()
            );

            response.file.push(File {
                name: Some(output_file),
                insertion_point: None,
                content: Some(content),
                generated_code_info: None,
            });
        }
    }

    Ok(response)
}

fn generate_unary_unary_client(
    service_name: &str,
    method_name: &str,
    input_type: &str,
    output_type: &str,
    package_name: &str,
) -> String {
    format!(
        r#"
        public async Task<{}> {}Async({} request, TimeSpan? timeout = null, IReadOnlyDictionary<string, string>? metadata = null, CancellationToken cancellationToken = default)
        {{
            cancellationToken.ThrowIfCancellationRequested();
            var reqBytes = request.ToByteString().ToByteArray();
            var respBytes = await _channel.CallUnaryAsync("{}.{}", "{}", reqBytes, timeout, metadata != null ? new Dictionary<string, string>(metadata) : null);
            return {}.Parser.ParseFrom(respBytes);
        }}
"#,
        output_type, method_name, input_type, package_name, service_name, method_name, output_type
    )
}

fn generate_unary_stream_client(
    service_name: &str,
    method_name: &str,
    input_type: &str,
    output_type: &str,
    package_name: &str,
) -> String {
    format!(
        r#"
        public async IAsyncEnumerable<{}> {}Async({} request, TimeSpan? timeout = null, IReadOnlyDictionary<string, string>? metadata = null, [System.Runtime.CompilerServices.EnumeratorCancellation] CancellationToken cancellationToken = default)
        {{
            var reqBytes = request.ToByteString().ToByteArray();
            var stream = await _channel.CallUnaryStreamAsync("{}.{}", "{}", reqBytes, timeout, metadata != null ? new Dictionary<string, string>(metadata) : null);
            await foreach (var msg in SlimRpcStreams.ReadResponseStreamAsync<{}>(stream, cancellationToken))
            {{
                yield return msg;
            }}
        }}
"#,
        output_type, method_name, input_type, package_name, service_name, method_name, output_type
    )
}

fn generate_stream_unary_client(
    service_name: &str,
    method_name: &str,
    input_type: &str,
    output_type: &str,
    package_name: &str,
) -> String {
    format!(
        r#"
        public async Task<{}> {}Async(IAsyncEnumerable<{}> requestStream, TimeSpan? timeout = null, IReadOnlyDictionary<string, string>? metadata = null, CancellationToken cancellationToken = default)
        {{
            var writer = _channel.CallStreamUnary("{}.{}", "{}", timeout, metadata != null ? new Dictionary<string, string>(metadata) : null);
            await foreach (var req in requestStream.WithCancellation(cancellationToken))
            {{
                await writer.SendAsync(req.ToByteString().ToByteArray());
            }}
            var respBytes = await writer.FinalizeAsync();
            return {}.Parser.ParseFrom(respBytes);
        }}
"#,
        output_type, method_name, input_type, package_name, service_name, method_name, output_type
    )
}

fn generate_stream_stream_client(
    service_name: &str,
    method_name: &str,
    input_type: &str,
    output_type: &str,
    package_name: &str,
) -> String {
    format!(
        r#"
        public async IAsyncEnumerable<{}> {}Async(IAsyncEnumerable<{}> requestStream, TimeSpan? timeout = null, IReadOnlyDictionary<string, string>? metadata = null, [System.Runtime.CompilerServices.EnumeratorCancellation] CancellationToken cancellationToken = default)
        {{
            var bidi = _channel.CallStreamStream("{}.{}", "{}", timeout, metadata != null ? new Dictionary<string, string>(metadata) : null);
            var sendTask = Task.Run(async () =>
            {{
                await foreach (var req in requestStream.WithCancellation(cancellationToken))
                {{
                    await bidi.SendAsync(req.ToByteString().ToByteArray());
                }}
                await bidi.CloseSendAsync();
            }});
            try
            {{
                await foreach (var msg in SlimRpcStreams.ReadBidiStreamAsync<{}>(bidi, cancellationToken))
                {{
                    yield return msg;
                }}
            }}
            finally
            {{
                await sendTask;
            }}
        }}
"#,
        output_type, method_name, input_type, package_name, service_name, method_name, output_type
    )
}

fn generate_unary_unary_handler(
    service_name: &str,
    method_name: &str,
    input_type: &str,
    _output_type: &str,
) -> String {
    format!(
        r#"
    internal sealed class {}_{}_Handler : UnaryUnaryHandler
    {{
        private readonly I{}Server _impl;

        public {}_{}_Handler(I{}Server impl) => _impl = impl;

        public async Task<byte[]> Handle(byte[] request, Context context)
        {{
            var req = {}.Parser.ParseFrom(request);
            var ctx = SlimRpcContext.FromContext(context);
            try
            {{
                var resp = await _impl.{}(req, ctx);
                return resp.ToByteString().ToByteArray();
            }}
            catch (RpcException)
            {{
                throw;
            }}
            catch (Exception ex)
            {{
                throw new RpcException.Rpc(RpcCode.Internal, ex.Message, null);
            }}
        }}
    }}
"#,
        service_name,
        method_name,
        service_name,
        service_name,
        method_name,
        service_name,
        input_type,
        method_name
    )
}

fn generate_unary_stream_handler(
    service_name: &str,
    method_name: &str,
    input_type: &str,
    _output_type: &str,
) -> String {
    format!(
        r#"
    internal sealed class {}_{}_Handler : UnaryStreamHandler
    {{
        private readonly I{}Server _impl;

        public {}_{}_Handler(I{}Server impl) => _impl = impl;

        public async Task Handle(byte[] request, Context context, ResponseSink sink)
        {{
            var req = {}.Parser.ParseFrom(request);
            var ctx = SlimRpcContext.FromContext(context);
            try
            {{
                await foreach (var resp in _impl.{}(req, ctx))
                {{
                    await sink.SendAsync(resp.ToByteString().ToByteArray());
                }}
                await sink.CloseAsync();
            }}
            catch (RpcException ex)
            {{
                await sink.SendErrorAsync(ex);
            }}
            catch (Exception ex)
            {{
                await sink.SendErrorAsync(new RpcException.Rpc(RpcCode.Internal, ex.Message, null));
            }}
        }}
    }}
"#,
        service_name,
        method_name,
        service_name,
        service_name,
        method_name,
        service_name,
        input_type,
        method_name
    )
}

fn generate_stream_unary_handler(
    service_name: &str,
    method_name: &str,
    input_type: &str,
    _output_type: &str,
) -> String {
    format!(
        r#"
    internal sealed class {}_{}_Handler : StreamUnaryHandler
    {{
        private readonly I{}Server _impl;

        public {}_{}_Handler(I{}Server impl) => _impl = impl;

        public async Task<byte[]> Handle(RequestStream stream, Context context)
        {{
            var ctx = SlimRpcContext.FromContext(context);
            var reqStream = SlimRpcStreams.ReadRequestStreamAsync<{}>(stream);
            try
            {{
                var resp = await _impl.{}(reqStream, ctx);
                return resp.ToByteString().ToByteArray();
            }}
            catch (RpcException)
            {{
                throw;
            }}
            catch (Exception ex)
            {{
                throw new RpcException.Rpc(RpcCode.Internal, ex.Message, null);
            }}
        }}
    }}
"#,
        service_name,
        method_name,
        service_name,
        service_name,
        method_name,
        service_name,
        input_type,
        method_name
    )
}

fn generate_stream_stream_handler(
    service_name: &str,
    method_name: &str,
    input_type: &str,
    _output_type: &str,
) -> String {
    format!(
        r#"
    internal sealed class {}_{}_Handler : StreamStreamHandler
    {{
        private readonly I{}Server _impl;

        public {}_{}_Handler(I{}Server impl) => _impl = impl;

        public async Task Handle(RequestStream stream, Context context, ResponseSink sink)
        {{
            var ctx = SlimRpcContext.FromContext(context);
            var reqStream = SlimRpcStreams.ReadRequestStreamAsync<{}>(stream);
            try
            {{
                await foreach (var resp in _impl.{}(reqStream, ctx))
                {{
                    await sink.SendAsync(resp.ToByteString().ToByteArray());
                }}
                await sink.CloseAsync();
            }}
            catch (RpcException ex)
            {{
                await sink.SendErrorAsync(ex);
            }}
            catch (Exception ex)
            {{
                await sink.SendErrorAsync(new RpcException.Rpc(RpcCode.Internal, ex.Message, null));
            }}
        }}
    }}
"#,
        service_name,
        method_name,
        service_name,
        service_name,
        method_name,
        service_name,
        input_type,
        method_name
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost_types::compiler::CodeGeneratorRequest;
    use prost_types::{FileDescriptorProto, MethodDescriptorProto, ServiceDescriptorProto};
    use std::collections::HashMap;

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

    #[test]
    fn test_package_to_namespace() {
        assert_eq!(package_to_namespace("example_service"), "ExampleService");
        assert_eq!(package_to_namespace("foo.bar.example"), "Foo.Bar.Example");
    }

    #[test]
    fn test_bare_type_name() {
        assert_eq!(bare_type_name(".pkg.Request"), "Request");
        assert_eq!(bare_type_name("Request"), "Request");
        assert_eq!(bare_type_name(".foo.bar.Response"), "Response");
    }

    #[test]
    fn test_resolve_csharp_type_same_package() {
        let type_to_ns: HashMap<String, String> = HashMap::new();
        let (ty, using) = resolve_csharp_type(
            ".example_service.GetUserRequest",
            "example_service",
            &Some("ExampleService.Types".to_string()),
            &type_to_ns,
        );
        assert_eq!(ty, "ExampleService.Types.GetUserRequest");
        assert_eq!(using, None);
    }

    #[test]
    fn test_resolve_csharp_type_different_package() {
        let mut type_to_ns = HashMap::new();
        type_to_ns.insert("other.pkg.OtherRequest".to_string(), "OtherPkg".to_string());
        let (ty, using) = resolve_csharp_type(
            ".other.pkg.OtherRequest",
            "example_service",
            &Some("ExampleService.Types".to_string()),
            &type_to_ns,
        );
        assert_eq!(ty, "OtherPkg.OtherRequest");
        assert_eq!(using, Some("using OtherPkg;".to_string()));
    }

    #[test]
    fn test_resolve_csharp_type_google_protobuf() {
        let type_to_ns: HashMap<String, String> = HashMap::new();
        let (ty, using) = resolve_csharp_type(
            ".google.protobuf.Empty",
            "example_service",
            &None,
            &type_to_ns,
        );
        assert_eq!(ty, "Google.Protobuf.WellKnownTypes.Empty");
        assert!(using.is_some());
    }

    #[test]
    fn test_generate_no_services() {
        let request = CodeGeneratorRequest {
            file_to_generate: vec!["test.proto".to_string()],
            parameter: None,
            proto_file: vec![create_test_file_descriptor(
                "test.proto",
                "test.package",
                vec![],
            )],
            compiler_version: None,
        };

        let response = generate(request).unwrap();
        assert_eq!(response.file.len(), 0);
    }

    #[test]
    fn test_generate_with_unary_unary_method() {
        let method = create_test_method(
            "GetUser",
            ".example_service.GetUserRequest",
            ".example_service.GetUserResponse",
            false,
            false,
        );
        let service = create_test_service("UserService", vec![method]);
        let file_descriptor =
            create_test_file_descriptor("user.proto", "example_service", vec![service]);

        let request = CodeGeneratorRequest {
            file_to_generate: vec!["user.proto".to_string()],
            parameter: None,
            proto_file: vec![file_descriptor],
            compiler_version: None,
        };

        let response = generate(request).unwrap();

        assert_eq!(response.file.len(), 1);
        let generated_file = &response.file[0];
        assert_eq!(generated_file.name.as_ref().unwrap(), "user_slimrpc.cs");

        let content = generated_file.content.as_ref().unwrap();
        assert!(content.contains("UserServiceClient"));
        assert!(content.contains("IUserServiceServer"));
        assert!(content.contains("GetUserAsync"));
        assert!(content.contains("GetUserResponse"));
        assert!(content.contains("GetUser("));
        assert!(content.contains("RegisterUnaryUnary"));
    }

    #[test]
    fn test_generate_with_streaming_methods() {
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
        let file_descriptor = create_test_file_descriptor("stream.proto", "pkg", vec![service]);

        let request = CodeGeneratorRequest {
            file_to_generate: vec!["stream.proto".to_string()],
            parameter: None,
            proto_file: vec![file_descriptor],
            compiler_version: None,
        };

        let response = generate(request).unwrap();

        assert_eq!(response.file.len(), 1);
        let content = response.file[0].content.as_ref().unwrap();
        assert!(content.contains("IAsyncEnumerable"));
        assert!(content.contains("RegisterUnaryStream"));
        assert!(content.contains("RegisterStreamUnary"));
        assert!(content.contains("RegisterStreamStream"));
    }

    #[test]
    fn test_generate_with_base_namespace_param() {
        let method = create_test_method(
            "Echo",
            ".pkg.EchoRequest",
            ".pkg.EchoResponse",
            false,
            false,
        );
        let service = create_test_service("EchoService", vec![method]);
        let file_descriptor = create_test_file_descriptor("echo.proto", "pkg", vec![service]);

        let request = CodeGeneratorRequest {
            file_to_generate: vec!["echo.proto".to_string()],
            parameter: Some(r#"base_namespace="Custom.Namespace""#.to_string()),
            proto_file: vec![file_descriptor],
            compiler_version: None,
        };

        let response = generate(request).unwrap();

        assert_eq!(response.file.len(), 1);
        let content = response.file[0].content.as_ref().unwrap();
        assert!(content.contains("namespace Custom.Namespace"));
    }
}
