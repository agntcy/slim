# SLIM Test App

React Native test application for end-to-end testing of the SLIM JavaScript bindings.

## Purpose

This app tests that the generated JSI bindings work correctly on iOS and Android devices/simulators. It runs smoke tests that verify:

- Crypto provider initialization
- Version retrieval
- Name object creation
- App creation with shared secrets

## Prerequisites

### iOS
- macOS with Xcode installed
- CocoaPods: `sudo gem install cocoapods`
- iOS Simulator

### Android
- Android Studio with SDK installed
- Android Emulator configured
- Java 17+

### Testing Tool
- Maestro: `curl -Ls "https://get.maestro.mobile.dev" | bash`

## Setup

From the project root (`data-plane/bindings/javascript`):

```bash
# Set up the test app
task test:e2e:setup
```

This will:
1. Install npm dependencies
2. Install iOS pods
3. Link the local SLIM bindings

## Running Tests

### iOS

```bash
# From project root
task test:e2e:ios
```

Or manually:
```bash
cd examples/react-native/test-app

# Build and run
npx react-native run-ios

# In another terminal, run Maestro tests
maestro test maestro/smoke-tests.yaml
```

### Android

```bash
# From project root
task test:e2e:android
```

Or manually:
```bash
cd examples/react-native/test-app

# Start emulator first
emulator -avd Pixel_8_API_34 &

# Build and run
npx react-native run-android

# In another terminal, run Maestro tests
maestro test maestro/smoke-tests.yaml
```

## Test Structure

### App.tsx
The main test UI that:
- Loads the SLIM bindings module
- Provides a "Run Tests" button
- Executes smoke tests
- Displays results with pass/fail status

### maestro/smoke-tests.yaml
Maestro test definition that:
- Launches the app
- Taps "Run Tests"
- Waits for completion
- Verifies all tests passed
- Checks for no failures

## Troubleshooting

### "SLIM bindings module not loaded"
The bindings weren't properly linked. Try:
```bash
cd examples/react-native/test-app
rm -rf node_modules
npm install
cd ios && pod install && cd ..
```

### iOS build fails
```bash
cd ios
pod deintegrate
pod install
cd ..
npx react-native run-ios --clean
```

### Android build fails
```bash
cd android
./gradlew clean
cd ..
npx react-native run-android
```

### Maestro can't find app
Make sure the app is running on the simulator/emulator before running Maestro tests.

## CI Integration

This test app is used in GitHub Actions CI:
- `.github/workflows/test-javascript-bindings.yml`
- Runs on both iOS (macOS runners) and Android (Ubuntu + emulator)
- Tests are executed automatically on PR and push to main

## Development

To add new tests:

1. Add test logic to `App.tsx` in the `runTests()` function
2. Update `maestro/smoke-tests.yaml` to verify the new test
3. Test locally on both platforms
4. Verify in CI

## Architecture

```
App.tsx
  ↓ imports
@agntcy/slim-bindings (local)
  ↓ uses
generated/typescript/slim_bindings.ts
  ↓ calls
generated/cpp/slim_bindings.cpp (JSI layer)
  ↓ calls
Rust library (libslim_bindings.a)
```

## Related Documentation

- [Maestro Documentation](https://maestro.mobile.dev/)
- [React Native Testing](https://reactnative.dev/docs/testing-overview)
- [SLIM JavaScript Bindings](../../README.md)
