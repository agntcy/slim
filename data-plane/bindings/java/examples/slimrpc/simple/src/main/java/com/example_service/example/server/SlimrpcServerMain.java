// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package com.example_service.example.server;

import io.agntcy.slim.examples.common.Common;
import com.example_service.ExampleRequest;
import com.example_service.ExampleResponse;
import com.example_service.TestSlimrpc;
import io.agntcy.slim.bindings.App;
import io.agntcy.slim.bindings.ClientConfig;
import io.agntcy.slim.bindings.Context;
import io.agntcy.slim.bindings.Name;
import io.agntcy.slim.bindings.RequestStream;
import io.agntcy.slim.bindings.ResponseSink;
import io.agntcy.slim.bindings.RuntimeConfig;
import io.agntcy.slim.bindings.Server;
import io.agntcy.slim.bindings.ServerConfig;
import io.agntcy.slim.bindings.ServiceConfig;
import io.agntcy.slim.bindings.Service;
import io.agntcy.slim.bindings.SlimBindings;
import io.agntcy.slim.bindings.TracingConfig;
import io.agntcy.slim.bindings.slimrpc.ServerBidiStream;
import io.agntcy.slim.bindings.slimrpc.ServerRequestStream;
import io.agntcy.slim.bindings.slimrpc.ServerResponseStream;

import java.util.concurrent.CompletableFuture;

public final class SlimrpcServerMain {

    public static void main(String[] args) throws Exception {
        // Parse --instance flag (default: "server")
        String instance = "server";
        for (int i = 0; i < args.length; i++) {
            if ("--instance".equals(args[i]) && i + 1 < args.length) {
                instance = args[i + 1];
            }
        }

        RuntimeConfig runtime = SlimBindings.newRuntimeConfig();
        TracingConfig tracing = SlimBindings.newTracingConfigWith(
                "debug",
                Boolean.TRUE,
                Boolean.FALSE,
                java.util.List.of());

        ServiceConfig serviceConfig = SlimBindings.newServiceConfig();
        SlimBindings.initializeWithConfigs(runtime, tracing, java.util.List.of(serviceConfig));

        Service service = SlimBindings.getGlobalService();

        Name localName = new Name(Common.NAME_ORG, Common.NAME_NS, instance);
        App app = service.createAppWithSecret(localName, Common.DEFAULT_SHARED_SECRET);

        ClientConfig clientConfig = SlimBindings.newInsecureClientConfig(Common.DEFAULT_SERVER_ENDPOINT);
        long connId = service.connect(clientConfig);
        app.subscribe(localName, connId);

        Server rpcServer = Server.newWithConnection(app, localName, connId);
        TestSlimrpc.registerTestServer(rpcServer, new TestSlimrpc.UnimplementedTestServer() {
            @Override
            public CompletableFuture<ExampleResponse> ExampleUnaryUnary(ExampleRequest request, Context context) {
                ExampleResponse response = ExampleResponse.newBuilder()
                        .setExampleString("Hello, " + request.getExampleString() + "!")
                        .setExampleInteger(request.getExampleInteger() * 2)
                        .build();
                return CompletableFuture.completedFuture(response);
            }

            @Override
            public CompletableFuture<Void> ExampleUnaryStream(ExampleRequest request, Context context,
                    ResponseSink sink) {
                ServerRequestStream<ExampleResponse> stream = ServerRequestStream.create(
                        sink,
                        ExampleResponse::toByteArray);
                for (long i = 1; i <= 3; i++) {
                    ExampleResponse response = ExampleResponse.newBuilder()
                            .setExampleString(request.getExampleString() + " reply #" + i)
                            .setExampleInteger(request.getExampleInteger() * i)
                            .build();
                    try {
                        stream.send(response);
                    } catch (Exception e) {
                        return CompletableFuture.failedFuture(e);
                    }
                }
                return CompletableFuture.completedFuture(null);
            }

            @Override
            public CompletableFuture<ExampleResponse> ExampleStreamUnary(RequestStream stream, Context context) {
                ServerResponseStream<ExampleRequest> requestStream = ServerResponseStream.create(
                        stream,
                        bytes -> {
                            try {
                                return ExampleRequest.parseFrom(bytes);
                            } catch (Exception e) {
                                throw new RuntimeException(e);
                            }
                        });
                StringBuilder names = new StringBuilder();
                long sum = 0;
                while (true) {
                    ExampleRequest req;
                    try {
                        req = requestStream.recv();
                    } catch (Exception e) {
                        return CompletableFuture.failedFuture(e);
                    }
                    if (req == null) {
                        break;
                    }
                    if (!names.isEmpty()) {
                        names.append(", ");
                    }
                    names.append(req.getExampleString());
                    sum += req.getExampleInteger();
                }
                ExampleResponse response = ExampleResponse.newBuilder()
                        .setExampleString("Received from: " + names)
                        .setExampleInteger(sum)
                        .build();
                return CompletableFuture.completedFuture(response);
            }

            @Override
            public CompletableFuture<Void> ExampleStreamStream(RequestStream stream, Context context,
                    ResponseSink sink) {
                ServerBidiStream<ExampleRequest, ExampleResponse> bidiStream = ServerBidiStream.create(
                        stream,
                        sink,
                        bytes -> {
                            try {
                                return ExampleRequest.parseFrom(bytes);
                            } catch (Exception e) {
                                throw new RuntimeException(e);
                            }
                        },
                        ExampleResponse::toByteArray);
                while (true) {
                    ExampleRequest req;
                    try {
                        req = bidiStream.recv();
                    } catch (Exception e) {
                        return CompletableFuture.failedFuture(e);
                    }
                    if (req == null) {
                        break;
                    }
                    ExampleResponse response = ExampleResponse.newBuilder()
                            .setExampleString("Hello, " + req.getExampleString() + "!")
                            .setExampleInteger(req.getExampleInteger() * 2)
                            .build();
                    try {
                        bidiStream.send(response);
                    } catch (Exception e) {
                        return CompletableFuture.failedFuture(e);
                    }
                }
                return CompletableFuture.completedFuture(null);
            }
        });

        System.out.println("SlimRPC server '" + instance + "' ready on " + Common.DEFAULT_SERVER_ENDPOINT);
        rpcServer.serve();
    }
}
