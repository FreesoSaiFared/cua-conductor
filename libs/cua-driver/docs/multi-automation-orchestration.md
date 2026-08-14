# Multi-Automation Orchestration

`cua-conductor` is a thin supervisor beside cua-driver. It does not replace cua-driver's MCP, CLI, SDK, capture, or input paths.

## Phase 1 model

Configured automations are ordinary child processes. Each record supplies:

- stable automation id;
- executable plus argument vector;
- kind label;
- target hint for the warning surface;
- optional exclusive group.

A run receives a UUID session id. Session state is written under the conductor data directory as JSON so separate CLI invocations can list or stop runs without an in-memory daemon.

Exclusive groups provide the first coordination primitive. Two still-live sessions in the same non-empty group cannot be launched concurrently through the conductor.
## CLI

Current commands:

```text
cua-conductor init [--force]
cua-conductor list
cua-conductor run <id>
cua-conductor status
cua-conductor stop <session-id>
cua-conductor overlay-test [--seconds N] [--label TEXT]
```

On Windows, state lives under `%LOCALAPPDATA%\CuaConductor`. Other platforms use the XDG data directory or `~/.local/share/cua-conductor`.

## Scope

The conductor may supervise cua-driver, browser automation, Python, PowerShell, shell programs, or other CLIs. Phase 1 treats them uniformly as processes and does not attempt to infer their internal actions.

Future action-event integration should enrich session state rather than remove the process-lifetime warning invariant.
