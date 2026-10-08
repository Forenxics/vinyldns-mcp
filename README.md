# vinyldns-mcp

A [Model Context Protocol](https://modelcontextprotocol.io) (MCP) server for
[VinylDNS](https://www.vinyldns.io), the open-source DNS governance platform.
It lets an AI assistant such as Claude look up zones and records, audit
changes, and make DNS changes through VinylDNS. Every change shows a preview
first and needs your explicit confirmation before it is applied.

- **Single static binary**, written in Rust with the official [`rmcp`](https://crates.io/crates/rmcp) SDK; talks MCP over stdio.
- **Uses the normal VinylDNS REST API with your own access keys**, so VinylDNS ACLs, group ownership, approvals and audit history all still apply.
- **Read-only by default.** Write tools are only registered when `VINYLDNS_MCP_ENABLE_WRITES=true`; zone management and batch review tools also need `VINYLDNS_MCP_ENABLE_ADMIN=true`.
- **Two-step writes.** `plan_*` tools validate the input, look up current state, and return a preview plus a single-use token. `confirm_change` applies it. Where the client supports [elicitation](https://modelcontextprotocol.io/specification/2025-06-18/client/elicitation), the user is asked directly in a confirmation dialog.

> Status: **v0.1.0** (initial release). Tested against VinylDNS 0.20.2. See [CHANGELOG.md](CHANGELOG.md).

## Quick start

### 1. Install

**Prebuilt binary:** download the archive for your platform from the
[Releases](https://github.com/Forenxics/vinyldns-mcp/releases) page, check it
against `SHA256SUMS.txt`, extract it, and put `vinyldns-mcp` on your `PATH`.
Builds are provided for Linux (x86_64 static, ARM64), macOS (Apple silicon,
Intel) and Windows (x86_64). See [docs/RELEASING.md](docs/RELEASING.md#unsigned-binaries)
for the macOS and Windows first-run notes.

```sh
sha256sum -c SHA256SUMS.txt --ignore-missing   # macOS: shasum -a 256 -c …
tar -xzf vinyldns-mcp-v*-x86_64-unknown-linux-musl.tar.gz
vinyldns-mcp-v*/vinyldns-mcp --version
```

**From source** (Rust 1.89+):

```sh
cargo install --path .            # or: cargo build --release → target/release/vinyldns-mcp
```

Run `vinyldns-mcp --help` for a summary of the configuration variables.

### 2. Get VinylDNS credentials

In the VinylDNS portal, open your user menu and choose **Download credentials**.
This gives you an access key and a secret key.

### 3. Register the server with your MCP client

**Claude Code**

```sh
claude mcp add vinyldns \
  -e VINYLDNS_API_URL=https://vinyldns-api.example.com \
  -e VINYLDNS_ACCESS_KEY=... -e VINYLDNS_SECRET_KEY=... \
  -- vinyldns-mcp
```

**Claude Desktop** (`claude_desktop_config.json`) and other JSON-configured clients:

```json
{
  "mcpServers": {
    "vinyldns": {
      "command": "vinyldns-mcp",
      "env": {
        "VINYLDNS_API_URL": "https://vinyldns-api.example.com",
        "VINYLDNS_ACCESS_KEY": "your-access-key",
        "VINYLDNS_SECRET_KEY": "your-secret-key",
        "VINYLDNS_MCP_ENABLE_WRITES": "false"
      }
    }
  }
}
```

Then ask something like *"Which zones can I see in VinylDNS?"*, or with writes
enabled, *"Point api.example.com at 10.0.0.5 with a 5 minute TTL"*.

## Tools

| Tool | Kind | Purpose |
|---|---|---|
| `check_connection` | read | Server status and whether the credentials are accepted |
| `list_zones`, `get_zone` | read | Find zones (by ID or name) |
| `list_zone_changes` | read | History of zone setting changes |
| `list_record_sets`, `search_record_sets`, `get_record_set` | read | Records in a zone, or searched across all zones |
| `list_record_set_changes`, `get_record_set_change` | read | Record change audit trail and the status of a change |
| `list_batch_changes`, `get_batch_change` | read | Batch changes and their per-change status and errors |
| `list_groups`, `get_group`, `list_group_members`, `get_user` | read | Ownership and membership lookups |
| `plan_create_record_set` | write (plan) | Preview a new record set; warns about conflicts |
| `plan_update_record_set` | write (plan) | Before/after diff; unchanged fields (including the owner group) are kept |
| `plan_delete_record_set` | write (plan) | Shows exactly what will be removed |
| `plan_batch_change` | write (plan) | Multi-record and multi-zone change |
| `plan_cancel_batch_change` | write (plan) | Cancel your own batch change that is still waiting for review |
| `confirm_change` | write | Applies a plan (and asks the user, when the client supports it) |
| `list_pending_changes`, `discard_pending_change` | write | Manage plans that have not been applied yet |
| `list_deleted_zones`, `list_backend_ids` | read | Deleted (abandoned) zones; DNS backends configured on the server |
| `plan_approve_batch_change`, `plan_reject_batch_change` | admin (plan) | Review batch changes waiting for manual approval (support/super users) |
| `plan_connect_zone` | admin (plan) | Bring an existing DNS zone under VinylDNS management; TSIG secrets are redacted from previews |
| `plan_update_zone` | admin (plan) | Change email, admin group, shared flag, backend or sync schedule; everything else is carried over |
| `plan_sync_zone` | admin (plan) | Re-read the zone from the DNS server |
| `plan_delete_zone` | admin (plan) | Abandon a zone (records stay in DNS); the zone name must be typed again |
| `plan_add_zone_acl_rule`, `plan_delete_zone_acl_rule` | admin (plan) | Grant or revoke access to records in a zone |

Full parameter reference: [docs/TOOLS.md](docs/TOOLS.md).

## How a change is made

```
assistant ──plan_update_record_set──▶ server ──GET current record──▶ VinylDNS
          ◀── preview (before/after) + token ──
user  ◀── assistant shows the preview, asks "apply?" ──
assistant ──confirm_change(token)──▶ server ──(elicitation dialog to the user, if supported)──
                                     server ──PUT signed request──▶ VinylDNS
```

Tokens are single-use and expire after 10 minutes by default. If you decline
in the dialog, the plan is discarded. See [docs/SECURITY.md](docs/SECURITY.md)
for the threat model and recommended client settings.

## Documentation

- [Configuration reference](docs/CONFIGURATION.md)
- [Tool reference](docs/TOOLS.md)
- [Security model](docs/SECURITY.md)
- [Development guide](docs/DEVELOPMENT.md): building, testing, the live smoke test, and how request signing is verified
- [Releasing](docs/RELEASING.md): published binaries and the release process
- [Project status](docs/PROJECT_STATUS.md) and [task list](docs/TASKS.md)
- [Changelog](CHANGELOG.md)

## License

Apache License 2.0, the same license as VinylDNS. See [LICENSE](LICENSE).
This project is not affiliated with Comcast or the VinylDNS maintainers.
