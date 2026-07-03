# Tutorials

These tutorials walk you through using the SLIM SDK step by step — from establishing a connection to calling remote services with SLIMRPC. Each tutorial builds on the previous one, so work through them in order.

Tutorials include code examples for Python, Go, Java, Kotlin, Node.js, .NET, and React Native where supported.

## Prerequisites

All tutorials assume you have the SLIM SDK installed. See [Installation](../install.md) for setup instructions for your language.

## SDK Tutorials

| Tutorial | What you'll learn |
|---|---|
| [Connecting to SLIM](./tutorial-connect.md) | Configure a SLIM client and connect to a node |
| [Creating an App](./tutorial-app.md) | Register a named identity on the SLIM network |
| [Creating a Session](./tutorial-session.md) | Open P2P and group sessions, send messages, and configure reliability and encryption |
| [Receiving a Session](./tutorial-receive.md) | Listen for incoming sessions, receive messages, and reply |

## SLIMRPC Tutorials

| Tutorial | What you'll learn |
|---|---|
| [Serving a SLIMRPC Server](./slimrpc/tutorial-serve.md) | Define a proto, generate stubs, implement handlers, and serve |
| [Using a SLIMRPC Server](./slimrpc/tutorial-client.md) | Create a channel and call all four streaming patterns |
| [Multicast SLIMRPC](./slimrpc/tutorial-multicast.md) | Fan out a single RPC call to multiple servers and collect per-server responses |
