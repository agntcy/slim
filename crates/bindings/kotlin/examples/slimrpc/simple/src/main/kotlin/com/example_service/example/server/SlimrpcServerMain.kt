// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package com.example_service.example.server

import com.example_service.ExampleRequest
import com.example_service.ExampleResponse
import com.example_service.TestSlimrpc
import com.example_service.exampleResponse
import io.agntcy.slim.bindings.*
import io.agntcy.slim.bindings.slimrpc.ServerBidiStream
import io.agntcy.slim.bindings.slimrpc.ServerRequestStream
import io.agntcy.slim.bindings.slimrpc.ServerResponseStream
import kotlinx.coroutines.runBlocking

private const val NAME_ORG = "agntcy"
private const val NAME_NS = "grpc"
private const val DEFAULT_SERVER_ENDPOINT = "http://localhost:46357"
private const val DEFAULT_SHARED_SECRET = "my_shared_secret_for_testing_purposes_only"

fun getServerEndpoint(): String {
    return System.getenv("SLIM_ADDR")?.takeIf { it.isNotEmpty() } ?: DEFAULT_SERVER_ENDPOINT
}

fun main(args: Array<String>) = runBlocking {
    var instance = "server"
    var serverAddr = getServerEndpoint()
    var i = 0
    while (i < args.size) {
        when (args[i]) {
            "--instance" -> if (i + 1 < args.size) instance = args[++i]
            "--server" -> if (i + 1 < args.size) serverAddr = args[++i]
        }
        i++
    }

    val runtime = newRuntimeConfig()
    val tracing = newTracingConfigWith("info", true, false, emptyList())
    val serviceConfig = newServiceConfig()
    initializeWithConfigs(runtime, tracing, listOf(serviceConfig))

    val service = getGlobalService()

    val localName = Name(NAME_ORG, NAME_NS, instance)
    val app = service.createAppWithSecret(localName, DEFAULT_SHARED_SECRET)

    val clientConfig = newInsecureClientConfig(serverAddr)
    val connId = service.connectAsync(clientConfig)
    app.subscribeAsync(app.name(), connId)

    val rpcServer = Server.newWithConnection(app, localName, connId)
    TestSlimrpc.registerTestServer(rpcServer, object : TestSlimrpc.UnimplementedTestServer() {

        override suspend fun ExampleUnaryUnary(request: ExampleRequest, context: Context): ExampleResponse {
            return exampleResponse {
                exampleString = "Hello, ${request.exampleString}!"
                exampleInteger = request.exampleInteger * 2
            }
        }

        override suspend fun ExampleUnaryStream(request: ExampleRequest, context: Context, sink: ResponseSink) {
            val stream = ServerRequestStream.create<ExampleResponse>(sink) { it.toByteArray() }
            for (i in 1..3) {
                val response = exampleResponse {
                    exampleString = "${request.exampleString} reply #$i"
                    exampleInteger = request.exampleInteger * i
                }
                stream.send(response)
            }
        }

        override suspend fun ExampleStreamUnary(stream: RequestStream, context: Context): ExampleResponse {
            val requestStream = ServerResponseStream.create<ExampleRequest>(stream) { bytes ->
                ExampleRequest.parseFrom(bytes)
            }
            val names = mutableListOf<String>()
            var sum = 0L
            while (true) {
                val req = requestStream.recv() ?: break
                names.add(req.exampleString)
                sum += req.exampleInteger
            }
            return exampleResponse {
                exampleString = "Received from: ${names.joinToString(", ")}"
                exampleInteger = sum
            }
        }

        override suspend fun ExampleStreamStream(stream: RequestStream, context: Context, sink: ResponseSink) {
            val bidiStream = ServerBidiStream.create<ExampleRequest, ExampleResponse>(
                stream, sink,
                { bytes -> ExampleRequest.parseFrom(bytes) },
                { it.toByteArray() }
            )
            while (true) {
                val req = bidiStream.recv() ?: break
                val response = exampleResponse {
                    exampleString = "Hello, ${req.exampleString}!"
                    exampleInteger = req.exampleInteger * 2
                }
                bidiStream.send(response)
            }
        }
    })

    println("SLIM_RPC_SERVER_READY")
    rpcServer.serve()
}
