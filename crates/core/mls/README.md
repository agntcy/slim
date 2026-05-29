# SLIM MLS Module

[![License](https://img.shields.io/badge/license-Apache%202.0-blue.svg)](../../../LICENSE)

The `agntcy-slim-mls` module provides Messaging Layer Security (MLS) implementation
for the Agntcy SLIM data plane, enabling secure group communication with end-to-end
encryption and forward secrecy.

## Overview

The MLS module implements the IETF Messaging Layer Security protocol to provide:

- **End-to-end encryption** for group communications
- **Forward secrecy** through continuous key rotation
- **Post-compromise security** via tree-based key derivation
- **Dynamic group membership** with secure member addition/removal
- **Identity verification** integrated with SLIM's authentication system

## Features

### Core Capabilities

- **Group Management**: Create, join, and manage secure communication groups
- **Member Operations**: Add and remove members with proper key distribution
- **Message Encryption**: Secure application messages with group keys
- **Key Rotation**: Automatic and manual key updates for forward secrecy
- **Persistent Storage**: Save and restore group state and identities

### Security Properties

- **Confidentiality**: Messages are encrypted and only readable by group members
- **Authentication**: Message sender identity is cryptographically verified
- **Integrity**: Message tampering is detected and prevented
- **Forward Secrecy**: Past messages remain secure even if current keys are compromised
- **Post-Compromise Security**: Future messages are secure after key compromise recovery
