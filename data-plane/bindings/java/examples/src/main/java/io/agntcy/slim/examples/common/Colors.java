// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package io.agntcy.slim.examples.common;

/**
 * ANSI color codes for terminal output.
 */
public class Colors {
    
    // ANSI escape codes
    public static final String RESET = "\033[0m";
    public static final String CYAN = "\033[96m";
    public static final String YELLOW = "\033[93m";
    public static final String GREEN = "\033[92m";
    public static final String RED = "\033[91m";
    public static final String BLUE = "\033[94m";
    
    /**
     * Format a message with color.
     * 
     * @param color ANSI color code
     * @param message Message to format
     * @return Colored message string
     */
    public static String colored(String color, String message) {
        return color + message + RESET;
    }
    
    /**
     * Format an instance prefix with color.
     * 
     * @param instanceId Instance ID number
     * @return Formatted prefix like "[123] " in cyan
     */
    public static String instancePrefix(long instanceId) {
        return colored(CYAN, "[" + instanceId + "]") + " ";
    }
}
