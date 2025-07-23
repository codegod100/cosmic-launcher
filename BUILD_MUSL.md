# Building cosmic-launcher with musl

## Prerequisites

1. Install musl cross-compilation toolchain:
   ```bash
   # On Fedora/RHEL
   dnf install musl-gcc musl-devel
   
   # On Ubuntu/Debian
   apt install musl-tools musl-dev
   ```

2. Add the musl target:
   ```bash
   rustup target add x86_64-unknown-linux-musl
   ```

3. Install musl-compatible libxkbcommon:
   ```bash
   # This varies by distribution - you may need to compile from source
   # or install from musl-specific package repositories
   ```

## Building

```bash
cargo build --target x86_64-unknown-linux-musl --release
```

## Notes

- The project is configured to use dynamic linking (`-crt-static` disabled) for better compatibility
- Only libxkbcommon needs to be available as a musl-compatible library
- PKG_CONFIG paths are configured to find musl libraries in `/usr/x86_64-linux-musl/`



  # Install dependencies
  git clone https://github.com/xkbcommon/libxkbcommon.git
  cd libxkbcommon

  # Configure for musl
  ```
  CC=musl-gcc meson setup build-musl --prefix=/usr/x86_64-linux-musl \
    --cross-file=musl-cross.txt


  # Build and install
  ninja -C build-musl
  sudo ninja -C build-musl install

  ```