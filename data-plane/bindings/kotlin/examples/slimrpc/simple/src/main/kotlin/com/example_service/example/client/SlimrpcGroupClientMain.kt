// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package com.example_service.example.client

import com.example_service.ExampleResponse
import com.example_service.TestSlimrpc
import com.example_service.exampleRequest
import io.agntcy.slim.bindings.*
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import java.time.Duration

private const val NAME_ORG = "agntcy"
private const val NAME_NS = "grpc"
private const val DEFAULT_SERVER_ENDPOINT = "http://localhost:46357"
private const val DEFAULT_SHARED_SECRET = "my_shared_secret_for_testing_purposes_only"

fun main(args: Array<String>) = runBlocking {
    var serverAddr = System.getenv("SLIM_ADDR")?.takeIf { it.isNotEmpty() } ?: DEFAULT_SERVER_ENDPOINT
    var serversStr = "server1,server2"
    var i = 0
    while (i < args.size) {
        when (args[i]) {
            "--server" -> if (i + 1 < args.size) serverAddr = args[++i]
            "--servers" -> if (i + 1 < args.size) serversStr = args[++i]
        }
        i++
    }

    val serverNames = serversStr.split(",").map { Name(NAME_ORG, NAME_NS, it.trim()) }

    initializeWithDefaults()
    val service = getGlobalService()

    val localName = Name(NAME_ORG, NAME_NS, "client")
    val app = service.createAppWithSecret(localName, DEFAULT_SHARED_SECRET)

    val clientConfig = newInsecureClientConfig(serverAddr)
    val connId = service.connectAsync(clientConfig)
    app.subscribeAsync(app.name(), connId)

    val channel = Channel.newGroupWithConnection(app, serverNames, connId)
    val client = TestSlimrpc.TestGroupClientImpl(channel)

    try {
        println("SLIM_RPC_GROUP_CLIENT_STARTED")
        runMulticastUnary(client)
        runMulticastUnaryStream(client)
        runMulticastStreamUnary(client)
        runMulticastStreamStream(client)
        println("SLIM_RPC_GROUP_CLIENT_DONE")
    } catch (e: Exception) {
        System.err.println("Group client error: ${e.message}")
        throw e
    } finally {
        channel.closeAsync(null)
    }
}

private suspend fun runMulticastUnary(client: TestSlimrpc.TestGroupClient) {
    println("=== Multicast Unary-Unary ===")
    val request = exampleRequest {
        exampleString = "hello"
        exampleInteger = 1
    }

    val stream = client.ExampleUnaryUnary(request, Duration.ofSeconds(5), null)
    while (true) {
        val msg = stream.next()
        when (msg) {
            is MulticastStreamMessage.End -> break
            is MulticastStreamMessage.Error -> {
                println("  Error: ${msg.error}")
                break
            }
            is MulticastStreamMessage.Data -> {
                val item = msg.item
                val resp = ExampleResponse.parseFrom(item.message)
                println("  [${item.context.source}] $resp")
            }
        }
    }
}

private suspend fun runMulticastUnaryStream(client: TestSlimrpc.TestGroupClient) {
    println("\n=== Multicast Unary-Stream ===")
    val request = exampleRequest {
        exampleString = "hello"
        exampleInteger = 1
    }

    val stream = client.ExampleUnaryStream(request, Duration.ofSeconds(5), null)
    while (true) {
        val msg = stream.next()
        when (msg) {
            is MulticastStreamMessage.End -> break
            is MulticastStreamMessage.Error -> {
                println("  Error: ${msg.error}")
                break
            }
            is MulticastStreamMessage.Data -> {
                val item = msg.item
                val resp = ExampleResponse.parseFrom(item.message)
                println("  [${item.context.source}] $resp")
            }
        }
    }
}

private suspend fun runMulticastStreamUnary(client: TestSlimrpc.TestGroupClient) {
    println("\n=== Multicast Stream-Unary ===")

    val stream = client.ExampleStreamUnary(Duration.ofSeconds(5), null)
    for (i in 0 until 3) {
        val req = exampleRequest {
            exampleString = "item $i"
            exampleInteger = i.toLong()
        }
        stream.send(req)
    }
    stream.closeSend()

    while (true) {
        val msg = stream.recv()
        when (msg) {
            is MulticastStreamMessage.End -> break
            is MulticastStreamMessage.Error -> {
                println("  Error: ${msg.error}")
                break
            }
            is MulticastStreamMessage.Data -> {
                val item = msg.item
                val resp = ExampleResponse.parseFrom(item.message)
                println("  [${item.context.source}] $resp")
            }
        }
    }
}

private suspend fun runMulticastStreamStream(client: TestSlimrpc.TestGroupClient) {
    println("\n=== Multicast Stream-Stream ===")

    val stream = client.ExampleStreamStream(Duration.ofSeconds(5), null)

    coroutineScope {
        launch {
            for (i in 0 until 3) {
                val req = exampleRequest {
                    exampleString = "item $i"
                    exampleInteger = i.toLong()
                }
                stream.send(req)
            }
            stream.closeSend()
        }
    }

    while (true) {
        val msg = stream.recv()
        when (msg) {
            is MulticastStreamMessage.End -> break
            is MulticastStreamMessage.Error -> {
                println("  Error: ${msg.error}")
                break
            }
            is MulticastStreamMessage.Data -> {
                val item = msg.item
                val resp = ExampleResponse.parseFrom(item.message)
                println("  [${item.context.source}] $resp")
            }
        }
    }
}
