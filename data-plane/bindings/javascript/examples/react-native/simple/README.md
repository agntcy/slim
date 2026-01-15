# SLIM React Native - Simple Example

This example demonstrates basic SLIM usage in a React Native application.

## What This Example Shows

- Initializing the crypto provider
- Creating an app with shared secret authentication
- Creating a point-to-point session
- Publishing messages
- Proper cleanup

## Prerequisites

- Node.js 18+
- React Native development environment set up
- iOS: Xcode and CocoaPods
- Android: Android Studio and SDK

## Installation

```bash
# Install dependencies
npm install

# iOS setup
cd ios && pod install && cd ..

# Run on iOS
npx react-native run-ios

# Run on Android
npx react-native run-android
```

## Code

```typescript
import React, { useEffect, useState } from 'react'
import { View, Text, Button, StyleSheet, ScrollView, SafeAreaView } from 'react-native'
import { 
  initializeCryptoProvider,
  getVersion,
  createAppWithSecret,
  Name,
  SessionType,
  type SessionConfig
} from '@agntcy/slim-bindings'

export default function SimpleExample() {
  const [logs, setLogs] = useState<string[]>([])
  const [version, setVersion] = useState<string>('')

  const log = (message: string) => {
    console.log(message)
    setLogs(prev => [...prev, `${new Date().toLocaleTimeString()}: ${message}`])
  }

  useEffect(() => {
    // Initialize on mount
    try {
      initializeCryptoProvider()
      log('✅ Crypto provider initialized')
      
      const ver = getVersion()
      setVersion(ver)
      log(`📦 SLIM version: ${ver}`)
    } catch (error: any) {
      log(`❌ Initialization failed: ${error.message}`)
    }
  }, [])

  const runExample = async () => {
    setLogs([])
    log('🚀 Starting SLIM example...')

    try {
      // Create app
      const appName = new Name(['org', 'myapp', 'v1'], undefined)
      const sharedSecret = 'my-shared-secret-value-must-be-at-least-32-bytes-long!'
      
      log('Creating app with shared secret...')
      const app = createAppWithSecret(appName, sharedSecret)
      log(`✅ App created with ID: ${app.id()}`)
      
      const returnedName = app.name()
      const components = returnedName.components()
      log(`   Name components: ${components.join('/')}`)

      // Create session configuration
      const sessionConfig: SessionConfig = {
        sessionType: SessionType.PointToPoint,
        enableMls: false,
        maxRetries: undefined,
        interval: undefined,
        metadata: {}
      }

      const destination = new Name(['org', 'receiver', 'v1'], undefined)
      log('📡 Creating session to destination...')

      try {
        const session = await app.createSessionAndWait(sessionConfig, destination)
        log('✅ Session created')

        // Publish a message
        const message = new TextEncoder().encode('Hello from React Native! 👋')
        log('📤 Publishing message...')
        
        try {
          await session.publishAndWait(message, undefined, undefined)
          log('✅ Message published successfully')
        } catch (error: any) {
          log(`⚠️  Publish failed (expected without network): ${error.message}`)
        }

        // Cleanup session
        log('🗑️  Cleaning up session...')
        await app.deleteSessionAndWait(session)
        log('✅ Session deleted')
      } catch (error: any) {
        log(`⚠️  Session creation failed (expected without network): ${error.message}`)
      }

      // Test subscription
      const subscriptionName = new Name(['org', 'myapp', 'events'], undefined)
      log('📥 Testing subscription...')
      
      try {
        await app.subscribe(subscriptionName, undefined)
        log('✅ Subscribed successfully')
        
        await app.unsubscribe(subscriptionName, undefined)
        log('✅ Unsubscribed successfully')
      } catch (error: any) {
        log(`⚠️  Subscribe failed (expected without network): ${error.message}`)
      }

      // Cleanup app
      app.destroy()
      log('✅ App destroyed')

      log('\\n✨ Example completed successfully!')
      log('\\n📝 Note: Some operations may fail without a running SLIM network,')
      log('   but the bindings are working correctly if you see this message.')
    } catch (error: any) {
      log(`❌ Error: ${error.message}`)
    }
  }

  return (
    <SafeAreaView style={styles.container}>
      <View style={styles.header}>
        <Text style={styles.title}>🚀 SLIM React Native</Text>
        <Text style={styles.subtitle}>Simple Example</Text>
        {version && <Text style={styles.version}>Version: {version}</Text>}
      </View>

      <Button title="Run Example" onPress={runExample} />

      <ScrollView style={styles.logContainer}>
        {logs.map((log, index) => (
          <Text key={index} style={styles.logText}>{log}</Text>
        ))}
      </ScrollView>
    </SafeAreaView>
  )
}

const styles = StyleSheet.create({
  container: {
    flex: 1,
    padding: 20,
    backgroundColor: '#f5f5f5',
  },
  header: {
    alignItems: 'center',
    marginBottom: 20,
  },
  title: {
    fontSize: 24,
    fontWeight: 'bold',
    marginBottom: 5,
  },
  subtitle: {
    fontSize: 16,
    color: '#666',
    marginBottom: 10,
  },
  version: {
    fontSize: 12,
    color: '#999',
  },
  logContainer: {
    marginTop: 20,
    flex: 1,
    backgroundColor: '#1e1e1e',
    borderRadius: 8,
    padding: 10,
  },
  logText: {
    fontFamily: 'Courier',
    fontSize: 12,
    color: '#00ff00',
    marginBottom: 2,
  },
})
```

## Expected Behavior

The example will:
1. Initialize the crypto provider
2. Display the SLIM version
3. Create an app with a shared secret
4. Create a session (may fail without network - this is expected)
5. Attempt to publish a message
6. Test subscription operations
7. Clean up resources

**Note**: Without a running SLIM server, network operations will fail with errors. This is expected behavior and demonstrates that the bindings are working correctly.

## Troubleshooting

### iOS Build Errors

```bash
cd ios
pod install
cd ..
```

### Android Build Errors

Make sure you have the Android NDK installed and configured.

### Binding Not Found

Regenerate bindings:

```bash
cd ../../
task generate:react-native
```

## Related Examples

- [Point-to-Point Example](../point-to-point/) - Alice/Bob messaging
- [Group Example](../group/) - Multi-party messaging
