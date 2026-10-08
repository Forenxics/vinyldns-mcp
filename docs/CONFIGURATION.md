# Configuration

All configuration comes from environment variables, which the MCP client sets
when it launches the server. Invalid values stop the server at startup with a
message that names the variable.

| Variable | Required | Default | Description |
|---|---|---|---|
| `VINYLDNS_API_URL` | yes | – | Base URL of the VinylDNS **API** (not the portal), e.g. `https://vinyldns-api.example.com`. A path prefix is allowed (`https://gw.example.com/vinyldns`). |
| `VINYLDNS_ACCESS_KEY` | yes | – | Your VinylDNS access key. |
| `VINYLDNS_SECRET_KEY` | yes | – | Your VinylDNS secret key. It is never logged or shown in debug output. |
| `VINYLDNS_MCP_ENABLE_WRITES` | no | `false` | When `true`, registers the `plan_*`, `confirm_change` and pending-change tools. When `false`, those tools do not exist for the client at all. |
| `VINYLDNS_MCP_ENABLE_ADMIN` | no | `false` | When `true`, also registers the zone management and batch review tools (`plan_connect_zone`, `plan_update_zone`, `plan_sync_zone`, `plan_delete_zone`, ACL rule tools, `plan_approve_batch_change`, `plan_reject_batch_change`). Requires `VINYLDNS_MCP_ENABLE_WRITES=true`; the server refuses to start otherwise. |
| `VINYLDNS_MCP_CONFIRMATION` | no | `auto` | How `confirm_change` gets human approval: `auto` uses an elicitation dialog when the client supports it and otherwise relies on the client's tool approval prompt; `elicit` always requires a dialog and refuses to apply changes on clients without elicitation support; `token` never shows a dialog. |
| `VINYLDNS_MCP_PENDING_TTL_SECS` | no | `600` | How long a plan can be confirmed before it expires. |
| `VINYLDNS_HTTP_TIMEOUT_SECS` | no | `30` | Timeout for each VinylDNS API request. |
| `VINYLDNS_SIGNING_REGION` | no | `us-east-1` | Region in the request signature's credential scope. VinylDNS does not check it. |
| `VINYLDNS_SIGNING_SERVICE` | no | `VinylDNS` | Service name in the credential scope. VinylDNS does not check it. |
| `VINYLDNS_MCP_LOG` | no | `info` | Log filter in [`tracing` `EnvFilter`](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html) syntax, e.g. `debug` or `vinyldns_mcp=debug`. Logs go to **stderr**, because stdout carries the MCP protocol. |
| `HTTPS_PROXY` / `HTTP_PROXY` / `NO_PROXY` | no | – | Standard proxy variables. The system proxy configuration is honored. |

## Recommended profiles

**Exploration and auditing:** leave writes off (the default).

**Day-to-day changes in a client with confirmation dialogs** (for example Claude
Code or Claude Desktop):

```
VINYLDNS_MCP_ENABLE_WRITES=true
VINYLDNS_MCP_CONFIRMATION=auto
```

**Strict:** changes are only possible through an interactive dialog:

```
VINYLDNS_MCP_ENABLE_WRITES=true
VINYLDNS_MCP_CONFIRMATION=elicit
```

**Zone administrators and reviewers:** add the admin tools.

```
VINYLDNS_MCP_ENABLE_WRITES=true
VINYLDNS_MCP_ENABLE_ADMIN=true
VINYLDNS_MCP_CONFIRMATION=elicit
```

Only enable this for users who administer zones or review batch changes.
Approving and rejecting batch changes also needs a VinylDNS support or super
user.

## Notes

- Use `https://` for any non-local API. The server logs a warning if
  credentials would be sent over plain `http` to a remote host. Requests are
  signed, so the secret itself never leaves your machine, but the request
  contents would travel unencrypted.
- Use a separate VinylDNS user for automation if you want its changes to be
  easy to tell apart in the VinylDNS audit history.
