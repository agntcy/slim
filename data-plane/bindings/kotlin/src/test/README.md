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