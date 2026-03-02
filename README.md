# Repository Layout

This repository is organized for a multi-project setup:

- `mcp/`: Rust MCP server (`train-mcp`)
- `ios/`: Native iOS SwiftUI app (`TrainiOS`)

## Quick Start

### MCP server

```bash
cd mcp
cargo run
```

MCP usage/config docs are in `mcp/README.md`.

### iOS app

```bash
cd ios
xcodegen generate
open TrainiOS.xcodeproj
```

CLI build check:

```bash
cd ios
xcodebuild -project TrainiOS.xcodeproj -scheme TrainiOS -destination 'generic/platform=iOS Simulator' build
```
