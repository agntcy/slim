=== "Rust"

    ```rust
    use slim_service::Service;
    use slim_service::config::ClientConfig;
    use slim_auth::shared_secret::SharedSecret;
    use slim_datapath::api::ProtoName;

    #[tokio::main]
    async fn main() -> anyhow::Result<()> {
        let service = Service::builder().build("slim/0")?;
        service.run().await?;
        let conn_id = service.connect(ClientConfig::with_endpoint("http://127.0.0.1:46357")).await?;

        let name = ProtoName::from_strings(["myorg", "default", "my-service"]);
        let provider = SharedSecret::new("myorg/default/my-service", "change-me-before-going-to-production")?;
        let verifier = SharedSecret::new("myorg/default/my-service", "change-me-before-going-to-production")?;

        let (app, _rx) = service.create_app(&name, provider, verifier)?;
        app.subscribe(&name, Some(conn_id)).await?;

        println!("App ready: myorg/default/my-service");
        // app and conn_id are passed to create_session in the next tutorial

        Ok(())
    }
    ```

=== "Python"

    ```python
    import asyncio
    import slim_bindings

    async def main():
        slim_bindings.uniffi_set_event_loop(asyncio.get_running_loop())
        slim_bindings.initialize_with_defaults()

        service = slim_bindings.get_global_service()

        client_config = slim_bindings.new_insecure_client_config("http://127.0.0.1:46357")
        conn_id = await service.connect_async(client_config)

        local_name = slim_bindings.Name("myorg", "default", "my-service")
        local_app = service.create_app_with_secret(local_name, "change-me-before-going-to-production")
        await local_app.subscribe_async(local_name, conn_id)

        print(f"App ready: {local_name}, id={local_app.id()}")
        # local_app and conn_id are passed to create_session in the next tutorial

    asyncio.run(main())
    ```

=== "Go"

    ```go
    package main

    import (
        "fmt"
        "log"

        slim "github.com/agntcy/slim-bindings-go/v2"
    )

    func main() {
        slim.InitializeWithDefaults()

        config := slim.NewInsecureClientConfig("http://127.0.0.1:46357")
        connID, err := slim.GetGlobalService().ConnectAsync(config)
        if err != nil {
            log.Fatal(err)
        }

        appName, err := slim.NameFromString("myorg/default/my-service")
        if err != nil {
            log.Fatal(err)
        }

        app, err := slim.GetGlobalService().CreateAppWithSecret(appName, "change-me-before-going-to-production")
        if err != nil {
            log.Fatal(err)
        }
        defer app.Destroy()

        if err := app.SubscribeAsync(app.Name(), &connID); err != nil {
            log.Fatal(err)
        }

        fmt.Printf("App ready: %s, id=%s\n", appName, app.Id())
        // app and connID are passed to create_session in the next tutorial
    }
    ```

=== "Java"

    ```java
    import io.agntcy.slim.bindings.*;

    public class Main {
        public static void main(String[] args) {
            SlimBindings.initializeWithDefaults();
            Service service = SlimBindings.getGlobalService();

            ClientConfig config = SlimBindings.newInsecureClientConfig("http://127.0.0.1:46357");
            Long connId = service.connect(config);

            Name localName = Name.fromString("myorg/default/my-service");
            App app = service.createAppWithSecret(localName, "change-me-before-going-to-production");
            app.subscribe(app.name(), connId);

            System.out.println("App ready: " + localName + ", id=" + app.id());
            // app and connId are passed to create_session in the next tutorial
        }
    }
    ```

=== "Kotlin"

    ```kotlin
    import io.agntcy.slim.bindings.*

    fun main() {
        initializeWithDefaults()
        val service = getGlobalService()

        val clientConfig = newInsecureClientConfig("http://127.0.0.1:46357")
        val connId: ULong = service.connectAsync(clientConfig)

        val localName = Name.fromString("myorg/default/my-service")
        val localApp = service.createAppWithSecret(localName, "change-me-before-going-to-production")
        localApp.subscribeAsync(localName, connId)

        println("App ready: $localName, id=${localApp.id()}")
        // localApp and connId are passed to create_session in the next tutorial
    }
    ```

=== "Node.js"

    ```typescript
    import slimBindings from '@agntcy/slim-bindings';

    async function main() {
        slimBindings.initializeWithDefaults();
        const service = slimBindings.getGlobalService();

        const config = slimBindings.newInsecureClientConfig("http://127.0.0.1:46357");
        const connId = await service.connectAsync(config);

        const localName = new slimBindings.Name("myorg", "default", "my-service");
        const app = service.createAppWithSecret(localName, "change-me-before-going-to-production");
        await app.subscribeAsync(localName, BigInt(connId));

        console.log(`App ready: ${localName}, id=${app.id()}`);
        // app and connId are passed to create_session in the next tutorial
    }

    main();
    ```

=== ".NET"

    ```csharp
    using Agntcy.Slim;

    Slim.Initialize();

    var connId = Slim.Connect("http://127.0.0.1:46357");

    using var localName = SlimName.Parse("myorg/default/my-service");
    using var service = Slim.GetGlobalService();
    var app = service.CreateApp(localName, "change-me-before-going-to-production");
    app.Subscribe(app.Name, connId);

    Console.WriteLine($"App ready: {app.Name}, id={app.Id}");
    // app and connId are passed to CreateSession in the next tutorial
    ```

=== "React Native"

    ```tsx
    import slimBindings from '@agntcy/slim-bindings-react-native';

    await slimBindings.waitForJSIBindings(5000);
    slimBindings.initializeWithDefaults();
    const service = slimBindings.getGlobalService();

    const config = slimBindings.newInsecureClientConfig("http://192.168.1.x:46357");
    const connId = service.connect(config);

    const localName = new slimBindings.Name("myorg", "default", "my-service");
    const app = service.createAppWithSecret(localName, "change-me-before-going-to-production");
    await app.subscribeAsync(localName, connId);

    console.log(`App ready: ${localName}, id=${app.id()}`);
    // app and connId are passed to createSession in the next tutorial
    ```
