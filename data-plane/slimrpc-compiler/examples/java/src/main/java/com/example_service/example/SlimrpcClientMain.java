// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package com.example_service.example;

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

import java.time.Duration;

public final class SlimrpcClientMain {
    private static final String SERVER_ADDR = "127.0.0.1:46357";
    private static final String SHARED_SECRET = "my_shared_secret_for_testing_purposes_only";

    public static void main(String[] args) throws Exception {
        SlimBindings.initializeWithDefaults();
        Service service = SlimBindings.getGlobalService();

        Name localName = new Name("agntcy", "slimrpc", "client");
        Name remoteName = new Name("agntcy", "slimrpc", "server");

        App app = service.createAppWithSecret(localName, SHARED_SECRET);
        ClientConfig clientConfig = SlimBindings.newInsecureClientConfig("http://" + SERVER_ADDR);
        long connId = service.connect(clientConfig);
        app.subscribe(localName, connId);

        Channel channel = Channel.newWithConnection(app, remoteName, connId);

        TestSlimrpc.TestClientSync client = new TestSlimrpc.TestClientSyncImpl(channel);

        ExampleRequest request = ExampleRequest.newBuilder()
                .setExampleString("world")
                .setExampleInteger(41)
                .build();

        System.out.println("=== Unary-Unary ===");
        ExampleResponse response = client.ExampleUnaryUnary(request, Duration.ofSeconds(10), null);
        System.out.println("Response: " + response);

        System.out.println("=== Unary-Stream ===");
        ResponseStreamReader streamReader = client.ExampleUnaryStream(request, Duration.ofSeconds(10), null);
        TestSlimrpc.ClientResponseStreamSync<ExampleResponse> responseStream =
                TestSlimrpc.newClientResponseStreamSync(streamReader, bytes -> {
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
        TestSlimrpc.ClientRequestStreamSync<ExampleRequest, ExampleResponse> streamUnary =
                client.ExampleStreamUnary(Duration.ofSeconds(10), null);
        for (long i = 0; i < 5; i++) {
            ExampleRequest req = ExampleRequest.newBuilder()
                    .setExampleString("world")
                    .setExampleInteger(i)
                    .build();
            streamUnary.send(req);
        }
        ExampleResponse streamUnaryResp = streamUnary.finalizeStream();
        System.out.println("Stream Unary Response: " + streamUnaryResp);

        System.out.println("=== Stream-Stream ===");
        TestSlimrpc.ClientBidiStreamSync<ExampleRequest> streamStream =
                client.ExampleStreamStream(Duration.ofSeconds(10), null);
        for (long i = 0; i < 5; i++) {
            ExampleRequest req = ExampleRequest.newBuilder()
                    .setExampleString("request " + i)
                    .setExampleInteger(i)
                    .build();
            streamStream.send(req);
        }
        streamStream.closeSend();
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
    }
}
