// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package com.example_service.example;

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

import java.util.concurrent.CompletableFuture;

public final class SlimrpcServerMain {
    private static final String SERVER_ADDR = "127.0.0.1:46357";
    private static final String SHARED_SECRET = "my_shared_secret_for_testing_purposes_only";

    public static void main(String[] args) throws Exception {
        // SlimBindings.initializeWithDefaults();
        RuntimeConfig runtime = SlimBindings.newRuntimeConfig();
        TracingConfig tracing = SlimBindings.newTracingConfigWith(
                "debug",
                Boolean.TRUE,   // displayThreadNames
                Boolean.FALSE,  // displayThreadIds
                java.util.List.of() // filters
        );

        ServiceConfig serviceConfig = SlimBindings.newServiceConfig();
        SlimBindings.initializeWithConfigs(runtime, tracing, java.util.List.of(serviceConfig));


        //
        Service service = SlimBindings.getGlobalService();

        // Start SLIM server
        ServerConfig serverConfig = SlimBindings.newInsecureServerConfig(SERVER_ADDR);
        service.runServer(serverConfig);

        Name localName = new Name("agntcy", "slimrpc", "server");
        App app = service.createAppWithSecret(localName, SHARED_SECRET);

        ClientConfig clientConfig = SlimBindings.newInsecureClientConfig("http://" + SERVER_ADDR);
        long connId = service.connect(clientConfig);
        app.subscribe(localName, connId);

        Server rpcServer = Server.newWithConnection(app, localName, connId);
        TestSlimrpc.registerTestServer(rpcServer, new TestSlimrpc.UnimplementedTestServer() {
            @Override
            public CompletableFuture<ExampleResponse> ExampleUnaryUnary(ExampleRequest request, Context context) {
                ExampleResponse response = ExampleResponse.newBuilder()
                        .setExampleString("hello " + request.getExampleString())
                        .setExampleInteger(request.getExampleInteger() + 1)
                        .build();
                return CompletableFuture.completedFuture(response);
            }

            @Override
            public CompletableFuture<Void> ExampleUnaryStream(ExampleRequest request, Context context, ResponseSink sink) {
                TestSlimrpc.ServerRequestStreamSync<ExampleResponse> stream = TestSlimrpc.newServerRequestStreamSync(
                        sink,
                        ExampleResponse::toByteArray
                );
                for (long i = 0; i < 5; i++) {
                    ExampleResponse response = ExampleResponse.newBuilder()
                            .setExampleString("hello " + request.getExampleString())
                            .setExampleInteger(request.getExampleInteger() + i)
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
                TestSlimrpc.ServerResponseStreamSync<ExampleRequest> requestStream = TestSlimrpc.newServerResponseStreamSync(
                        stream,
                        bytes -> {
                            try {
                                return ExampleRequest.parseFrom(bytes);
                            } catch (Exception e) {
                                throw new RuntimeException(e);
                            }
                        }
                );
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
                    sum += req.getExampleInteger();
                }
                ExampleResponse response = ExampleResponse.newBuilder()
                        .setExampleString("hello stream")
                        .setExampleInteger(sum)
                        .build();
                return CompletableFuture.completedFuture(response);
            }

            @Override
            public CompletableFuture<Void> ExampleStreamStream(RequestStream stream, Context context, ResponseSink sink) {
                TestSlimrpc.ServerBidiStreamSync<ExampleRequest, ExampleResponse> bidiStream = TestSlimrpc.newServerBidiStreamSync(
                        stream,
                        sink,
                        bytes -> {
                            try {
                                return ExampleRequest.parseFrom(bytes);
                            } catch (Exception e) {
                                throw new RuntimeException(e);
                            }
                        },
                        ExampleResponse::toByteArray
                );
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
                            .setExampleString("echo " + req.getExampleString())
                            .setExampleInteger(req.getExampleInteger() * 100)
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

        System.out.println("SlimRPC server ready on " + SERVER_ADDR);
        rpcServer.serve();
    }
}
