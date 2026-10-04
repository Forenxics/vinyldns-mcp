# Tool reference

All tools return JSON text. Failures (validation errors, VinylDNS 4xx/5xx
responses, transport errors) come back as **tool errors**: the model sees the
VinylDNS message, plus a hint for common status codes, so it can correct
itself. Parameters use `snake_case`. IDs are VinylDNS UUIDs.

Each tool carries MCP annotations (`readOnlyHint`, `destructiveHint`,
`openWorldHint`), so clients can treat read and write tools differently, for
example by auto-approving reads only.

## Read tools (always available)

| Tool | Parameters | VinylDNS endpoint |
|---|---|---|
| `check_connection` | – | `GET /status` plus an authenticated probe (`GET /groups?maxItems=1`) |
| `list_zones` | `name_filter?`, `start_from?`, `max_items?`, `search_by_admin_group?`, `ignore_access?`, `include_reverse?` | `GET /zones` |
| `get_zone` | `zone_id` **or** `zone_name` | `GET /zones/{id}` or `GET /zones/name/{name}` |
| `list_zone_changes` | `zone_id`, `start_from?`, `max_items?` | `GET /zones/{id}/changes` |
| `list_record_sets` | `zone_id`, `name_filter?`, `type_filter?`, `owner_group_filter?`, `start_from?`, `max_items?`, `name_sort?` | `GET /zones/{id}/recordsets` |
| `search_record_sets` | `name_filter`, `type_filter?`, `owner_group_filter?`, `start_from?`, `max_items?` | `GET /recordsets` |
| `get_record_set` | `zone_id`, `record_set_id` | `GET /zones/{id}/recordsets/{rsId}` |
| `get_record_set_change` | `zone_id`, `record_set_id`, `change_id` | `GET /zones/{id}/recordsets/{rsId}/changes/{changeId}` |
| `list_record_set_changes` | `zone_id`, `start_from?`, `max_items?` | `GET /zones/{id}/recordsetchanges` |
| `list_batch_changes` | `approval_status?`, `user_name?`, `date_time_range_start?`, `date_time_range_end?`, `ignore_access?`, `start_from?`, `max_items?` | `GET /zones/batchrecordchanges` |
| `get_batch_change` | `id` | `GET /zones/batchrecordchanges/{id}` |
| `list_groups` | `name_filter?`, `ignore_access?`, `start_from?`, `max_items?` | `GET /groups` |
| `get_group` | `id` | `GET /groups/{id}` |
| `list_group_members` | `group_id`, `start_from?`, `max_items?` | `GET /groups/{id}/members` |
| `get_user` | `user` (ID or user name) | `GET /users/{id}` |

Pagination: list responses include `nextId` when there are more results. Pass
it back as `start_from`.

## Write tools (only when `VINYLDNS_MCP_ENABLE_WRITES=true`)

`plan_*` tools never change anything. Each one returns:

```json
{
  "status": "pending_confirmation",
  "token": "1f0c…",
  "summary": "Create A www.example.com. (ttl 300) with 1 record(s)",
  "expires_in_seconds": 600,
  "preview": { "...": "request body, current state, diff, warnings" },
  "next_step": "Show this preview to the user. Only if they approve, call confirm_change…"
}
```

| Tool | Parameters | Checks done before returning a plan |
|---|---|---|
| `plan_create_record_set` | `zone_id`, `name`, `record_type`, `ttl`, `records[]`, `owner_group_id?` | Record type supported (SOA rejected); TTL ≥ 30; the fields each record type needs; one record per CNAME; zone exists; warns if a record set with the same name and type already exists, or if a shared zone has no owner group |
| `plan_update_record_set` | `zone_id`, `record_set_id`, `ttl?`, `records?`, `owner_group_id?` | Fetches the current record set; fields you omit, **including `ownerGroupId`**, are carried over; refuses no-op updates; returns `before`/`after` |
| `plan_delete_record_set` | `zone_id`, `record_set_id` | Fetches the record set and shows it in full under `will_delete` |
| `plan_batch_change` | `changes[]` (`change_type`: `Add` or `DeleteRecordSet`, `input_name`, `record_type`, `ttl?`, `record?`), `comments?`, `owner_group_id?`, `scheduled_time?`, `allow_manual_review?` (default `true`) | Each `Add` has a valid `record`; TTLs; renders a readable change list. VinylDNS does the full validation when the batch is submitted. |
| `plan_cancel_batch_change` | `id` | Batch change exists and is in `PendingReview` |
| `confirm_change` | `token` | See below |
| `list_pending_changes` | – | Lists plans that have not expired |
| `discard_pending_change` | `token` | – |

### `confirm_change`

1. Looks up the plan. An unknown or expired token is a tool error.
2. Unless `VINYLDNS_MCP_CONFIRMATION=token`, sends an **elicitation** request to
   the client showing the summary and preview:
   - the user confirms → continue;
   - the user declines → the plan is discarded and nothing is applied;
   - the user dismisses the dialog → nothing is applied and the plan stays pending;
   - the client has no elicitation support → `auto` continues (relying on the client's
     tool approval prompt), while `elicit` refuses.
3. Removes the token (each token can be used once) and makes the signed API call.
4. Returns the VinylDNS response and how to follow it up. Record set changes are
   asynchronous: poll `get_record_set_change` until the status is `Complete` or
   `Failed`. Right after submission it can return 404 for a few seconds while
   the change is queued.

### Record data formats

| Type | Fields |
|---|---|
| A, AAAA | `address` |
| CNAME | `cname` |
| PTR | `ptrdname` |
| NS | `nsdname` (non-apex, approved name servers only) |
| TXT, SPF | `text` |
| MX | `preference`, `exchange` |
| SRV | `priority`, `weight`, `port`, `target` |
| NAPTR | `order`, `preference`, `flags`, `service`, `regexp`, `replacement` |
| DS | `keytag`, `algorithm`, `digesttype`, `digest` |
| SSHFP | `algorithm`, `type`, `fingerprint` |

Hostnames should be fully qualified, with a trailing dot (`target.example.com.`).
