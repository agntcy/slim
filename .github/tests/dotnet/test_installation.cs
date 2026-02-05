// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

using System;
using Agntcy.Slim;

/// <summary>
/// Installation test for SLIM .NET bindings.
/// 
/// This test verifies that the slim_bindings package can be imported
/// and initialized successfully.
/// </summary>
class Program
{
    static void Main(string[] args)
    {
        Console.WriteLine("SLIM .NET Bindings Installation Test");
        Console.WriteLine("==================================================");

        try
        {
            // Initialize SLIM (required before any operations)
            Slim.Initialize();
            Console.WriteLine("SLIM initialized successfully");

            // Verify initialization state
            if (!Slim.IsInitialized)
            {
                Console.WriteLine("SLIM initialization check failed");
                Environment.Exit(1);
            }
            Console.WriteLine("SLIM is initialized");

            // Get and display version information
            string version = Slim.Version;
            Console.WriteLine($"SLIM version: {version}");

            // Get and display build information
            string buildInfo = Slim.BuildInfo;
            Console.WriteLine($"SLIM build info: {buildInfo}");

            // Clean shutdown
            Slim.Shutdown();
            Console.WriteLine("SLIM shutdown successfully");

            Console.WriteLine("Installation test passed!");
            Environment.Exit(0);
        }
        catch (Exception ex)
        {
            Console.WriteLine($"Test failed with exception: {ex.Message}");
            Console.WriteLine(ex.StackTrace);
            Environment.Exit(1);
        }
    }
}
