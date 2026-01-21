#!/usr/bin/env python3
"""
Installation test for SLIM Python bindings.

This test verifies that the slim_bindings package can be imported
and initialized successfully.
"""

import slim_bindings


def main():
    print("SLIM Python Bindings Installation Test")
    print("=" * 50)

    # Initialize SLIM (required before any operations)
    slim_bindings.initialize_with_defaults()
    print("SLIM initialized successfully")

    print("Installation test passed!")


if __name__ == "__main__":
    main()
