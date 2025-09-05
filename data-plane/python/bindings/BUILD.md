# SLIM Python Bindings

Bindings to call the SLIM APIs from a python program.

## Build the pypi package

Install [uv](https://github.com/astral-sh/uv) with the preferred method for your
environment. On macOS:

```bash
brew install uv
```

Once uv is installed, run:

```bash
task python-bindings:build
```

This will build the python bindings in debug mode. Once they are built, you can
run the examples in the folder.

## Run SLIM via python bindings

### Server

```bash
task python-bindings:example:server
```

### First client

```bash
task python-bindings:example:alice
```

### Second client

```bash
task python-bindings:example:bob
```
