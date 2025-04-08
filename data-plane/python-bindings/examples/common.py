# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0


class color:
    PURPLE = "\033[95m"
    CYAN = "\033[96m"
    DARKCYAN = "\033[36m"
    BLUE = "\033[94m"
    GREEN = "\033[92m"
    YELLOW = "\033[93m"
    RED = "\033[91m"
    BOLD = "\033[1m"
    UNDERLINE = "\033[4m"
    END = "\033[0m"


def format_message(message1, message2=""):
    return f"{color.BOLD}{color.CYAN}{message1.capitalize():<45}{color.END}{message2}"


def split_id(id):
    # Split the IDs into their respective components
    try:
        local_organization, local_namespace, local_agent = id.split("/")
    except ValueError as e:
        print("Error: IDs must be in the format organization/namespace/agent.")
        raise e

    return local_organization, local_namespace, local_agent
