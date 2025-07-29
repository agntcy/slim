# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

from .fire_and_forget import main as fire_and_forget_main
from .pubsub import main as pubsub_main
from .request_reply import main as request_reply_main
from .slim import main as slim_main

HELP = """
This is the slim bindings examples package.
Available commands:
    - fire-and-forget: Demonstrates fire-and-forget messaging.
    - request-reply: Demonstrates request-reply messaging.
    - pubsub: Demonstrates publish-subscribe messaging.
    - slim: Starts a SLIM instance.

Use 'slim-bindings-examples <command>' to run a specific example.
For example: 'slim-bindings-examples fire-and-forget'.
"""


def main():
    # Check what command was provided
    import sys

    if len(sys.argv) > 1:
        command = sys.argv[1]
        sys.argv = sys.argv[1:]

        if command == "ff":
            fire_and_forget_main()
        elif command == "rr":
            request_reply_main()
        elif command == "pubsub":
            pubsub_main()
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
