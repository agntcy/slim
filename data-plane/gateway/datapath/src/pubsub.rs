// SPDX-FileCopyrightText: Copyright (c) 2025 Cisco and/or its affiliates.
// SPDX-License-Identifier: Apache-2.0

//! gRPC bindings for pubsub service.
pub mod proto;

pub use proto::pubsub::v1::message::MessageType;
pub use proto::pubsub::v1::message::MessageType::Publish as ProtoPublishType;
pub use proto::pubsub::v1::message::MessageType::Subscribe as ProtoSubscribeType;
pub use proto::pubsub::v1::message::MessageType::Unsubscribe as ProtoUnsubscribeType;
pub use proto::pubsub::v1::Agent as ProtoAgent;
pub use proto::pubsub::v1::AgpHeader;
pub use proto::pubsub::v1::Content;
pub use proto::pubsub::v1::Message as ProtoMessage;
pub use proto::pubsub::v1::Publish as ProtoPublish;
pub use proto::pubsub::v1::ServiceHeader;
pub use proto::pubsub::v1::Subscribe as ProtoSubscribe;
pub use proto::pubsub::v1::Unsubscribe as ProtoUnsubscribe;
