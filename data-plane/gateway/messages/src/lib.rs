// SPDX-FileCopyrightText: Copyright (c) 2025 Cisco and/or its affiliates.
// SPDX-License-Identifier: Apache-2.0

pub mod encoder;
pub mod messages;

pub use agp_gw_pubsub_proto::proto::pubsub::v1::message::MessageType;
pub use agp_gw_pubsub_proto::proto::pubsub::v1::message::MessageType::Publish as ProtoPublishType;
pub use agp_gw_pubsub_proto::proto::pubsub::v1::message::MessageType::Subscribe as ProtoSubscribeType;
pub use agp_gw_pubsub_proto::proto::pubsub::v1::message::MessageType::Unsubscribe as ProtoUnsubscribeType;
pub use agp_gw_pubsub_proto::proto::pubsub::v1::AgentClass as ProtoAgentClass;
pub use agp_gw_pubsub_proto::proto::pubsub::v1::AgentGroup as ProtoAgentGroup;
pub use agp_gw_pubsub_proto::proto::pubsub::v1::AgentId as ProtoAgentId;

pub use agp_gw_pubsub_proto::proto::pubsub::v1::Content;
pub use agp_gw_pubsub_proto::proto::pubsub::v1::Message as ProtoMessage;
pub use agp_gw_pubsub_proto::proto::pubsub::v1::Publish as ProtoPublish;
pub use agp_gw_pubsub_proto::proto::pubsub::v1::Subscribe as ProtoSubscribe;
pub use agp_gw_pubsub_proto::proto::pubsub::v1::Unsubscribe as ProtoUnsubscribe;
