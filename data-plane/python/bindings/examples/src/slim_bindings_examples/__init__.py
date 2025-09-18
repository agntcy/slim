# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

from .multicast import main as multicast_main
from .point_to_point import main_anycast as anycast_main
from .point_to_point import main_unicast as unicast_main
from .slim import main as slim_main

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
    # Check what command was provided
    import sys

    if len(sys.argv) > 1:
        command = sys.argv[1]
        sys.argv = sys.argv[1:]

        if command == "anycast":
            anycast_main()
        if command == "unicast":
            unicast_main()
        elif command == "multicast":
            multicast_main()
        elif command == "slim":
            slim_main()
        else:
            print(f"Unknown command: {command}")
            print(HELP)
    else:
        print("No command provided.")
        print(HELP)


if __name__ == "__main__":
    main()
