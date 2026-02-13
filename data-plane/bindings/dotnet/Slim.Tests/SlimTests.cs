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
/// Unit tests that verify bindings load correctly (no server required).
/// </summary>
[Collection("Slim")]
public class UnitTests
{
    private const string SharedSecret = "test-shared-secret-must-be-at-least-32-bytes-long!";

    [Fact]
    public void Initialize_Works()
    {
        // Should not throw - already initialized by fixture
        Assert.True(Slim.IsInitialized);
        
        // Multiple calls should be safe
        Slim.Initialize();
        Assert.True(Slim.IsInitialized);
    }

    [Fact]
    public void GetVersion_ReturnsValue()
    {
        var version = Slim.Version;
        Assert.NotEmpty(version);
    }

    [Fact]
    public void AppCreation_Succeeds()
    {
        using var service = Slim.GetGlobalService();
        using var appName = new SlimName("org", "testapp", "v1");
        using var app = service.CreateApp(appName, SharedSecret);

        Assert.NotNull(app);
        Assert.True(app.Id > 0);

        // Test app properties
        var returnedName = app.Name;
        Assert.Contains("org", returnedName.ToString());
        Assert.Contains("testapp", returnedName.ToString());

        app.Destroy();
    }

    [Fact]
    public void NameStructure_CreatesValidName()
    {
        using var name = new SlimName("org", "app", "v1");
        Assert.NotNull(name);
        Assert.Contains("org", name.ToString());
        Assert.Contains("app", name.ToString());
        Assert.Contains("v1", name.ToString());
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
    public void SessionConfig_DefaultValues()
    {
        var config = new SlimSessionConfig
        {
            SessionType = SlimSessionType.PointToPoint,
            EnableMls = false
        };

        Assert.Equal(SlimSessionType.PointToPoint, config.SessionType);
        Assert.False(config.EnableMls);
    }

    [Fact]
    public void SessionConfig_GroupType()
    {
        var config = new SlimSessionConfig
        {
            SessionType = SlimSessionType.Group,
            EnableMls = false
        };

        Assert.Equal(SlimSessionType.Group, config.SessionType);
        Assert.False(config.EnableMls);
    }

    [Fact]
    public void SessionConfig_WithMls()
    {
        var config = new SlimSessionConfig
        {
            SessionType = SlimSessionType.PointToPoint,
            EnableMls = true
        };

        Assert.True(config.EnableMls);
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

    [Fact]
    public void MultipleApps_HaveDifferentIds()
    {
        using var service = Slim.GetGlobalService();
        
        using var app1 = service.CreateApp("org", "app1", "v1", SharedSecret);
        using var app2 = service.CreateApp("org", "app2", "v1", SharedSecret);

        Assert.NotEqual(app1.Id, app2.Id);

        app1.Destroy();
        app2.Destroy();
    }

    [Fact]
    public void Cleanup_Succeeds()
    {
        using var service = Slim.GetGlobalService();
        using var app = service.CreateApp("org", "cleanup", "v1", SharedSecret);

        Assert.NotNull(app);

        // Cleanup
        app.Destroy();
    }
}

/// <summary>
/// Integration tests that require a running SLIM server.
/// Run server first: cd ../slim/data-plane && task data-plane:run:server
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
