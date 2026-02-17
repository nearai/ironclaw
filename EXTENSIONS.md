# IronClaw Extensions Matrix

This document tracks features and capabilities that extend IronClaw's core functionality, specifically focusing on IoT, Robotics, and Edge Computing capabilities.

**Legend:**
- ✅ Implemented
- 🚧 Partial (in progress or incomplete)
- ❌ Not implemented
- 🔮 Planned

---

## 1. Edge & IoT Hardware Support

| Feature | Status | Architecture | Notes |
|---------|--------|--------------|-------|
| **ARM64 Build Target** | ✅ | `aarch64-unknown-linux-gnu` | Supported via standard Cargo cross-compilation |
| **Edge Deployment Guide** | 🚧 | Systemd + Binary | Setup script created; full guide pending |
| **MCP: GPIO** | ✅ | MCP Server (Rust) | **Edge Extension**. Control GPIO (Digital I/O) with allowlists & rate limits. |
| **MCP: I2C/SPI** | ❌ | MCP Server | Low-level bus access for sensors |
| **MCP: Camera** | ❌ | MCP Server | Capture images/video streams |
| **MCP: ROS2 Bridge** | ❌ | MCP Server | Bridge to Robot Operating System (Topics/Services) |
| **Hardware "Simulation Mode"** | ❌ | Host Mocking | For dev/CI without physical hardware |

## 2. Security & Safety (Edge)

| Feature | Status | Complexity | Notes |
|---------|--------|------------|-------|
| **Hardware Daemon** | ✅ | High | Systemd service `ironclaw-gpio` managing /dev/ access |
| **Capability Tokens** | ❌ | Medium | Auth tokens for MCP servers to restrict agent access |
| **Pin/Bus Allowlisting** | ✅ | High | Implemented in `mcp-gpio` (args: `--allow-out`, `--allow-in`) |
| **Rate Limiting (Hardware)** | ✅ | Medium | Implemented in `mcp-gpio` (arg: `--rate-limit-ms`) |
| **Emergency Stop (E-STOP)** | ❌ | High | Hardware or software override to kill agent control |

## 3. Eventing & Telemetry

| Feature | Status | Notes |
|---------|--------|-------|
| **MCP: Pub/Sub** | ❌ | Subscribe to sensor changes (interrupts) |
| **Telemetry Buffering** | ❌ | Store-and-forward for offline edge devices |
| **Health Monitoring** | ❌ | Watchdog for agent/MCP server liveness |

---

## Architecture Pattern: "Secure Edge"

The standard deployment pattern for IronClaw on Edge devices follows the **Hardware Daemon + MCP** model:

1.  **Hardware Daemon**: A privileged process (running as root or `gpio` group) that owns the hardware interfaces.
2.  **MCP Servers**: Lightweight bridges that translate MCP protocol requests to hardware calls.
3.  **Sandboxed Agent**: The IronClaw agent runs in its standard WASM sandbox and communicates with hardware *only* via MCP.
4.  **Network Isolation**: The agent has no direct network access; it communicates with the cloud via the IronClaw Gateway if needed.

```mermaid
graph TD
    subgraph "Host OS (Raspberry Pi)"
        HW[Hardware /dev/gpio] <--> Daemon[Hardware Daemon / MCP Server]
        Daemon <-- MCP Protocol --> Agent[IronClaw Agent (WASM)]
        Agent -- "Sandboxed" --> Runtime[IronClaw Runtime]
    end
```
