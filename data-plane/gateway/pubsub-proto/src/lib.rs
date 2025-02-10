// SPDX-FileCopyrightText: Copyright (c) 2025 Cisco and/or its affiliates.
// SPDX-License-Identifier: Apache-2.0

//! gRPC bindings for pubsub service.

pub mod messages;
pub mod proto;

pub use messages::encoder::Agent;
pub use messages::encoder::AgentClass;

pub use proto::pubsub::v1::message::MessageType;
pub use proto::pubsub::v1::message::MessageType::Publish as ProtoPublishType;
pub use proto::pubsub::v1::message::MessageType::Subscribe as ProtoSubscribeType;
pub use proto::pubsub::v1::message::MessageType::Unsubscribe as ProtoUnsubscribeType;
pub use proto::pubsub::v1::AgentClass as ProtoAgentClass;
pub use proto::pubsub::v1::AgentGroup as ProtoAgentGroup;
pub use proto::pubsub::v1::AgentId as ProtoAgentId;
pub use proto::pubsub::v1::Content;
pub use proto::pubsub::v1::Message as ProtoMessage;
pub use proto::pubsub::v1::Publish as ProtoPublish;
pub use proto::pubsub::v1::Subscribe as ProtoSubscribe;
pub use proto::pubsub::v1::Unsubscribe as ProtoUnsubscribe;
