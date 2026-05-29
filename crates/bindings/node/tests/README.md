# Integration Tests for SLIM Node.js Bindings

Integration tests validate point-to-point communication between sender (Bob) and receiver (Alice) using an embedded SLIM server. Tests verify bidirectional message exchange and multi-message sessions.

## Running Tests

```bash
# Using Task (recommended)
task test:integration

# Using npm (from tests directory)
cd tests && npm test
```
