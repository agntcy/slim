# Simple Example

Basic demonstration of SLIM Python bindings functionality.

## What It Does

- Initializes the SLIM crypto provider
- Displays version and build information
- Creates a SLIM app with shared secret authentication
- Shows how to construct session configurations
- Demonstrates basic error handling

## Running

```bash
cd data-plane/bindings/python
task example
```

Or directly:
```bash
cd examples/simple
python main.py
```

## Code Walkthrough

1. **Import bindings**: Import the generated `slim_uniffi` module
2. **Initialize crypto**: Call `slim.initialize_crypto_provider()`
3. **Create app**: Use `slim.create_app_with_secret()` with a name and shared secret
4. **Get app info**: Access `app.id()` and `app.name()`
5. **Session config**: Create configuration dictionaries for sessions

## Example Output

```
============================================================
SLIM Python Bindings - Simple Example
============================================================

SLIM Version: 0.7.0
Build Info:
  Version: 0.7.0
  Git SHA: abc123
  Build Date: 2024-01-15T10:30:00Z
  Profile: release

✅ App created successfully!
   App ID: 12345678901234567890
   App Name: org/example/simple-app
   Name ID: 12345678901234567890

============================================================
Example completed successfully!
============================================================
```

## Key Concepts

### Name Structure
Names in SLIM have three components:
```python
{
    'components': ['org', 'namespace', 'app'],
    'id': None  # Auto-generated from auth token
}
```

### Session Configuration
```python
{
    'session_type': 'PointToPoint',  # or 'Group'
    'enable_mls': False,
    'max_retries': 3,
    'interval_ms': 100,
    'initiator': True,
    'metadata': {}
}
```

