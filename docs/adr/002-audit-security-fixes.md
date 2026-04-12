# ADR-002: Security Audit Fixes

Status: accepted
Date: 2025-07-24

## Context

A comprehensive security audit was performed on convergio-ipc covering:
SQL injection, path traversal, command injection, SSRF, secret exposure,
race conditions, unsafe blocks, input validation, auth/authz, and
message deserialization attacks.

## Findings and Fixes

### CRITICAL: Race condition in file lock acquisition

**Finding:** `locks::acquire()` used a deferred SQLite transaction (`unchecked_transaction`).
Two concurrent agents could both SELECT "no lock held" before either INSERTs,
causing the second to hit a UNIQUE constraint error instead of a clean `Rejected` result.

**Fix:** Upgraded to `BEGIN IMMEDIATE` transaction, which acquires a write lock before
the SELECT, serializing concurrent lock acquisitions properly.

### MEDIUM: Unsafe PID cast overflow (agents.rs, locks.rs)

**Finding:** `pid as i32` silently truncates `u32`/`i64` values exceeding `i32::MAX`,
potentially sending `kill(0)` to signal all processes in the group (PID 0) or
targeting wrong PIDs due to two's complement wrapping.

**Fix:** Replaced with `i32::try_from(pid)` with explicit bounds checking.
PIDs ≤ 0 or > i32::MAX now safely return `false` (not alive).

### MEDIUM: UTF-8 panic in process_scanner truncate()

**Finding:** `&s[..max]` byte-index slicing panics if `max` falls within a
multi-byte UTF-8 character (e.g., emoji 🌍 = 4 bytes).

**Fix:** Use `char_indices()` to find the last valid character boundary before `max`.

### MEDIUM: No input validation on API endpoints

**Finding:** `handle_send`, `handle_register_agent`, and `handle_context_set`
accepted unbounded strings, allowing memory exhaustion via oversized payloads.

**Fix:** Added length validation constants and checks:
- Agent names: 128 bytes
- Content/values: 64 KiB
- Keys: 256 bytes
- Metadata: 4 KiB
- Msg types: 64 bytes

### Passed checks (no issues found)

| Check | Result | Notes |
|-------|--------|-------|
| SQL injection | PASS | All queries use parameterized `?N` placeholders |
| Path traversal | PASS | File locks are DB keys, no filesystem I/O |
| Command injection | PASS | `ps aux` is hardcoded, no user input |
| SSRF | PASS | Probes only hardcoded localhost addresses |
| Secret exposure | PASS | No secrets in source code |
| Auth/AuthZ | N/A | Local daemon, auth handled at network layer |
| Deserialization | PASS | Malformed JSON silently ignored, no panics |

## Consequences

- File lock acquisition is now safe under concurrent access
- PID-based operations cannot cause undefined behavior via overflow
- Process scanner handles non-ASCII command lines safely
- API endpoints reject oversized inputs with clear error messages
- Test count increased from 41 to 42 (new UTF-8 boundary test)
