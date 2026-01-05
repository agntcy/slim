# Ping Routing Bug Analysis

## Issue Summary

Participants in a group session are being incorrectly disconnected because Ping messages are routed to the wrong destination.

## Symptoms

- `org/client2/app` was removed from a group session after ~30 seconds
- client2 was actively sending messages but still got disconnected
- Only `org/client1/app` was responding to Pings

## Root Cause

The `ControllerSender` in the session layer sends Ping messages to the **original session destination** (`org/client1/app`) instead of the **group name** (`org/test/chatroom11`).

### Evidence from Logs

| Time | Ping Direction | Response |
|------|---------------|----------|
| 19:48:47.829 | sessionmgr → `org/client1/app` | client1 replied |
| 19:48:52.833 | sessionmgr → `org/client1/app` | client1 replied |
| 19:48:57.843 | sessionmgr → `org/client1/app` | client1 replied |
| 19:49:17.855 | sessionmgr → `org/client1/app` | client1 replied |

**client2 never received any Pings** because they were sent to client1's address, not the group.

### Timeline

1. **19:48:41** - client2 joined the group, subscribed to `org/test/chatroom11`
2. **19:48:47-19:49:08** - client2 actively sending messages and MsgAcks
3. **19:48:47-19:49:17** - Multiple Pings sent, ALL to `org/client1/app` (wrong!)
4. **19:49:17** - client2 removed due to "missed" Pings it never received

## Code Analysis

### Problem Location

File: `data-plane/core/session/src/controller_sender.rs`

The `group_name` field is set incorrectly in `on_group_welcome()`:

```rust
// Line 214-224
if self.group_name.is_none() {
    debug!("update group name on welcome message");
    self.group_name = Some(message.get_dst());  // BUG: Gets P2P destination, not group
}
```

When Pings are sent (line ~481):
```rust
let mut builder = Message::builder()
    .destination(group_name.clone())  // Uses wrong group_name
```

### Why This Happens

1. For the **moderator** (sessionmgr), the session is created with `destination = org/client1/app`
2. When GroupWelcome is processed, `group_name` gets set to the message destination
3. But for group sessions, the actual group name comes from the `channel` field in JoinRequest
4. Pings are then sent to `org/client1/app` instead of `org/test/chatroom11`

## Expected Behavior

- Pings should be sent to the **group/channel name** for multicast sessions
- All participants subscribed to the group should receive Pings
- All participants should be able to respond

## Fix Requirements

1. For multicast sessions, use the actual group/channel name for Ping routing
2. The group name should be extracted from:
   - `JoinRequest.channel` when inviting participants
   - `GroupWelcome.participants` context or session configuration
3. Ensure `group_name` is set correctly before any Ping is sent

## Impact

- Any group session with 2+ participants will incorrectly disconnect participants
- Only the original destination participant receives Pings
- All other participants appear "unresponsive" and get removed

## Related Files

- `data-plane/core/session/src/controller_sender.rs` - Ping sending logic
- `data-plane/core/session/src/session_moderator.rs` - Group management
- `data-plane/core/session/src/session_participant.rs` - Ping response handling

## Fix Applied

### Root Cause (Deeper Analysis)

The issue had two components:

1. **Rust session layer**: The `group_name` field in `ControllerSender` was initially `None` and only set from the GroupWelcome message destination (which is a participant address, not a group name).

2. **Go example usage**: The sessionmgr Go example was using the first participant's address as the session `destination`, when for group sessions it should be using a unique group/channel name.

### Changes Made

#### 1. Rust: Set `group_name` at session creation (`session_controller.rs`)

For multicast sessions, the `group_name` is now set from `settings.destination` at construction time:

```rust
// For multicast sessions, set the group_name to the session destination (the group/channel)
// For point-to-point sessions, leave it as None (will be learned from welcome message)
let group_name = if settings.config.session_type == ProtoSessionType::Multicast {
    Some(settings.destination.clone())
} else {
    None
};
```

#### 2. Rust: Add `group_name` parameter to `ControllerSender::new()`

The `ControllerSender` now accepts `group_name` as a constructor parameter instead of always starting with `None`.

#### 3. Go Example: Clarified UI for group names (`home.html`)

The UI now shows different placeholder text based on session type:
- **P2P**: "Peer address (org/namespace/app)..."
- **Group**: "Group name (org/namespace/chatroom)..."

This helps users understand that for group sessions, they should enter a **group/channel name** (like `org/myteam/chatroom`), not a participant address.

After creating a group session, participants should be invited using the "Invite Participant" button.

### Verification

With these fixes:
- Group sessions use proper group names like `org/myteam/chatroom`
- Pings are sent to the group name, which all participants are subscribed to
- All participants receive and respond to Pings
- No participants are incorrectly disconnected

### Usage

1. **Create a group session**: Enter a group name (e.g., `org/myteam/chatroom`) as the destination
2. **Invite participants**: Use the "Invite Participant" button to add `org/client1/app`, `org/client2/app`, etc.
3. All participants will subscribe to the group name and receive pings correctly

### Verified

Tested on 2025-12-27:
- Created group session with destination `org/default/chat`
- Invited client1 and client2 (both accepted invitations)
- Both clients remained connected for over 2 minutes (previously disconnected after ~30-40 seconds)
- Debug logs confirmed pings were sent to `org/default/chat/ffffffffffffffff` (group name) instead of individual participant addresses
- No "participant disconnected" errors occurred
