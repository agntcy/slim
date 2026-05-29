// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package com.example_service.example.client

import com.example_service.ExampleResponse
import com.example_service.TestSlimrpc
import com.example_service.exampleRequest
import io.agntcy.slim.bindings.*
import io.agntcy.slim.bindings.slimrpc.ClientResponseStream
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import java.time.Duration

private const val NAME_ORG = "agntcy"
private const val NAME_NS = "grpc"
private const val DEFAULT_SERVER_ENDPOINT = "http://localhost:46357"
private const val DEFAULT_SHARED_SECRET = "my_shared_secret_for_testing_purposes_only"

fun getServerEndpoint(): String {
    return System.getenv("SLIM_ADDR")?.takeIf { it.isNotEmpty() } ?: DEFAULT_SERVER_ENDPOINT
}

fun main(args: Array<String>) = runBlocking {
    var serverAddr = getServerEndpoint()
    var i = 0
    while (i < args.size) {
        when (args[i]) {
            "--server" -> if (i + 1 < args.size) serverAddr = args[++i]
        }
        i++
    }

    initializeWithDefaults()
    val service = getGlobalService()

    val localName = Name(NAME_ORG, NAME_NS, "client")
    val remoteName = Name(NAME_ORG, NAME_NS, "server")

    val app = service.createAppWithSecret(localName, DEFAULT_SHARED_SECRET)
    val clientConfig = newInsecureClientConfig(serverAddr)
    val connId = service.connectAsync(clientConfig)
    app.subscribeAsync(app.name(), connId)

    val channel = Channel.newWithConnection(app, remoteName, connId)
    val client = TestSlimrpc.TestClientImpl(channel)

    val request = exampleRequest {
        exampleString = "Alice"
        exampleInteger = 42
    }

    println("SLIM_RPC_CLIENT_STARTED")

    println("=== Unary-Unary ===")
    val response = client.ExampleUnaryUnary(request, Duration.ofSeconds(10), null)
    println("Response: $response")

    println("=== Unary-Stream ===")
    val streamReader = client.ExampleUnaryStream(request, Duration.ofSeconds(10), null)
    val responseStream = ClientResponseStream.create(streamReader) { bytes ->
        ExampleResponse.parseFrom(bytes)
    }
    while (true) {
        val streamResp = responseStream.recv() ?: run {
            println("Stream ended")
            break
        }
        println("Stream Response: $streamResp")
    }

    println("=== Stream-Unary ===")
    val streamUnary = client.ExampleStreamUnary(Duration.ofSeconds(10), null)
    val names = arrayOf("Alice", "Bob", "Charlie", "Diana", "Eve")
    for ((idx, name) in names.withIndex()) {
        val req = exampleRequest {
            exampleString = name
            exampleInteger = (idx + 1).toLong()
        }
        streamUnary.send(req)
    }
    val streamUnaryResp = streamUnary.finalizeStream()
    println("Stream Unary Response: $streamUnaryResp")

    println("=== Stream-Stream ===")
    val streamStream = client.ExampleStreamStream(Duration.ofSeconds(10), null)

    val sendJob = launch {
        for ((idx, name) in names.withIndex()) {
            val req = exampleRequest {
                exampleString = name
                exampleInteger = ((idx + 1) * 10).toLong()
            }
            streamStream.send(req)
            println("Sent request $name")
        }
        streamStream.closeSend()
        println("Send stream closed")
    }

    while (true) {
        val msg = streamStream.recv()
        when (msg) {
            is StreamMessage.End -> {
                println("Stream Stream ended")
                break
            }
            is StreamMessage.Error -> throw msg.v1
            is StreamMessage.Data -> {
                val streamResp = ExampleResponse.parseFrom(msg.v1)
                println("Stream Stream Response: $streamResp")
            }
        }
    }

    sendJob.join()
    println("SLIM_RPC_CLIENT_DONE")
}
