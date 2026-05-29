// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package io.agntcy.slim.examples.common;

import java.util.List;
import java.util.ArrayList;

/**
 * Configuration classes for SLIM Java examples.
 */
public class Config {
    
    /**
     * Base configuration shared across all examples.
     */
    public static class BaseConfig {
        public String local;
        public String server = Common.DEFAULT_SERVER_ENDPOINT;
        public String sharedSecret = Common.DEFAULT_SHARED_SECRET;
        public boolean enableMls = false;
        public boolean enableOpentelemetry = false;
        
        public BaseConfig(String local) {
            this.local = local;
        }
    }
    
    /**
     * Configuration for Point-to-Point examples.
     */
    public static class PointToPointConfig extends BaseConfig {
        public String remote;
        public String message;
        public int iterations = 10;
        
        public PointToPointConfig(String local) {
            super(local);
        }
        
        public boolean isSender() {
            return message != null && !message.isEmpty();
        }
    }
    
    /**
     * Configuration for Group examples.
     */
    public static class GroupConfig extends BaseConfig {
        public String remote;
        public List<String> invites = new ArrayList<>();
        
        public GroupConfig(String local) {
            super(local);
        }
        
        public boolean isModerator() {
            return remote != null && !invites.isEmpty();
        }
    }
    
    /**
     * Configuration for Server example.
     */
    public static class ServerConfig {
        public String configPath;
        public boolean enableOpentelemetry = false;
        
        public ServerConfig() {
        }
    }
    
    /**
     * Simple CLI argument parser.
     */
    public static class ArgParser {
        private final String[] args;
        private int index = 0;
        
        public ArgParser(String[] args) {
            this.args = args;
        }
        
        public String getOption(String name, String defaultValue) {
            for (int i = 0; i < args.length - 1; i++) {
                if (args[i].equals("--" + name)) {
                    return args[i + 1];
                }
            }
            return defaultValue;
        }
        
        public String getRequiredOption(String name) {
            String value = getOption(name, null);
            if (value == null) {
                throw new IllegalArgumentException("--" + name + " is required");
            }
            return value;
        }
        
        public boolean hasFlag(String name) {
            for (String arg : args) {
                if (arg.equals("--" + name)) {
                    return true;
                }
            }
            return false;
        }
        
        public List<String> getMultipleOptions(String name) {
            List<String> values = new ArrayList<>();
            for (int i = 0; i < args.length - 1; i++) {
                if (args[i].equals("--" + name)) {
                    values.add(args[i + 1]);
                }
            }
            return values;
        }
        
        public int getIntOption(String name, int defaultValue) {
            String value = getOption(name, null);
            return value != null ? Integer.parseInt(value) : defaultValue;
        }
    }
}
