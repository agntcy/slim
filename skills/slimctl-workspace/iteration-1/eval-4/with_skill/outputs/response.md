# Creating a SLIM Channel, Adding Participants, and Listing Them

Here is a walkthrough of the `slimctl` commands to create a channel with `alice` as a moderator, add `bob` and `carol` as participants, and then list all participants.

## Step 1: Create the Channel with Alice as Moderator

```bash
slimctl controller channel create moderators=alice
```

This command talks to the SLIM Control Plane and creates a new MLS-backed channel with `alice` designated as a moderator. The command will return a **channel ID** (e.g., something like `ch-abc123`) that you will need for the subsequent steps.

## Step 2: Add Participants Bob and Carol

Using the channel ID returned from step 1, add `bob` and `carol` as participants:

```bash
slimctl controller participant add bob   -c <channel-id>
slimctl controller participant add carol -c <channel-id>
```

Replace `<channel-id>` with the actual ID returned when you created the channel.

## Step 3: List Participants

To verify that both participants were added successfully, list all participants on the channel:

```bash
slimctl controller participant list -c <channel-id>
```

This will display all participants currently in the channel, which should include `bob` and `carol` (and potentially `alice` as the moderator, depending on how the control plane reports membership).

## Notes

- All `controller` commands talk to the SLIM Control Plane (not a single node). Make sure your slimctl is pointed at the correct control plane server. The default is `127.0.0.1:46357`.
- If your control plane is running elsewhere, use the `--server` flag or configure it persistently:
  ```bash
  slimctl config set server <control-plane-host:port>
  ```
- The `controller` subcommand can also be shortened to `c` or `ctrl`:
  ```bash
  slimctl ctrl channel create moderators=alice
  slimctl ctrl participant add bob -c <channel-id>
  ```
