// SPDX-FileCopyrightText: Copyright (c) 2025 Cisco and/or its affiliates.
// SPDX-License-Identifier: Apache-2.0

pub mod encoder;
pub mod messages;

pub use gateway_pubsub_proto::proto::pubsub::v1::message::MessageType;
pub use gateway_pubsub_proto::proto::pubsub::v1::message::MessageType::Publish as ProtoPublishType;
pub use gateway_pubsub_proto::proto::pubsub::v1::message::MessageType::Subscribe as ProtoSubscribeType;
pub use gateway_pubsub_proto::proto::pubsub::v1::message::MessageType::Unsubscribe as ProtoUnsubscribeType;
pub use gateway_pubsub_proto::proto::pubsub::v1::AgentClass as ProtoAgentClass;
pub use gateway_pubsub_proto::proto::pubsub::v1::AgentGroup as ProtoAgentGroup;
pub use gateway_pubsub_proto::proto::pubsub::v1::AgentId as ProtoAgentId;

pub use gateway_pubsub_proto::proto::pubsub::v1::Content;
pub use gateway_pubsub_proto::proto::pubsub::v1::Message as ProtoMessage;
pub use gateway_pubsub_proto::proto::pubsub::v1::Publish as ProtoPublish;
pub use gateway_pubsub_proto::proto::pubsub::v1::Subscribe as ProtoSubscribe;
pub use gateway_pubsub_proto::proto::pubsub::v1::Unsubscribe as ProtoUnsubscribe;
