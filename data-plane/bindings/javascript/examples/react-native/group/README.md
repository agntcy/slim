# SLIM React Native - Group Messaging Example

Demonstrates multi-party group messaging with a moderator and participants.

## Overview

This example shows:
- Creating a group session
- Moderator inviting participants
- Broadcasting messages to all participants
- Receiving messages from multiple senders

## Running the Example

Requires a running SLIM server.

### Terminal 1 - SLIM Server

```bash
task example:server
```

### Terminal 2 - Moderator

```bash
npx react-native run-ios -- --simulator="iPhone 15 Pro"
# Select "Moderator" mode
```

### Terminal 3 - Alice (Participant)

```bash
npx react-native run-ios -- --simulator="iPhone 15 Pro Max"
# Select "Participant (Alice)" mode
```

### Terminal 4 - Bob (Participant)

```bash
npx react-native run-ios -- --simulator="iPhone 14"
# Select "Participant (Bob)" mode
```

## Architecture

```
                   ┌──────────────┐
                   │ SLIM  Server │
                   └──────────────┘
                          │
                  ┌───────┴───────┐
                  │               │
            ┌─────▼────┐    ┌─────▼────┐
            │Moderator │────│  Alice   │
            └─────┬────┘    └──────────┘
                  │
            ┌─────▼────┐
            │   Bob    │
            └──────────┘
```

## Features

- **Moderator Controls**: Create session, invite participants, broadcast messages
- **Participant Actions**: Wait for invitation, receive messages, send replies
- **Multi-Party**: Messages visible to all group members
- **Dynamic Membership**: Add/remove participants

## Code Highlights

```typescript
// Moderator creates group session
const config: SessionConfig = {
  sessionType: SessionType.Group,
  enableMls: false,
  maxRetries: undefined,
  interval: undefined,
  metadata: {}
}

const session = await app.createSessionAndWait(config, groupName)

// Invite participants
await session.inviteAndWait(new Name(['org', 'alice', 'app'], undefined))
await session.inviteAndWait(new Name(['org', 'bob', 'app'], undefined))

// Broadcast message
await session.publishAndWait(
  new TextEncoder().encode('Hello everyone!'),
  undefined,
  undefined
)
```

## Expected Output

**Moderator**:
```
👑 Creating group session...
✅ Session created
📤 Inviting Alice...
📤 Inviting Bob...
✅ Participants invited
📢 Broadcasting: "Meeting started!"
```

**Alice**:
```
📥 Received invitation
✅ Joined group
📥 Moderator: "Meeting started!"
📤 Replying: "Hello from Alice!"
```

**Bob**:
```
📥 Received invitation
✅ Joined group
📥 Moderator: "Meeting started!"
📥 Alice: "Hello from Alice!"
📤 Replying: "Hello from Bob!"
```

## Related Examples

- [Simple Example](../simple/) - Basic usage
- [Point-to-Point Example](../point-to-point/) - 1:1 messaging
