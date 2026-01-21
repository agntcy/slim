// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

using Agntcy.Slim;
using Xunit;

namespace Agntcy.Slim.Tests;

/// <summary>
/// Shared fixture that initializes SLIM once for all tests.
/// </summary>
public class SlimFixture : IDisposable
{
    public SlimFixture()
    {
        Slim.Initialize();
    }

    public void Dispose()
    {
        Slim.Shutdown();
    }
}

/// <summary>
/// Collection definition to share the fixture across test classes.
/// </summary>
[CollectionDefinition("Slim")]
public class SlimCollection : ICollectionFixture<SlimFixture> { }

/// <summary>
/// Smoke tests that verify bindings load correctly (no server required).
/// </summary>
[Collection("Slim")]
public class SmokeTests
{
    [Fact]
    public void Initialize_Works()
    {
        Assert.True(Slim.IsInitialized);
    }

    [Fact]
    public void Version_ReturnsValue()
    {
        Assert.NotEmpty(Slim.Version);
    }

    [Fact]
    public void BuildInfo_ReturnsValue()
    {
        var info = Slim.BuildInfo;
        Assert.NotEmpty(info.Version);
    }

    [Fact]
    public void SlimName_ParsesCorrectly()
    {
        using var name = SlimName.Parse("org/app/v1");
        Assert.StartsWith("org/app/v1", name.ToString());
    }

    [Fact]
    public void SlimName_Parse_TrimsWhitespace()
    {
        using var name = SlimName.Parse(" org / app / v1 ");
        Assert.StartsWith("org/app/v1", name.ToString());
    }

    [Theory]
    [InlineData("")]
    [InlineData("   ")]
    [InlineData("invalid")]
    [InlineData("a/b")]
    [InlineData("a/b/c/d")]
    [InlineData("/a/b")]
    [InlineData("a//b")]
    [InlineData("a/b/")]
    public void SlimName_Parse_InvalidFormat_ThrowsArgumentException(string invalidName)
    {
        Assert.Throws<ArgumentException>(() => SlimName.Parse(invalidName));
    }

    [Fact]
    public void SlimName_Parse_Null_ThrowsArgumentNullException()
    {
        Assert.Throws<ArgumentNullException>(() => SlimName.Parse(null!));
    }

    [Fact]
    public void SlimName_Constructor_CreatesValidName()
    {
        using var name = new SlimName("myorg", "myns", "myapp");
        Assert.Contains("myorg", name.ToString());
        Assert.Contains("myns", name.ToString());
        Assert.Contains("myapp", name.ToString());
    }

    [Fact]
    public void SlimSessionConfig_DefaultValues()
    {
        var config = new SlimSessionConfig();
        Assert.Equal(SlimSessionType.PointToPoint, config.SessionType);
        Assert.False(config.EnableMls);
        Assert.Equal(5u, config.MaxRetries);
        Assert.Equal(TimeSpan.FromSeconds(1), config.RetryInterval);
        Assert.Null(config.Metadata);
    }

    [Fact]
    public void SlimException_Properties_Work()
    {
        var ex = new SlimException("Connection timeout occurred");
        Assert.True(ex.IsTimeout);
        Assert.False(ex.IsClosed);
        Assert.True(ex.IsTransient);

        var closedEx = new SlimException("Session closed");
        Assert.True(closedEx.IsClosed);
        Assert.False(closedEx.IsTimeout);
    }

    [Fact]
    public void SlimExceptionExtensions_Work()
    {
        var ex = new Exception("Connection timeout");
        Assert.True(ex.IsTimeoutError());
        Assert.False(ex.IsClosedError());
        Assert.True(ex.IsTransientError());
    }
}

/// <summary>
/// Integration tests that require a running SLIM server.
/// Run server first: cd ../slim/data-plane && task data-plane:run:server
/// These tests run sequentially to avoid connection conflicts.
/// </summary>
[Collection("Slim")]
[Trait("Category", "Integration")]
public class IntegrationTests
{
    private const string ServerEndpoint = "http://localhost:46357";
    private const string SharedSecret = "test-shared-secret-minimum-32-characters!!";

    [Fact]
    public void CreateApp_Succeeds()
    {
        using var service = Slim.GetGlobalService();
        using var app = service.CreateApp("test-org", "create-app-test", "v1", SharedSecret);

        Assert.NotNull(app);
        Assert.True(app.Id > 0);

        // Verify Name property caching works (should return same instance)
        using var name1 = app.Name;
        var name2 = app.Name;
        Assert.Same(name1, name2);

        app.Destroy();
    }

    [Fact]
    public void CreateApp_WithSlimName_Succeeds()
    {
        using var service = Slim.GetGlobalService();
        using var name = new SlimName("test-org", "name-test", "v1");
        using var app = service.CreateApp(name, SharedSecret);

        Assert.NotNull(app);
        Assert.True(app.Id > 0);

        app.Destroy();
    }

    [Fact]
    public void Connect_CreateApp_Subscribe_Succeeds()
    {
        using var service = Slim.GetGlobalService();
        
        // Connect
        var connId = Slim.Connect(ServerEndpoint);
        Assert.True(connId >= 0);

        // Create app
        using var app = service.CreateApp("test-org", "full-test", "v1", SharedSecret);
        Assert.NotNull(app);

        // Subscribe (using cached Name property)
        var appName = app.Name;
        app.Subscribe(appName, connId);

        // Set route
        using var destination = SlimName.Parse("test-org/other-app/v1");
        app.SetRoute(destination, connId);

        // Cleanup
        app.RemoveRoute(destination, connId);
        app.Unsubscribe(appName, connId);
        app.Destroy();
        service.Disconnect(connId);
    }

    [Fact]
    public async Task ConnectAsync_Succeeds()
    {
        using var service = Slim.GetGlobalService();

        var connId = await Slim.ConnectAsync(ServerEndpoint);
        Assert.True(connId >= 0);

        service.Disconnect(connId);
    }

    [Fact]
    public async Task ConnectAsync_WithCancellation_ThrowsWhenCancelled()
    {
        using var cts = new CancellationTokenSource();
        cts.Cancel();

        await Assert.ThrowsAsync<OperationCanceledException>(
            () => Slim.ConnectAsync(ServerEndpoint, cts.Token));
    }
}
