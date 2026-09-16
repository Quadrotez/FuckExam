
## Linux compatibility

The Linux desktop artifact is built on Ubuntu 22.04 and targets `x86_64` with a GLIBC 2.35 baseline. This is intentional: building on newer Ubuntu releases can produce AppImages that require newer GLIBC/GLIBCXX versions and fail on older distributions. The filename includes `x86_64-glibc-2.35` to make the compatibility baseline explicit.

The AppImage still requires a 64-bit x86 Linux system and the runtime desktop dependencies supported by Tauri/WebKitGTK. If the host is ARM64, this artifact is not the correct build; the startup log should report `Exec format error` rather than a GLIBC version error.
