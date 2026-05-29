// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package io.agntcy.slim.examples;

import io.agntcy.slim.bindings.BuildInfo;
import io.agntcy.slim.bindings.SlimBindings;

/**
 * Simple test to verify native library loading works
 */
public class TestLibraryLoad {
    public static void main(String[] args) {
        try {
            System.out.println("Testing native library loading...");

            // This will trigger library loading
            String version = SlimBindings.getVersion();
            System.out.println("✅ Native library loaded successfully!");
            System.out.println("SLIM version: " + version);

            BuildInfo buildInfo = SlimBindings.getBuildInfo();
            System.out.println("Build info: " + buildInfo.version());

        } catch (Exception e) {
            System.err.println("❌ Failed to load native library:");
            e.printStackTrace();
            System.exit(1);
        }
    }
}
