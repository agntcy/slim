# SLIM Remote Procedure Call (SRPC) Compiler

`srpc-compiler` is a code generation tool designed to create SRPC (Slim RPC)
stubs from Protocol Buffers (`.proto`) definition files. Similar in concept
to `protoc`, the Protocol Buffer compiler, `srpc-compiler` generates the 
necessary code for implementing services and clients that can run on top of
the SLIM network, exploiting the SLIM capabilities.

## What is SRPC?

SRPC (Slim RPC) is a Remote Procedure Call (RPC) that runs on top of SLIM.
SRPC allows developers to define service interfaces and message structures
using the familiar Protobuf syntax, and then generate code that facilitates
seamless inter-service communication using SLIM. 

## Features

*   **Protobuf Compatibility**: Consumes standard `.proto` files as input,
allowing the user to define services and messages using the widely adopted
Protocol Buffers schema definition language.
*   **SRPC Stub Generation**: Generates client and server stubs for the SRPC
framework, enabling efficient RPC communication within applications running
on top of SLIM.
*   **Easy to use**: Starting from `.proto` file, it provides an easy way
to run an application service using SLIM.

## How it Works

`srpc-compiler` processes your `.proto` files, much like `protoc`. Instead of
producing gRPC-specific code, it generates code that implements the SRPC
communication patterns and data serialization/deserialization mechanisms.
This allows your services to communicate over SLIM using the defined Protobuf
messages and service methods.

## Usage

The `srpc-compiler` binary compiler can be found here: TODO
