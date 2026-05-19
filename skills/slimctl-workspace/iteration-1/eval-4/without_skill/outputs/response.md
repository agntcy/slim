# Creating a SLIM Channel and Managing Participants with slimctl

Here is a walkthrough of the `slimctl` commands to create a channel with `alice` as a moderator, add `bob` and `carol` as participants, and then list the participants.

## Prerequisites

Before running these commands, ensure that:

1. `slimctl` is built and available in your PATH.
2. A SLIM control plane is running and accessible.
3. Your `slimctl` configuration is set to point to the correct control plane server (you can verify with `slimctl config list` or set it with `slimctl config set server <host:port>`).

## Step 1: Create the Channel with Alice as Moderator

```bash
slimctl controller channel create moderators=alice
```

This command creates a new channel (MLS group) with `alice` designated as a moderator. The argument uses the format `moderators=<comma-separated-list>`. If the command succeeds, it will output something like:

```
Channel created successfully: <channel-name>
```

Take note of the returned `<channel-name>` -- you will need it in subsequent commands.

If you wanted multiple moderators at creation time, you could do:

```bash
slimctl controller channel create moderators=alice,dave
```

## Step 2: Add Participants Bob and Carol

Once you have the channel name from step 1, add `bob` and `carol` as participants:

```bash
slimctl controller participant add bob -c <channel-name>
slimctl controller participant add carol -c <channel-name>
```

The `-c` flag (short for `--channel-id`) specifies which channel to add the participant to. Each successful addition will produce output like:

```
Adding participant to channel <channel-name>: bob
Participant added successfully to channel <channel-name>: bob
```

## Step 3: List Participants

To verify that both participants were added correctly, list all participants in the channel:

```bash
slimctl controller participant list -c <channel-name>
```

This will output the list of participants registered in the channel:

```
Listing participants for channel ID: <channel-name>
Following participants found for channel <channel-name>: ["bob", "carol"]
```

## Complete Example

Putting it all together (assuming the channel name returned is `my-channel`):

```bash
# Create channel with alice as moderator
slimctl controller channel create moderators=alice
# Output: Channel created successfully: my-channel

# Add bob as a participant
slimctl controller participant add bob -c my-channel

# Add carol as a participant
slimctl controller participant add carol -c my-channel

# List all participants
slimctl controller participant list -c my-channel
```

## Additional Useful Commands

- **List all channels:** `slimctl controller channel list`
- **Delete a participant:** `slimctl controller participant delete bob -c my-channel`
- **Delete a channel:** `slimctl controller channel delete my-channel`

## Notes

- The `controller` subcommand can also be invoked using its aliases: `c` or `ctrl`. For example: `slimctl c channel create moderators=alice`.
- The `participant list` subcommand also supports the `ls` alias: `slimctl controller participant ls -c my-channel`.
- If TLS is not configured, you may need to pass `--tls.insecure` to disable TLS for the gRPC connection to the control plane.
