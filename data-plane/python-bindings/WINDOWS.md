## Windows

Instructions to compile `slim_bindings` wheel on Windows.

## 1. Build Requirements

1. **Rust Toolchain**
   - [Install Rust](https://www.rust-lang.org/tools/install)
   - Make sure `cargo`, `rustc` are in your PATH.
2. **Python**
   - Install a Python version (3.9–3.13).
   - **Optional**: Use [pyenv-win](https://github.com/pyenv-win/pyenv-win) or
     another Python version manager if you need multiple versions.
3. **Maturin**
   - You can install Maturin via `pip install maturin` or `cargo install
     maturin`.
4. **Task/`taskfile`**
   - If you’re using [go-task](https://taskfile.dev/) or a similar tool, make
     sure it’s installed.
   - Alternatively, if `task` is just a script/alias in your project, ensure
     it’s executable.
5. Install/verify you have "Desktop development with C++" in Visual Studio.

   In particular, make sure you have the following key items checked, which are
   most important for MSVC + CMake builds:

   - MSVC v143 – VS 2022 C++ x64/x86 build tools
   - C++ CMake Tools for Windows
   - Windows 11 SDKs

## 2. Run the Build Locally

Clone <https://github.com/agntcy/slim> and change to folder
`data-plane\python-bindings`

Inside this folder (where the Taskfile is), you can run:

```bash
# List all available tasks
task

# Build Python bindings in debug mode:
task python-bindings:build
```

This will build the wheel under a temporary folder that will be removed
immediately but serves to test if the toolchain is correctly setup.

You should see a similar output:

```Powershell
Built wheel for CPython 3.13 to C:\Users\dummy\AppData\Local\Temp\.tmpYMjkNn\slim_bindings-0.1.7-cp313-cp313-win_amd64.whl
```

### Build Bindings to Dist Directory

1. Disable any cloud data syncing programs, such as `OneDrive` or `Dropbox`,
   that may be monitoring the folder you are working on. Otherwise, the build
   will fail due to file locking issues.

2. Execute maturin

   ```powershell
   maturin build --release --out dist
   ```

   You should see a similar output:

   ```Powershell
   📦 Built wheel for CPython 3.13 to dist\slim_bindings-0.1.7-cp313-cp313-win_amd64.whl
   ```

## 3. Install Wheel and Verify the Installation

```Powershell
pip install .\dist\slim_bindings-0.1.7-cp313-cp313-win_amd64.whl
```

### Verify

It is very important that the path displayed **points to your virtual
environment** and not to the folder `slim_bindings`

```Powershell
cd slim\data-plane\
python -c "import slim_bindings; print(slim_bindings.__file__)"
```

That should show a path to the installed slim_bindings in your virtual
environment’s Lib\site-packages. Example:

```Powershell
slim\data-plane\python-bindings\.venv\Lib\site-packages\slim_bindings\__init__.py
```

## 4. Troubleshooting on Windows

- **MSVC / cl.exe** not found: Make sure you installed the **"Desktop
  development with C++"** workload in Visual Studio Installer and that you’re
  building in a Developer Command Prompt.
- **File Tracker (FTK1011) or Temp Directory** errors: If you see warnings about
  building from `Temp`, try changing or shortening your Windows temp directory
  [as discussed in previous
  steps](https://docs.microsoft.com/en-us/cpp/build/reference/filetracker).
- **There’s an Old \_slim_bindings.pyd or a Naming Conflict**

  Sometimes you can end up with two .pyd files or an out-of-date file in
  `slim_bindings`. This can confuse Python or Maturin. If you see multiple
  \_slim_bindings.cpXYZ-win_amd64.pyd files, remove the duplicates.
