// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package integration

// Test message constants for expected log outputs in integration tests
const (
	// SLIM Node messages
	MsgCreateChannelRequest     = "received a channel create request"
	MsgParticipantAddRequest    = "received a participant add request"
	MsgParticipantDeleteRequest = "received a participant delete request"
	MsgChannelDeleteRequest     = "received a channel delete request"

	// Control Plane messages
	MsgChannelCreatedSuccessfully        = "Channel created successfully"
	MsgChannelSavedSuccessfully          = "Channel saved successfully"
	MsgAckParticipantAddedSuccessfully   = "Ack message received, participant added successfully."
	MsgChannelUpdatedParticipantAdded    = "Channel updated, participant added successfully."
	MsgAckParticipantDeletedSuccessfully = "Ack message received, participant deleted successfully."
	MsgChannelUpdatedParticipantDeleted  = "Channel updated, participant deleted successfully"
	MsgAckChannelDeletedSuccessfully     = "Ack message received, channel deleted successfully."
	MsgChannelDeletedSuccessfully        = "Channel deleted successfully"

	// Client messages
	MsgSessionHandlerTaskStarted = "Session handler task started"
	MsgSessionClosed             = "session closed"

	// Test communication messages
	MsgTestClientCMessage = "hey there, I am c!"
)
