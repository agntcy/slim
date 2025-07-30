# SLIM Python Bindings

Bindings to call the SLIM APIs from a python program.

## Installation

```bash
pip install slim-bindings
```

## Include as dependency

### With pyproject.toml

```toml
[project]
name = "slim-example"
version = "0.1.0"
description = "Python program using SLIM"
requires-python = ">=3.9"
dependencies = [
    "slim-bindings>=0.1.0"
]
```

### With poetry project

```toml
[tool.poetry]
name = "slim-example"
version = "0.1.0"
description = "Python program using SLIM"

[tool.poetry.dependencies]
python = ">=3.9,<3.14"
slim-bindings = ">=0.1.0"
```

## Example programs

Example apps can be found in the [repo](https://github.com/agntcy/slim/tree/main/data-plane/python-bindings/examples/src/slim_bindings_examples)
