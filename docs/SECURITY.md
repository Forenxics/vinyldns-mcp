# Security model

DNS mistakes take effect immediately and can cause outages, so this server is
built around one rule: **an AI assistant cannot change DNS without the human
seeing exactly what will change and approving it.**

## Layers of protection

1. **VinylDNS authorization still applies.** The server makes ordinary signed
   API calls with *your* credentials. It cannot do anything your VinylDNS user
   cannot do. Zone ACLs, shared-zone ownership, manual review of batch changes,
   and the audit trail all apply as usual.
2. **Read-only by default.** Without `VINYLDNS_MCP_ENABLE_WRITES=true`, the
   write tools are not registered, so the model cannot call them at all.
3. **No single tool call changes DNS.** `plan_*` tools only build a preview.
   Only `confirm_change` makes a write call, and it needs a token from a
   previous plan.
4. **Single-use, short-lived tokens.** Tokens are random UUIDv4 values, kept
   only in the server's memory, valid for 10 minutes by default, and removed
   when used. At most 100 plans can be outstanding at once.
5. **Human confirmation.** With elicitation support, the server itself asks the
   user to confirm, through a client dialog the model cannot answer. Without
   it, the confirmation is the client's own approval prompt for
   `confirm_change`, which is annotated `destructiveHint: true`.
6. **Safer updates.** `plan_update_record_set` starts from the current record
   set, so leaving a field out keeps its current value. In particular, it
   avoids a VinylDNS behavior where an update without `ownerGroupId` removes
   the record's owner group.

## Recommendations

- **Never auto-approve `confirm_change`** in your MCP client. Auto-approving
  read tools (`readOnlyHint: true`) is fine.
- Prefer `VINYLDNS_MCP_CONFIRMATION=elicit` when your client supports
  elicitation.
- Use a dedicated VinylDNS user with the smallest set of zone access it needs.
- Keep `VINYLDNS_SECRET_KEY` out of shared config files. Most MCP clients
  accept environment variables, or a wrapper script that reads the key from
  your OS keychain.
- Use `https://` for the API URL.

## What the server does not protect against

- A user who approves a change without reading the preview.
- A compromised MCP client: anything that controls the client can also answer
  its dialogs.
- Prompt injection that convinces the *user* to approve something. Previews
  show the exact request body so the user can check it.

## Secrets handling

- The secret key is only used locally to compute HMAC signatures. It is never
  sent over the network.
- `Config` and `Signer` redact the secret in debug output. Logs go to stderr
  and never include credentials.

## Reporting vulnerabilities

Please report security issues privately to the repository owner rather than in
a public issue.
