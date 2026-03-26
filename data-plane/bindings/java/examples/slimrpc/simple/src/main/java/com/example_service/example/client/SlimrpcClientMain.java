// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package com.example_service.example.client;

import io.agntcy.slim.examples.common.Common;
import com.example_service.ExampleRequest;
import com.example_service.ExampleResponse;
import com.example_service.TestSlimrpc;
import io.agntcy.slim.bindings.App;
import io.agntcy.slim.bindings.Channel;
import io.agntcy.slim.bindings.ClientConfig;
import io.agntcy.slim.bindings.Name;
import io.agntcy.slim.bindings.ResponseStreamReader;
import io.agntcy.slim.bindings.Service;
import io.agntcy.slim.bindings.SlimBindings;
import io.agntcy.slim.bindings.StreamMessage;
import io.agntcy.slim.bindings.slimrpc.ClientBidiStream;
import io.agntcy.slim.bindings.slimrpc.ClientRequestStream;
import io.agntcy.slim.bindings.slimrpc.ClientResponseStream;

import java.time.Duration;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;

public final class SlimrpcClientMain {
    public static void main(String[] args) throws Exception {
        SlimBindings.initializeWithDefaults();
        Service service = SlimBindings.getGlobalService();

        Name localName = new Name(Common.NAME_ORG, Common.NAME_NS, "client");
        Name remoteName = new Name(Common.NAME_ORG, Common.NAME_NS, "server");

        App app = service.createAppWithSecret(localName, Common.DEFAULT_SHARED_SECRET);
        ClientConfig clientConfig = SlimBindings.newInsecureClientConfig(Common.DEFAULT_SERVER_ENDPOINT);
        long connId = service.connect(clientConfig);
        app.subscribe(localName, connId);

        Channel channel = Channel.newWithConnection(app, remoteName, connId);

        TestSlimrpc.TestClient client = new TestSlimrpc.TestClientImpl(channel);

        ExampleRequest request = ExampleRequest.newBuilder()
                .setExampleString("Alice")
                .setExampleInteger(42)
                .build();

        System.out.println("=== Unary-Unary ===");
        ExampleResponse response = client.ExampleUnaryUnary(request, Duration.ofSeconds(10), null);
        System.out.println("Response: " + response);

        System.out.println("=== Unary-Stream ===");
        ResponseStreamReader streamReader = client.ExampleUnaryStream(request, Duration.ofSeconds(10), null);
        ClientResponseStream<ExampleResponse> responseStream = ClientResponseStream.create(streamReader,
                bytes -> {
                    try {
                        return ExampleResponse.parseFrom(bytes);
                    } catch (Exception e) {
                        throw new RuntimeException(e);
                    }
                });
        while (true) {
            ExampleResponse streamResp;
            try {
                streamResp = responseStream.recv();
            } catch (Exception e) {
                throw e;
            }
            if (streamResp == null) {
                System.out.println("Stream ended");
                break;
            }
            System.out.println("Stream Response: " + streamResp);
        }

        System.out.println("=== Stream-Unary ===");
        ClientRequestStream<ExampleRequest, ExampleResponse> streamUnary = client
                .ExampleStreamUnary(Duration.ofSeconds(10), null);
        String[] names = {"Alice", "Bob", "Charlie", "Diana", "Eve"};
        for (int i = 0; i < names.length; i++) {
            ExampleRequest req = ExampleRequest.newBuilder()
                    .setExampleString(names[i])
                    .setExampleInteger(i + 1)
                    .build();
            streamUnary.send(req);
        }
        ExampleResponse streamUnaryResp = streamUnary.finalizeStream();
        System.out.println("Stream Unary Response: " + streamUnaryResp);

        System.out.println("=== Stream-Stream ===");
        ClientBidiStream<ExampleRequest> streamStream = client.ExampleStreamStream(Duration.ofSeconds(10), null);

        CompletableFuture<Void> sendFuture = CompletableFuture.runAsync(() -> {
            try {
                for (int i = 0; i < names.length; i++) {
                    ExampleRequest req = ExampleRequest.newBuilder()
                            .setExampleString(names[i])
                            .setExampleInteger((i + 1) * 10)
                            .build();
                    streamStream.send(req);
                    System.out.println("Sent request " + names[i]);
                    Thread.sleep(50);
                }
                streamStream.closeSend();
                System.out.println("Send stream closed");
            } catch (Exception e) {
                System.out.println("Send error: " + e.getMessage());
                throw new RuntimeException(e);
            }
        });

        while (true) {
            StreamMessage msg = streamStream.recv();
            if (msg instanceof StreamMessage.End) {
                System.out.println("Stream Stream ended");
                break;
            }
            if (msg instanceof StreamMessage.Error err) {
                throw err.v1();
            }
            if (msg instanceof StreamMessage.Data data) {
                ExampleResponse streamResp = ExampleResponse.parseFrom(data.v1());
                System.out.println("Stream Stream Response: " + streamResp);
            }
        }

        sendFuture.join();
    }
}
