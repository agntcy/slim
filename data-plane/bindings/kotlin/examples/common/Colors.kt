// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package io.agntcy.slim.examples.common

/**
 * ANSI color codes for terminal output formatting.
 */
object Colors {
    const val PURPLE = "\u001B[95m"
    const val CYAN = "\u001B[96m"
    const val DARKCYAN = "\u001B[36m"
    const val BLUE = "\u001B[94m"
    const val GREEN = "\u001B[92m"
    const val YELLOW = "\u001B[93m"
    const val RED = "\u001B[91m"
    const val BOLD = "\u001B[1m"
    const val UNDERLINE = "\u001B[4m"
    const val END = "\u001B[0m"
    
    /**
     * Format a message with bold cyan prefix column and optional suffix.
     * 
     * @param message1 Primary label (left column, capitalized & padded)
     * @param message2 Optional trailing description/value
     * @return Colorized string ready to print
     */
    fun formatMessage(message1: String, message2: String = ""): String {
        return "$BOLD$CYAN${message1.capitalize().padEnd(45)}$END$message2"
    }
    
    /**
     * Print a formatted message using formatMessage().
     */
    fun printFormatted(message1: String, message2: String = "") {
        println(formatMessage(message1, message2))
    }
}
