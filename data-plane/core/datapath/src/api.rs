// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! gRPC bindings for data plane service.
pub(crate) mod proto;

pub use proto::dataplane::v1::ApplicationPayload;
pub use proto::dataplane::v1::CommandPayload;
pub use proto::dataplane::v1::Content;
pub use proto::dataplane::v1::Message as ProtoMessage;
pub use proto::dataplane::v1::Name as ProtoName;
pub use proto::dataplane::v1::Publish as ProtoPublish;
pub use proto::dataplane::v1::SessionHeader;
pub use proto::dataplane::v1::SessionMessageType as ProtoSessionMessageType;
pub use proto::dataplane::v1::SessionType as ProtoSessionType;
pub use proto::dataplane::v1::SlimHeader;
pub use proto::dataplane::v1::Subscribe as ProtoSubscribe;
pub use proto::dataplane::v1::Unsubscribe as ProtoUnsubscribe;
pub use proto::dataplane::v1::data_plane_service_client::DataPlaneServiceClient;
pub use proto::dataplane::v1::data_plane_service_server::DataPlaneServiceServer;
pub use proto::dataplane::v1::message::MessageType;
pub use proto::dataplane::v1::message::MessageType::Publish as ProtoPublishType;
pub use proto::dataplane::v1::message::MessageType::Subscribe as ProtoSubscribeType;
pub use proto::dataplane::v1::message::MessageType::Unsubscribe as ProtoUnsubscribeType;
