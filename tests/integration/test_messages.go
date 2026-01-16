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
	MsgChannelCreated                   = "Channel creation result for %s: success=%t"
	MsgChannelSavedSuccessfully         = "Channel saved successfully"
	MsgParticipantAdded                 = "AddParticipant result success=%t"
	MsgChannelUpdatedParticipantAdded   = "Channel updated, participant added successfully."
	MsgParticipantDeleted               = "DeleteParticipant result success=%t"
	MsgChannelUpdatedParticipantDeleted = "Channel updated, participant deleted successfully"
	MsgAckChannelDeleted                = "Channel deletion result for %s: success=%t"
	MsgChannelDeletedSuccessfully       = "Channel deleted successfully"

	// Client messages
	MsgSessionHandlerTaskStarted = "Session handler task started"
	MsgSessionClosed             = "session closed"

	// Test communication messages
	MsgTestClientCMessage = "hey there, I am c!"
)
