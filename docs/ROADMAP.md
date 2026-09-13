# FuckExam roadmap

## Current prototype

- GUI in black/red/orange palette;
- host and viewer modes;
- VM/app area capture with local preview;
- local FFmpeg recording;
- one-viewer TCP transport for LAN testing;
- one-way viewer-to-host chat;
- portable runtime data in `FuckExamData`;
- build scripts for Windows EXE and Linux AppImage.

## Next steps for production

1. Replace the test TCP transport with WebRTC and an SFU provider.
2. Add backend-issued short-lived room tokens and server-side roles.
3. Add robust OS-specific window capture adapters and VirtualBox display capture.
4. Add multi-viewer support, reconnection, authentication and moderation.
5. Add signed installers, versioned migrations and end-to-end tests on Windows and Linux.
