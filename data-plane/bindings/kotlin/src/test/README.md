# SLIM Kotlin Bindings Tests

This directory contains comprehensive integration tests for the SLIM Kotlin bindings, ported from the Python test suite.

## Quick Start

### Using Task (Recommended)

```bash
# Run all tests
task test

# Run all tests with verbose output
task test:verbose

# Run a specific test class
task test:class TEST_CLASS=BindingsTest

# Run tests matching a pattern
task test:pattern TEST_PATTERN="*Identity*"

# Open test report
task test:report
```

### Using Gradle

```bash
# Run all tests
./gradlew test

# Run all tests with verbose output
./gradlew testVerbose

# Run a specific test class
./gradlew testClass -PtestClass=BindingsTest

# Run tests matching a pattern
./gradlew testPattern -PtestPattern="*Identity*"
```

## Available Test Tasks

### `test` (default)
Standard test execution with basic output showing passed/failed tests.

```bash
./gradlew test
```

### `testVerbose`
Runs all tests with verbose output including standard output and error streams.

```bash
./gradlew testVerbose
```

**Use this when:**
- You need to see print statements and logging output
- Debugging test failures
- Understanding test execution flow

### `testClass`
Runs a specific test class by name (without package prefix).

```bash
# Run BindingsTest
./gradlew testClass -PtestClass=BindingsTest

# Run IdentityTest
./gradlew testClass -PtestClass=IdentityTest

# Available test classes:
# - BindingsTest
# - SessionMetadataTest
# - PointToPointTest
# - GroupTest
# - IdentityTest
```

**Use this when:**
- You want to run all tests in a single test class
- Iterating on a specific test file

### `testPattern`
Runs tests matching a specific pattern.

```bash
# Run all identity-related tests
./gradlew testPattern -PtestPattern="*Identity*"

# Run all tests with "EndToEnd" in the name
./gradlew testPattern -PtestPattern="*EndToEnd*"

# Run all metadata tests
./gradlew testPattern -PtestPattern="*Metadata*"
```

**Use this when:**
- You want to run a subset of tests across multiple files
- Testing a specific feature area

## Standard Gradle Test Options

### Run Specific Test Method
```bash
./gradlew test --tests "io.agntcy.slim.bindings.BindingsTest.testEndToEnd"
```

### Run Tests in Parallel
```bash
./gradlew test --parallel
```

### Continue on Failure
```bash
./gradlew test --continue
```

### Show More Information
```bash
./gradlew test --info
./gradlew test --debug
```

### Clean Before Testing
```bash
./gradlew clean test
```

## Test Reports

After running tests, Gradle generates HTML reports:

```bash
# Location
build/reports/tests/test/index.html

# Open in browser (macOS)
open build/reports/tests/test/index.html

# Open in browser (Linux)
xdg-open build/reports/tests/test/index.html
```

The report includes:
- Test execution time
- Success/failure statistics
- Stack traces for failures
- Standard output/error for each test

## Test Structure

```
src/test/kotlin/io/agntcy/slim/bindings/
├── TestHelpers.kt           # Shared test utilities and fixtures
├── BindingsTest.kt          # End-to-end integration tests (5 tests)
├── SessionMetadataTest.kt   # Metadata propagation tests (1 test)
├── PointToPointTest.kt      # Sticky session tests (1 test)
├── GroupTest.kt             # Group messaging tests (1 test)
└── IdentityTest.kt          # JWT identity tests (3 tests)

src/test/resources/testdata/
├── ec256.pem                # EC256 private key for JWT tests
├── ec256-public.pem         # EC256 public key for JWT tests
├── ec384.pem                # EC384 private key for JWT tests
└── ec384-public.pem         # EC384 public key for JWT tests
```

## Test Coverage

### BindingsTest.kt (5 tests)
- `testEndToEnd()` - Full round-trip with publish/reply/delete
- `testAutoReconnectAfterServerRestart()` - Resilience testing
- `testErrorOnNonexistentSubscription()` - Error path validation
- `testListenForSessionTimeout()` - Timeout behavior (parameterized)
- `testGetMessageTimeout()` - Message timeout behavior (parameterized)

### SessionMetadataTest.kt (1 test)
- `testSessionMetadataMergeRoundtrip()` - Metadata preservation end-to-end

### PointToPointTest.kt (1 test)
- `testStickySession()` - 10 receivers, 1000 messages, validates affinity
  - Parameterized: MLS enabled/disabled, local/global service

### GroupTest.kt (1 test)
- `testGroup()` - 10 participants relay messages in a ring pattern
  - Parameterized: MLS enabled/disabled

### IdentityTest.kt (3 tests)
- `testIdentityVerification()` - JWT authentication (parameterized)
- `testInvalidSharedSecretTooShort()` - Error handling for short secrets
- `testInvalidSharedSecretEmpty()` - Error handling for empty secrets

**Total: 12 test methods** covering all scenarios from Python test suite

## Examples

### Run All Tests (Quick Check)
```bash
./gradlew test
```

### Debug a Failing Test
```bash
# Run with verbose output to see print statements
./gradlew testVerbose --tests "*testEndToEnd*"
```

### Run All JWT Identity Tests
```bash
./gradlew testClass -PtestClass=IdentityTest
```

### Run Only Sticky Session Tests
```bash
./gradlew testPattern -PtestPattern="*Sticky*"
```

### Full Test Run with Clean Build
```bash
./gradlew clean test --parallel
```

## Continuous Integration

For CI environments:

```bash
# Run all tests with full output and continue on failure
./gradlew clean test --continue --info

# Run tests in parallel for faster execution
./gradlew clean test --parallel --continue
```

## Troubleshooting

### Tests Timing Out
- Some tests use 30-180 second timeouts for long-running scenarios
- Ensure you have sufficient system resources
- Check that no other SLIM servers are running on test ports

### Tests Failing on Port Conflicts
- Tests use various ports (12344-12349, 22345, etc.)
- Ensure these ports are available
- Kill any existing SLIM server processes

### JWT Tests Failing
- Ensure test resources are present in `src/test/resources/testdata/`
- Check that JWT key files have correct permissions

### Running Out of Memory
```bash
# Increase Gradle heap size
./gradlew test -Xmx4g
```

## More Information

For detailed implementation notes, see [TEST_IMPLEMENTATION.md](TEST_IMPLEMENTATION.md)
