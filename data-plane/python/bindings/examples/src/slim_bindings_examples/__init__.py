# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0
#
# This module acts as the entry-point for the slim_bindings example CLI.
# It exposes a single main() callable that dispatches to one of several
# example subcommands (anycast, unicast, multicast, slim) based on the
# first command line argument. Each imported `main` function below
# comes from a dedicated example module.

# Import the multicast example's main function under an alias for dispatch.
from .multicast import main as multicast_main

# Import the point-to-point ANYCAST example's main function.
from .point_to_point import main_anycast as anycast_main

# Import the point-to-point UNICAST example's main function.
from .point_to_point import main_unicast as unicast_main

# Import the generic slim server example's main function.
from .slim import main as slim_main

# Multi-line help message displayed when the user provides no (or an
# unknown) command. This string deliberately includes usage guidance.
HELP = """
This is the slim bindings examples package.
Available commands:
    - anycast: Demonstrates point-to-point anycast messaging.
    - unicast: Demonstrates point-to-point unicast messaging.
    - multicast: Demonstrates multicast messaging using a channels.
    - slim: Starts a SLIM instance.

Use 'slim-bindings-examples <command>' to run a specific example.
For example: 'slim-bindings-examples anycast'.
"""


def main():
    """
    Entry point for the examples CLI.

    Behavior:
        * Reads the first CLI argument (if any) to select a subcommand.
        * Rewrites sys.argv so the delegated example sees only its own args.
        * Falls back to printing the HELP text if no/unknown command is given.
    """
    # Import inside function to avoid importing sys at module load if
    # this main isn't executed (e.g. introspection tools).
    import sys

    # Ensure at least one argument beyond the program name was provided.
    if len(sys.argv) > 1:
        # Extract the subcommand (first user-supplied arg).
        command = sys.argv[1]
        # Shift sys.argv so the subcommand's own main sees its arguments
        # starting at index 1 (mimics typical python -m package behavior).
        sys.argv = sys.argv[1:]

        # Dispatch to the selected example.
        if command == "anycast":
            # Run the anycast point-to-point messaging example.
            anycast_main()
        elif command == "unicast":
            # Run the unicast (sticky, fixed-destination) example.
            unicast_main()
        elif command == "multicast":
            # Run the multicast (channel/topic) example.
            multicast_main()
        elif command == "slim":
            # Run the slim server example (starts a server endpoint).
            slim_main()
        else:
            # Unknown command: inform the user and show help text.
            print(f"Unknown command: {command}")
            print(HELP)
    else:
        # No subcommand provided: display guidance.
        print("No command provided.")
        print(HELP)


# Standard Python module guard so `python -m slim_bindings_examples`
# or direct execution runs main(), but importing this module does not.
if __name__ == "__main__":
    main()
