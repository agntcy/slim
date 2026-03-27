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
import io.agntcy.slim.bindings.MulticastStreamMessage;
import io.agntcy.slim.bindings.Name;
import io.agntcy.slim.bindings.RpcMulticastItem;
import io.agntcy.slim.bindings.Service;
import io.agntcy.slim.bindings.SlimBindings;
import io.agntcy.slim.bindings.slimrpc.MulticastClientBidiStream;
import io.agntcy.slim.bindings.slimrpc.MulticastResponseStream;

import java.time.Duration;
import java.util.ArrayList;
import java.util.List;

/**
 * Multicast (group) RPC client example.
 *
 * <p>Demonstrates broadcasting RPC calls to multiple server instances using the
 * generated {@link TestSlimrpc.TestGroupClient} stub. All four streaming patterns
 * are exercised: unary-unary, unary-stream, stream-unary, and stream-stream.
 *
 * <p>Start two server instances (server1 and server2) before running this client.
 */
public final class SlimrpcGroupClientMain {

    public static void main(String[] args) throws Exception {
        String serverAddr = Common.getServerEndpoint();
        String serversStr = "server1,server2";
        for (int i = 0; i < args.length; i++) {
            if ("--server".equals(args[i]) && i + 1 < args.length) {
                serverAddr = args[i + 1];
            } else if ("--servers".equals(args[i]) && i + 1 < args.length) {
                serversStr = args[i + 1];
            }
        }

        List<Name> serverNames = new ArrayList<>();
        for (String s : serversStr.split(",")) {
            serverNames.add(new Name(Common.NAME_ORG, Common.NAME_NS, s.trim()));
        }

        SlimBindings.initializeWithDefaults();
        Service service = SlimBindings.getGlobalService();

        Name localName = new Name(Common.NAME_ORG, Common.NAME_NS, "client");
        App app = service.createAppWithSecret(localName, Common.DEFAULT_SHARED_SECRET);

        ClientConfig clientConfig = SlimBindings.newInsecureClientConfig(serverAddr);
        long connId = service.connect(clientConfig);
        app.subscribe(app.name(), connId);

        // Group channel targeting all server instances
        Channel channel = Channel.newGroupWithConnection(app, serverNames, connId);
        TestSlimrpc.TestGroupClient client = new TestSlimrpc.TestGroupClientImpl(channel);

        try {
            System.out.println("SLIM_RPC_GROUP_CLIENT_STARTED");
            runMulticastUnary(client);
            runMulticastUnaryStream(client);
            runMulticastStreamUnary(client);
            runMulticastStreamStream(client);
            System.out.println("SLIM_RPC_GROUP_CLIENT_DONE");
        } catch (Exception e) {
            System.err.println("Group client error: " + e.getMessage());
            throw e;
        } finally {
            channel.closeAsync(null);
        }
    }

    // ---- Multicast Unary-Unary ----

    private static void runMulticastUnary(TestSlimrpc.TestGroupClient client) throws Exception {
        System.out.println("=== Multicast Unary-Unary ===");
        ExampleRequest request = ExampleRequest.newBuilder()
                .setExampleString("hello")
                .setExampleInteger(1)
                .build();

        MulticastResponseStream<ExampleResponse> stream =
                client.ExampleUnaryUnary(request, Duration.ofSeconds(5), null);

        while (true) {
            MulticastStreamMessage msg = stream.next();
            if (msg instanceof MulticastStreamMessage.End) break;
            if (msg instanceof MulticastStreamMessage.Error err) {
                System.out.println("  Error: " + err.error());
                break;
            }
            if (msg instanceof MulticastStreamMessage.Data data) {
                RpcMulticastItem item = data.item();
                ExampleResponse resp = ExampleResponse.parseFrom(item.message());
                System.out.println("  [" + item.context().source().toString() + "] " + resp);
            }
        }
    }

    // ---- Multicast Unary-Stream ----

    private static void runMulticastUnaryStream(TestSlimrpc.TestGroupClient client) throws Exception {
        System.out.println("\n=== Multicast Unary-Stream ===");
        ExampleRequest request = ExampleRequest.newBuilder()
                .setExampleString("hello")
                .setExampleInteger(1)
                .build();

        MulticastResponseStream<ExampleResponse> stream =
                client.ExampleUnaryStream(request, Duration.ofSeconds(5), null);

        while (true) {
            MulticastStreamMessage msg = stream.next();
            if (msg instanceof MulticastStreamMessage.End) break;
            if (msg instanceof MulticastStreamMessage.Error err) {
                System.out.println("  Error: " + err.error());
                break;
            }
            if (msg instanceof MulticastStreamMessage.Data data) {
                RpcMulticastItem item = data.item();
                ExampleResponse resp = ExampleResponse.parseFrom(item.message());
                System.out.println("  [" + item.context().source().toString() + "] " + resp);
            }
        }
    }

    // ---- Multicast Stream-Unary ----

    private static void runMulticastStreamUnary(TestSlimrpc.TestGroupClient client) throws Exception {
        System.out.println("\n=== Multicast Stream-Unary ===");

        MulticastClientBidiStream<ExampleRequest, ExampleResponse> stream =
                client.ExampleStreamUnary(Duration.ofSeconds(5), null);

        // Send requests
        for (int i = 0; i < 3; i++) {
            ExampleRequest req = ExampleRequest.newBuilder()
                    .setExampleString("item " + i)
                    .setExampleInteger(i)
                    .build();
            stream.send(req);
        }
        stream.closeSend();

        // Receive multicast responses
        while (true) {
            MulticastStreamMessage msg = stream.recv();
            if (msg instanceof MulticastStreamMessage.End) break;
            if (msg instanceof MulticastStreamMessage.Error err) {
                System.out.println("  Error: " + err.error());
                break;
            }
            if (msg instanceof MulticastStreamMessage.Data data) {
                RpcMulticastItem item = data.item();
                ExampleResponse resp = ExampleResponse.parseFrom(item.message());
                System.out.println("  [" + item.context().source().toString() + "] " + resp);
            }
        }
    }

    // ---- Multicast Stream-Stream ----

    private static void runMulticastStreamStream(TestSlimrpc.TestGroupClient client) throws Exception {
        System.out.println("\n=== Multicast Stream-Stream ===");

        MulticastClientBidiStream<ExampleRequest, ExampleResponse> stream =
                client.ExampleStreamStream(Duration.ofSeconds(5), null);

        // Send requests in a background thread
        Thread sendThread = new Thread(() -> {
            try {
                for (int i = 0; i < 3; i++) {
                    ExampleRequest req = ExampleRequest.newBuilder()
                            .setExampleString("item " + i)
                            .setExampleInteger(i)
                            .build();
                    stream.send(req);
                }
                stream.closeSend();
            } catch (Exception e) {
                System.out.println("Send error: " + e.getMessage());
            }
        });
        sendThread.start();

        // Receive multicast responses
        while (true) {
            MulticastStreamMessage msg = stream.recv();
            if (msg instanceof MulticastStreamMessage.End) break;
            if (msg instanceof MulticastStreamMessage.Error err) {
                System.out.println("  Error: " + err.error());
                break;
            }
            if (msg instanceof MulticastStreamMessage.Data data) {
                RpcMulticastItem item = data.item();
                ExampleResponse resp = ExampleResponse.parseFrom(item.message());
                System.out.println("  [" + item.context().source().toString() + "] " + resp);
            }
        }

        sendThread.join();
    }
}
