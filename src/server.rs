//! MCP server: tool definitions and the plan → confirm write workflow.

use std::sync::Arc;

use rmcp::{
    ErrorData as McpError, Peer, RoleServer, ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig},
    schemars::{self, JsonSchema},
    service::ElicitationError,
    tool, tool_handler, tool_router,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::client::{ApiError, VinylDnsClient, segment};
use crate::config::{Config, ConfirmationMode};
use crate::pending::{PendingChange, PendingStore, PlannedAction};

/// Record types VinylDNS can manage through the record set API.
const SUPPORTED_TYPES: &[&str] = &[
    "A", "AAAA", "CNAME", "DS", "MX", "NAPTR", "NS", "PTR", "SPF", "SRV", "SSHFP", "TXT",
];
const MIN_TTL: i64 = 30;

/// Names of tools that change state. They are not registered in read-only mode.
pub const WRITE_TOOLS: &[&str] = &[
    "plan_create_record_set",
    "plan_update_record_set",
    "plan_delete_record_set",
    "plan_batch_change",
    "plan_cancel_batch_change",
    "list_pending_changes",
    "discard_pending_change",
    "confirm_change",
];

// ---------------------------------------------------------------------------
// Tool parameter types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListZonesParams {
    /// Only zones whose name contains this text (supports `*` wildcard).
    #[serde(default)]
    pub name_filter: Option<String>,
    /// Pagination cursor: the `nextId` from a previous response.
    #[serde(default)]
    pub start_from: Option<String>,
    /// Page size, 1-100 (server default 100).
    #[serde(default)]
    pub max_items: Option<u32>,
    /// Treat `name_filter` as an admin group name instead of a zone name.
    #[serde(default)]
    pub search_by_admin_group: Option<bool>,
    /// List all zones, not just those the user can access (only metadata is shown for others).
    #[serde(default)]
    pub ignore_access: Option<bool>,
    /// Include reverse (in-addr.arpa / ip6.arpa) zones. Default true.
    #[serde(default)]
    pub include_reverse: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetZoneParams {
    /// Zone ID (UUID). Provide this or `zone_name`.
    #[serde(default)]
    pub zone_id: Option<String>,
    /// Zone name, e.g. `example.com.`. Provide this or `zone_id`.
    #[serde(default)]
    pub zone_name: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ZonePageParams {
    /// Zone ID (UUID).
    pub zone_id: String,
    /// Pagination cursor from the previous response (`nextId`).
    #[serde(default)]
    pub start_from: Option<String>,
    /// Page size, 1-100.
    #[serde(default)]
    pub max_items: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListRecordSetsParams {
    /// Zone ID (UUID).
    pub zone_id: String,
    /// Record name filter (supports `*` wildcard).
    #[serde(default)]
    pub name_filter: Option<String>,
    /// Comma-separated record types, e.g. `A,AAAA`.
    #[serde(default)]
    pub type_filter: Option<String>,
    /// Only records owned by this group ID (shared zones).
    #[serde(default)]
    pub owner_group_filter: Option<String>,
    /// Pagination cursor from the previous response (`nextId`).
    #[serde(default)]
    pub start_from: Option<String>,
    /// Page size, 1-100.
    #[serde(default)]
    pub max_items: Option<u32>,
    /// `ASC` (default) or `DESC`.
    #[serde(default)]
    pub name_sort: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchRecordSetsParams {
    /// Record name filter across all zones, e.g. `www*` or `api.example.com.`. At least two
    /// non-wildcard characters are required by VinylDNS.
    pub name_filter: String,
    /// Comma-separated record types, e.g. `A,CNAME`.
    #[serde(default)]
    pub type_filter: Option<String>,
    /// Only records owned by this group ID.
    #[serde(default)]
    pub owner_group_filter: Option<String>,
    /// Pagination cursor from the previous response (`nextId`).
    #[serde(default)]
    pub start_from: Option<String>,
    /// Page size, 1-100.
    #[serde(default)]
    pub max_items: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RecordSetRef {
    /// Zone ID (UUID).
    pub zone_id: String,
    /// Record set ID (UUID).
    pub record_set_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RecordSetChangeRef {
    /// Zone ID (UUID).
    pub zone_id: String,
    /// Record set ID (UUID).
    pub record_set_id: String,
    /// Change ID returned when the change was submitted.
    pub change_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListBatchChangesParams {
    /// Filter by approval status: AutoApproved, PendingReview, ManuallyApproved, Rejected, Cancelled.
    #[serde(default)]
    pub approval_status: Option<String>,
    /// Only batch changes submitted by this user name.
    #[serde(default)]
    pub user_name: Option<String>,
    /// ISO-8601 lower bound on creation time.
    #[serde(default)]
    pub date_time_range_start: Option<String>,
    /// ISO-8601 upper bound on creation time.
    #[serde(default)]
    pub date_time_range_end: Option<String>,
    /// Include other users' batch changes (support/super users only).
    #[serde(default)]
    pub ignore_access: Option<bool>,
    /// Pagination offset from the previous response (`nextId`).
    #[serde(default)]
    pub start_from: Option<u32>,
    /// Page size, 1-100.
    #[serde(default)]
    pub max_items: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct IdParam {
    /// Resource ID.
    pub id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListGroupsParams {
    /// Only groups whose name contains this text.
    #[serde(default)]
    pub name_filter: Option<String>,
    /// List all groups, not just the user's.
    #[serde(default)]
    pub ignore_access: Option<bool>,
    /// Pagination cursor from the previous response (`nextId`).
    #[serde(default)]
    pub start_from: Option<String>,
    /// Page size, 1-100.
    #[serde(default)]
    pub max_items: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GroupPageParams {
    /// Group ID (UUID).
    pub group_id: String,
    /// Pagination cursor from the previous response (`nextId`).
    #[serde(default)]
    pub start_from: Option<String>,
    /// Page size, 1-100.
    #[serde(default)]
    pub max_items: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetUserParams {
    /// User ID or user name.
    pub user: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PlanCreateRecordSetParams {
    /// Zone ID (UUID) to create the record set in.
    pub zone_id: String,
    /// Record name relative to the zone (e.g. `www`), or `@`/the zone name for the apex.
    pub name: String,
    /// Record type, e.g. `A`, `CNAME`, `TXT`.
    pub record_type: String,
    /// TTL in seconds (minimum 30).
    pub ttl: i64,
    #[schemars(
        description = "Record data objects; all must match the record type. A/AAAA {address}; CNAME {cname}; PTR {ptrdname}; NS {nsdname}; TXT/SPF {text}; MX {preference, exchange}; SRV {priority, weight, port, target}; NAPTR {order, preference, flags, service, regexp, replacement}; DS {keytag, algorithm, digesttype, digest}; SSHFP {algorithm, type, fingerprint}. Hostnames should be fully qualified with a trailing dot."
    )]
    pub records: Vec<Map<String, Value>>,
    /// Owner group ID; needed in shared zones.
    #[serde(default)]
    pub owner_group_id: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PlanUpdateRecordSetParams {
    /// Zone ID (UUID).
    pub zone_id: String,
    /// Record set ID (UUID) to update.
    pub record_set_id: String,
    /// New TTL in seconds. Omit to keep the current TTL.
    #[serde(default)]
    pub ttl: Option<i64>,
    /// Replacement record data (replaces ALL existing records in the set). Omit to keep current records.
    #[serde(default)]
    pub records: Option<Vec<Map<String, Value>>>,
    /// New owner group ID. Omit to keep the current owner group (it is preserved automatically).
    #[serde(default)]
    pub owner_group_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub enum BatchChangeType {
    /// Add a record (an Add plus a DeleteRecordSet of the same name/type acts as an update).
    Add,
    /// Delete a whole record set, or one record if `record` is given.
    DeleteRecordSet,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct BatchChangeItem {
    /// `Add` or `DeleteRecordSet`.
    pub change_type: BatchChangeType,
    /// Fully qualified name (e.g. `www.example.com.`); for PTR, the IP address.
    pub input_name: String,
    /// Record type: A, AAAA, CNAME, MX, PTR, TXT (others depend on server config).
    pub record_type: String,
    /// TTL in seconds (Add only, optional).
    #[serde(default)]
    pub ttl: Option<i64>,
    /// Record data (required for Add; optional for DeleteRecordSet to delete a single record).
    #[serde(default)]
    pub record: Option<Map<String, Value>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PlanBatchChangeParams {
    /// Changes to apply, possibly across zones. Deletes are applied before adds.
    pub changes: Vec<BatchChangeItem>,
    /// Free-text comment stored with the batch change.
    #[serde(default)]
    pub comments: Option<String>,
    /// Owner group ID; required when adding unowned records in shared zones.
    #[serde(default)]
    pub owner_group_id: Option<String>,
    /// ISO-8601 time to process the change (scheduled changes require manual review to be enabled).
    #[serde(default)]
    pub scheduled_time: Option<String>,
    /// Allow VinylDNS to route the batch to manual review instead of failing it. Default true.
    #[serde(default)]
    pub allow_manual_review: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TokenParam {
    /// Token returned by a `plan_*` tool.
    pub token: String,
}

/// Answer requested from the human through MCP elicitation.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ConfirmDecision {
    /// Set to true to apply this DNS change.
    pub confirm: bool,
}
rmcp::elicit_safe!(ConfirmDecision);

// ---------------------------------------------------------------------------
// Server
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct VinylDnsServer {
    client: VinylDnsClient,
    pending: Arc<PendingStore>,
    confirmation: ConfirmationMode,
    writes_enabled: bool,
    tool_router: ToolRouter<Self>,
}

type ToolResult = Result<CallToolResult, McpError>;

fn ok_json(value: Value) -> ToolResult {
    Ok(CallToolResult::success(vec![ContentBlock::json(value)?]))
}

/// Tool-level error: visible to the model so it can correct itself.
fn tool_error(message: impl Into<String>) -> ToolResult {
    Ok(CallToolResult::error(vec![ContentBlock::text(message.into())]))
}

fn api_error(err: ApiError) -> ToolResult {
    let mut msg = err.to_string();
    if let Some(hint) = err.hint() {
        msg.push_str("\nHint: ");
        msg.push_str(hint);
    }
    tool_error(msg)
}

/// Unwraps an API result or returns it as a tool error from the enclosing function.
macro_rules! api {
    ($e:expr) => {
        match $e {
            Ok(v) => v,
            Err(err) => return api_error(err),
        }
    };
}

fn num(v: Option<u32>) -> Option<String> {
    v.map(|n| n.to_string())
}

fn flag(v: Option<bool>) -> Option<String> {
    v.map(|b| b.to_string())
}

fn validate_records(record_type: &str, records: &[Map<String, Value>]) -> Result<(), String> {
    if records.is_empty() {
        return Err("at least one record is required".into());
    }
    let required: &[&str] = match record_type {
        "A" | "AAAA" => &["address"],
        "CNAME" => &["cname"],
        "PTR" => &["ptrdname"],
        "NS" => &["nsdname"],
        "TXT" | "SPF" => &["text"],
        "MX" => &["preference", "exchange"],
        "SRV" => &["priority", "weight", "port", "target"],
        "NAPTR" => &["order", "preference", "flags", "service", "regexp", "replacement"],
        "DS" => &["keytag", "algorithm", "digesttype", "digest"],
        "SSHFP" => &["algorithm", "type", "fingerprint"],
        _ => &[],
    };
    for (i, r) in records.iter().enumerate() {
        let missing: Vec<_> = required.iter().filter(|f| !r.contains_key(**f)).collect();
        if !missing.is_empty() {
            return Err(format!(
                "record {i} for type {record_type} is missing field(s): {missing:?}"
            ));
        }
    }
    if record_type == "CNAME" && records.len() > 1 {
        return Err("a CNAME record set can contain only one record".into());
    }
    Ok(())
}

fn normalize_type(t: &str) -> Result<String, String> {
    let upper = t.trim().to_ascii_uppercase();
    if upper == "SOA" {
        return Err("SOA records are read-only in VinylDNS".into());
    }
    if SUPPORTED_TYPES.contains(&upper.as_str()) {
        Ok(upper)
    } else {
        Err(format!(
            "unsupported record type '{t}'; supported: {}",
            SUPPORTED_TYPES.join(", ")
        ))
    }
}

fn validate_ttl(ttl: i64) -> Result<(), String> {
    if ttl < MIN_TTL {
        Err(format!("ttl must be at least {MIN_TTL} seconds (got {ttl})"))
    } else {
        Ok(())
    }
}

/// Fully qualified name of a record within a zone, for previews.
fn fqdn(name: &str, zone_name: &str) -> String {
    let zone = if zone_name.ends_with('.') {
        zone_name.to_string()
    } else {
        format!("{zone_name}.")
    };
    if name == "@" || name == zone || format!("{name}.") == zone {
        zone
    } else if name.ends_with('.') {
        name.to_string()
    } else {
        format!("{name}.{zone}")
    }
}

#[tool_router]
impl VinylDnsServer {
    pub fn new(config: &Config, client: VinylDnsClient) -> Self {
        let mut tool_router = Self::tool_router();
        if !config.enable_writes {
            for name in WRITE_TOOLS {
                tool_router.remove_route(name);
            }
        }
        Self {
            client,
            pending: Arc::new(PendingStore::new(config.pending_ttl)),
            confirmation: config.confirmation,
            writes_enabled: config.enable_writes,
            tool_router,
        }
    }

    // ----- read tools ------------------------------------------------------

    #[tool(
        description = "Check connectivity to VinylDNS: server status (version, whether processing is disabled) and whether the configured credentials are accepted.",
        annotations(title = "Check VinylDNS connection", read_only_hint = true, open_world_hint = true)
    )]
    async fn check_connection(&self) -> ToolResult {
        let status = self.client.get("status", &[]).await;
        let auth = self.client.get("groups", &[("maxItems", Some("1".into()))]).await;
        ok_json(json!({
            "status": match &status { Ok(v) => v.clone(), Err(e) => json!({ "error": e.to_string() }) },
            "credentials_valid": auth.is_ok(),
            "credentials_error": auth.err().map(|e| e.to_string()),
            "writes_enabled": self.writes_enabled,
        }))
    }

    #[tool(
        description = "List DNS zones visible to the user. Returns zones and a `nextId` cursor for pagination.",
        annotations(title = "List zones", read_only_hint = true, open_world_hint = true)
    )]
    async fn list_zones(&self, Parameters(p): Parameters<ListZonesParams>) -> ToolResult {
        let v = api!(
            self.client
                .get(
                    "zones",
                    &[
                        ("nameFilter", p.name_filter),
                        ("startFrom", p.start_from),
                        ("maxItems", num(p.max_items)),
                        ("searchByAdminGroup", flag(p.search_by_admin_group)),
                        ("ignoreAccess", flag(p.ignore_access)),
                        ("includeReverse", flag(p.include_reverse)),
                    ],
                )
                .await
        );
        ok_json(v)
    }

    #[tool(
        description = "Get one zone by ID or by name (including its admin group, ACL rules, and whether it is shared).",
        annotations(title = "Get zone", read_only_hint = true, open_world_hint = true)
    )]
    async fn get_zone(&self, Parameters(p): Parameters<GetZoneParams>) -> ToolResult {
        let path = match (p.zone_id, p.zone_name) {
            (Some(id), _) => format!("zones/{}", segment(&id)),
            (None, Some(name)) => format!("zones/name/{}", segment(&name)),
            (None, None) => return tool_error("provide zone_id or zone_name"),
        };
        ok_json(api!(self.client.get(&path, &[]).await))
    }

    #[tool(
        description = "List the change history of a zone's settings (create/update/sync/delete of the zone itself).",
        annotations(title = "List zone changes", read_only_hint = true, open_world_hint = true)
    )]
    async fn list_zone_changes(&self, Parameters(p): Parameters<ZonePageParams>) -> ToolResult {
        let path = format!("zones/{}/changes", segment(&p.zone_id));
        ok_json(api!(
            self.client
                .get(&path, &[("startFrom", p.start_from), ("maxItems", num(p.max_items))])
                .await
        ))
    }

    #[tool(
        description = "List record sets in a zone, optionally filtered by name, type, or owner group.",
        annotations(title = "List record sets", read_only_hint = true, open_world_hint = true)
    )]
    async fn list_record_sets(&self, Parameters(p): Parameters<ListRecordSetsParams>) -> ToolResult {
        let path = format!("zones/{}/recordsets", segment(&p.zone_id));
        ok_json(api!(
            self.client
                .get(
                    &path,
                    &[
                        ("recordNameFilter", p.name_filter),
                        ("recordTypeFilter", p.type_filter),
                        ("recordOwnerGroupFilter", p.owner_group_filter),
                        ("startFrom", p.start_from),
                        ("maxItems", num(p.max_items)),
                        ("nameSort", p.name_sort),
                    ],
                )
                .await
        ))
    }

    #[tool(
        description = "Search record sets by name across all zones the user can see (e.g. to find which zone holds a name).",
        annotations(title = "Search record sets", read_only_hint = true, open_world_hint = true)
    )]
    async fn search_record_sets(&self, Parameters(p): Parameters<SearchRecordSetsParams>) -> ToolResult {
        ok_json(api!(
            self.client
                .get(
                    "recordsets",
                    &[
                        ("recordNameFilter", Some(p.name_filter)),
                        ("recordTypeFilter", p.type_filter),
                        ("recordOwnerGroupFilter", p.owner_group_filter),
                        ("startFrom", p.start_from),
                        ("maxItems", num(p.max_items)),
                    ],
                )
                .await
        ))
    }

    #[tool(
        description = "Get one record set by zone ID and record set ID.",
        annotations(title = "Get record set", read_only_hint = true, open_world_hint = true)
    )]
    async fn get_record_set(&self, Parameters(p): Parameters<RecordSetRef>) -> ToolResult {
        let path = format!("zones/{}/recordsets/{}", segment(&p.zone_id), segment(&p.record_set_id));
        ok_json(api!(self.client.get(&path, &[]).await))
    }

    #[tool(
        description = "Get the status of a single record set change (Pending, Complete, Failed) — use after confirming a record set change. \
                       A 404 shortly after submission means the change is still queued; retry after a few seconds.",
        annotations(title = "Get record set change", read_only_hint = true, open_world_hint = true)
    )]
    async fn get_record_set_change(&self, Parameters(p): Parameters<RecordSetChangeRef>) -> ToolResult {
        let path = format!(
            "zones/{}/recordsets/{}/changes/{}",
            segment(&p.zone_id),
            segment(&p.record_set_id),
            segment(&p.change_id)
        );
        ok_json(api!(self.client.get(&path, &[]).await))
    }

    #[tool(
        description = "List recent record set changes in a zone (an audit trail: who changed what, when, and the outcome).",
        annotations(title = "List record set changes", read_only_hint = true, open_world_hint = true)
    )]
    async fn list_record_set_changes(&self, Parameters(p): Parameters<ZonePageParams>) -> ToolResult {
        let path = format!("zones/{}/recordsetchanges", segment(&p.zone_id));
        ok_json(api!(
            self.client
                .get(&path, &[("startFrom", p.start_from), ("maxItems", num(p.max_items))])
                .await
        ))
    }

    #[tool(
        description = "List batch change summaries (newest first), optionally filtered by approval status, user, or date range.",
        annotations(title = "List batch changes", read_only_hint = true, open_world_hint = true)
    )]
    async fn list_batch_changes(&self, Parameters(p): Parameters<ListBatchChangesParams>) -> ToolResult {
        ok_json(api!(
            self.client
                .get(
                    "zones/batchrecordchanges",
                    &[
                        ("approvalStatus", p.approval_status),
                        ("userName", p.user_name),
                        ("dateTimeRangeStart", p.date_time_range_start),
                        ("dateTimeRangeEnd", p.date_time_range_end),
                        ("ignoreAccess", flag(p.ignore_access)),
                        ("startFrom", num(p.start_from)),
                        ("maxItems", num(p.max_items)),
                    ],
                )
                .await
        ))
    }

    #[tool(
        description = "Get a batch change with the status and any validation errors of each single change.",
        annotations(title = "Get batch change", read_only_hint = true, open_world_hint = true)
    )]
    async fn get_batch_change(&self, Parameters(p): Parameters<IdParam>) -> ToolResult {
        let path = format!("zones/batchrecordchanges/{}", segment(&p.id));
        ok_json(api!(self.client.get(&path, &[]).await))
    }

    #[tool(
        description = "List groups the user belongs to (or all groups with ignore_access).",
        annotations(title = "List groups", read_only_hint = true, open_world_hint = true)
    )]
    async fn list_groups(&self, Parameters(p): Parameters<ListGroupsParams>) -> ToolResult {
        ok_json(api!(
            self.client
                .get(
                    "groups",
                    &[
                        ("groupNameFilter", p.name_filter),
                        ("ignoreAccess", flag(p.ignore_access)),
                        ("startFrom", p.start_from),
                        ("maxItems", num(p.max_items)),
                    ],
                )
                .await
        ))
    }

    #[tool(
        description = "Get a group by ID, including its members and admins.",
        annotations(title = "Get group", read_only_hint = true, open_world_hint = true)
    )]
    async fn get_group(&self, Parameters(p): Parameters<IdParam>) -> ToolResult {
        let path = format!("groups/{}", segment(&p.id));
        ok_json(api!(self.client.get(&path, &[]).await))
    }

    #[tool(
        description = "List the members of a group.",
        annotations(title = "List group members", read_only_hint = true, open_world_hint = true)
    )]
    async fn list_group_members(&self, Parameters(p): Parameters<GroupPageParams>) -> ToolResult {
        let path = format!("groups/{}/members", segment(&p.group_id));
        ok_json(api!(
            self.client
                .get(&path, &[("startFrom", p.start_from), ("maxItems", num(p.max_items))])
                .await
        ))
    }

    #[tool(
        description = "Look up a user by ID or user name (returns ID, user name, and group IDs).",
        annotations(title = "Get user", read_only_hint = true, open_world_hint = true)
    )]
    async fn get_user(&self, Parameters(p): Parameters<GetUserParams>) -> ToolResult {
        let path = format!("users/{}", segment(&p.user));
        ok_json(api!(self.client.get(&path, &[]).await))
    }

    // ----- write planning tools -------------------------------------------

    #[tool(
        description = "Plan creating a record set. Validates input and checks for an existing record set with the same name and type, \
                       then returns a preview and a confirmation token. NOTHING is changed until `confirm_change` is called with the token. \
                       Show the preview to the user before confirming.",
        annotations(
            title = "Plan: create record set",
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = true
        )
    )]
    async fn plan_create_record_set(&self, Parameters(p): Parameters<PlanCreateRecordSetParams>) -> ToolResult {
        let record_type = match normalize_type(&p.record_type) {
            Ok(t) => t,
            Err(e) => return tool_error(e),
        };
        if let Err(e) = validate_ttl(p.ttl).and_then(|_| validate_records(&record_type, &p.records)) {
            return tool_error(e);
        }

        let zone = api!(self.client.get(&format!("zones/{}", segment(&p.zone_id)), &[]).await);
        let zone_name = zone
            .pointer("/zone/name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();

        let mut warnings = Vec::new();
        let existing = api!(
            self.client
                .get(
                    &format!("zones/{}/recordsets", segment(&p.zone_id)),
                    &[
                        ("recordNameFilter", Some(p.name.clone())),
                        ("recordTypeFilter", Some(record_type.clone()))
                    ],
                )
                .await
        );
        let conflict = existing
            .get("recordSets")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find(|rs| rs.get("name").and_then(Value::as_str) == Some(p.name.as_str()));
        if let Some(rs) = conflict {
            warnings.push(format!(
                "A {record_type} record set named '{}' already exists (id {}); creating will fail with 409. Use plan_update_record_set instead.",
                p.name,
                rs.get("id").and_then(Value::as_str).unwrap_or("?")
            ));
        }
        if zone.pointer("/zone/shared").and_then(Value::as_bool) == Some(true) && p.owner_group_id.is_none() {
            warnings.push("This is a shared zone; consider setting owner_group_id so the record has an owner.".into());
        }

        let mut body = json!({
            "zoneId": p.zone_id,
            "name": p.name,
            "type": record_type,
            "ttl": p.ttl,
            "records": p.records,
        });
        if let Some(g) = &p.owner_group_id {
            body["ownerGroupId"] = json!(g);
        }

        let summary = format!(
            "Create {record_type} {} (ttl {}) with {} record(s)",
            fqdn(&p.name, &zone_name),
            p.ttl,
            p.records.len()
        );
        self.store_plan(
            PlannedAction::CreateRecordSet {
                zone_id: p.zone_id,
                body: body.clone(),
            },
            summary,
            json!({ "zone": zone_name, "request": body, "warnings": warnings }),
        )
    }

    #[tool(
        description = "Plan updating a record set's TTL, records, and/or owner group. Fetches the current record set, preserves fields you \
                       do not change (including ownerGroupId), and returns a before/after diff and a confirmation token. \
                       NOTHING is changed until `confirm_change` is called.",
        annotations(
            title = "Plan: update record set",
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = true
        )
    )]
    async fn plan_update_record_set(&self, Parameters(p): Parameters<PlanUpdateRecordSetParams>) -> ToolResult {
        if p.ttl.is_none() && p.records.is_none() && p.owner_group_id.is_none() {
            return tool_error("nothing to change: provide ttl, records, and/or owner_group_id");
        }
        let path = format!("zones/{}/recordsets/{}", segment(&p.zone_id), segment(&p.record_set_id));
        let current = api!(self.client.get(&path, &[]).await);
        let Some(rs) = current.get("recordSet").cloned() else {
            return tool_error("unexpected response: no recordSet in VinylDNS reply");
        };
        let record_type = rs.get("type").and_then(Value::as_str).unwrap_or_default().to_string();
        if let Some(ttl) = p.ttl
            && let Err(e) = validate_ttl(ttl)
        {
            return tool_error(e);
        }
        if let Some(records) = &p.records
            && let Err(e) = validate_records(&record_type, records)
        {
            return tool_error(e);
        }

        let before = json!({
            "ttl": rs.get("ttl"),
            "records": rs.get("records"),
            "ownerGroupId": rs.get("ownerGroupId"),
        });
        let mut body = json!({
            "id": rs.get("id"),
            "zoneId": rs.get("zoneId"),
            "name": rs.get("name"),
            "type": rs.get("type"),
            "ttl": p.ttl.map(Value::from).or_else(|| rs.get("ttl").cloned()),
            "records": p.records.map(|r| json!(r)).or_else(|| rs.get("records").cloned()),
        });
        if let Some(owner) = p
            .owner_group_id
            .map(Value::from)
            .or_else(|| rs.get("ownerGroupId").cloned())
        {
            body["ownerGroupId"] = owner;
        }
        let after = json!({ "ttl": body["ttl"], "records": body["records"], "ownerGroupId": body.get("ownerGroupId") });
        if before == after {
            return tool_error("the requested values are identical to the current record set; nothing to do");
        }

        let summary = format!(
            "Update {record_type} {} ({})",
            rs.get("fqdn")
                .or_else(|| rs.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("?"),
            p.record_set_id
        );
        self.store_plan(
            PlannedAction::UpdateRecordSet {
                zone_id: p.zone_id,
                record_set_id: p.record_set_id,
                body: body.clone(),
            },
            summary,
            json!({ "before": before, "after": after, "request": body }),
        )
    }

    #[tool(
        description = "Plan deleting a record set. Fetches it so the user can see exactly what will be removed, and returns a \
                       confirmation token. NOTHING is deleted until `confirm_change` is called.",
        annotations(
            title = "Plan: delete record set",
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = true
        )
    )]
    async fn plan_delete_record_set(&self, Parameters(p): Parameters<RecordSetRef>) -> ToolResult {
        let path = format!("zones/{}/recordsets/{}", segment(&p.zone_id), segment(&p.record_set_id));
        let current = api!(self.client.get(&path, &[]).await);
        let rs = current.get("recordSet").cloned().unwrap_or(Value::Null);
        let summary = format!(
            "DELETE {} {} ({} record(s))",
            rs.get("type").and_then(Value::as_str).unwrap_or("?"),
            rs.get("fqdn")
                .or_else(|| rs.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("?"),
            rs.get("records").and_then(Value::as_array).map_or(0, Vec::len)
        );
        self.store_plan(
            PlannedAction::DeleteRecordSet {
                zone_id: p.zone_id,
                record_set_id: p.record_set_id,
            },
            summary,
            json!({ "will_delete": rs }),
        )
    }

    #[tool(
        description = "Plan a batch change: several record adds/deletes, possibly across zones, applied together. A DeleteRecordSet \
                       plus an Add of the same name and type acts as an update. Returns a preview and a confirmation token; \
                       NOTHING is submitted until `confirm_change`. VinylDNS validates the batch on submission and may route it \
                       to manual review.",
        annotations(
            title = "Plan: batch change",
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = true
        )
    )]
    async fn plan_batch_change(&self, Parameters(p): Parameters<PlanBatchChangeParams>) -> ToolResult {
        if p.changes.is_empty() {
            return tool_error("a batch change needs at least one change");
        }
        let mut changes = Vec::with_capacity(p.changes.len());
        let mut lines = Vec::with_capacity(p.changes.len());
        for (i, c) in p.changes.iter().enumerate() {
            let record_type = c.record_type.trim().to_ascii_uppercase();
            let mut change = json!({
                "changeType": c.change_type,
                "inputName": c.input_name,
                "type": record_type,
            });
            match c.change_type {
                BatchChangeType::Add => {
                    let Some(record) = &c.record else {
                        return tool_error(format!("change {i}: Add requires `record`"));
                    };
                    if let Err(e) = validate_records(&record_type, std::slice::from_ref(record)) {
                        return tool_error(format!("change {i}: {e}"));
                    }
                    if let Some(ttl) = c.ttl {
                        if let Err(e) = validate_ttl(ttl) {
                            return tool_error(format!("change {i}: {e}"));
                        }
                        change["ttl"] = json!(ttl);
                    }
                    change["record"] = json!(record);
                    lines.push(format!(
                        "ADD {record_type} {} -> {}",
                        c.input_name,
                        Value::Object(record.clone())
                    ));
                }
                BatchChangeType::DeleteRecordSet => {
                    if let Some(record) = &c.record {
                        change["record"] = json!(record);
                        lines.push(format!(
                            "DELETE {record_type} {} record {}",
                            c.input_name,
                            Value::Object(record.clone())
                        ));
                    } else {
                        lines.push(format!("DELETE {record_type} {} (entire record set)", c.input_name));
                    }
                }
            }
            changes.push(change);
        }

        let mut body = json!({ "changes": changes });
        if let Some(c) = &p.comments {
            body["comments"] = json!(c);
        }
        if let Some(g) = &p.owner_group_id {
            body["ownerGroupId"] = json!(g);
        }
        if let Some(t) = &p.scheduled_time {
            body["scheduledTime"] = json!(t);
        }
        let allow_manual_review = p.allow_manual_review.unwrap_or(true);
        let summary = format!("Submit batch change with {} change(s)", changes.len());
        self.store_plan(
            PlannedAction::SubmitBatchChange {
                body: body.clone(),
                allow_manual_review,
            },
            summary,
            json!({ "changes": lines, "allow_manual_review": allow_manual_review, "request": body }),
        )
    }

    #[tool(
        description = "Plan cancelling one of your own batch changes that is waiting for manual review. Returns a confirmation token.",
        annotations(
            title = "Plan: cancel batch change",
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = true
        )
    )]
    async fn plan_cancel_batch_change(&self, Parameters(p): Parameters<IdParam>) -> ToolResult {
        let batch = api!(
            self.client
                .get(&format!("zones/batchrecordchanges/{}", segment(&p.id)), &[])
                .await
        );
        let approval = batch.get("approvalStatus").and_then(Value::as_str).unwrap_or_default();
        if approval != "PendingReview" {
            return tool_error(format!(
                "only batch changes in PendingReview can be cancelled; this one is '{approval}'"
            ));
        }
        let summary = format!(
            "Cancel batch change {} ({} change(s), comments: {})",
            p.id,
            batch.get("changes").and_then(Value::as_array).map_or(0, Vec::len),
            batch.get("comments").and_then(Value::as_str).unwrap_or("-")
        );
        self.store_plan(
            PlannedAction::CancelBatchChange { batch_change_id: p.id },
            summary,
            json!({ "batch_change": batch }),
        )
    }

    // ----- pending change management --------------------------------------

    #[tool(
        description = "List planned changes that have not been confirmed or discarded yet.",
        annotations(title = "List pending changes", read_only_hint = true, open_world_hint = false)
    )]
    async fn list_pending_changes(&self) -> ToolResult {
        ok_json(json!({ "pending": self.pending.list() }))
    }

    #[tool(
        description = "Discard a planned change without applying it.",
        annotations(
            title = "Discard pending change",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn discard_pending_change(&self, Parameters(p): Parameters<TokenParam>) -> ToolResult {
        let found = self.pending.discard(&p.token);
        ok_json(json!({ "discarded": found }))
    }

    #[tool(
        description = "Apply a planned change to VinylDNS. Only call this after the user has seen the plan's preview and explicitly \
                       agreed. If the client supports it, the user is asked to confirm in a dialog; declining discards the plan.",
        annotations(
            title = "Confirm and apply change",
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = true
        )
    )]
    async fn confirm_change(&self, Parameters(p): Parameters<TokenParam>, peer: Peer<RoleServer>) -> ToolResult {
        let change = match self.pending.peek(&p.token) {
            Ok(c) => c,
            Err(e) => return tool_error(e.to_string()),
        };

        if self.confirmation != ConfirmationMode::Token {
            let message = format!(
                "VinylDNS change requested by the assistant:\n\n{}\n\nDetails:\n{}\n\nApply this change?",
                change.summary,
                serde_json::to_string_pretty(&change.preview).unwrap_or_default()
            );
            match peer.elicit::<ConfirmDecision>(message).await {
                Ok(Some(ConfirmDecision { confirm: true })) => {}
                Ok(_) | Err(ElicitationError::UserDeclined) => {
                    self.pending.discard(&p.token);
                    return tool_error("The user declined this change; it was discarded and nothing was applied.");
                }
                Err(ElicitationError::UserCancelled) => {
                    return tool_error(
                        "The user dismissed the confirmation; nothing was applied. The plan is still pending.",
                    );
                }
                Err(ElicitationError::CapabilityNotSupported) if self.confirmation == ConfirmationMode::Auto => {
                    tracing::info!("client lacks elicitation support; relying on tool-call approval");
                }
                Err(ElicitationError::CapabilityNotSupported) => {
                    return tool_error(
                        "This server requires interactive confirmation (VINYLDNS_MCP_CONFIRMATION=elicit), \
                         but the MCP client does not support elicitation. Nothing was applied.",
                    );
                }
                Err(e) => return tool_error(format!("confirmation failed: {e}; nothing was applied")),
            }
        }

        let change = match self.pending.take(&p.token) {
            Ok(c) => c,
            Err(e) => return tool_error(e.to_string()),
        };
        self.apply(change).await
    }
}

impl VinylDnsServer {
    fn store_plan(&self, action: PlannedAction, summary: String, preview: Value) -> ToolResult {
        match self.pending.insert(action, summary, preview) {
            Ok(change) => ok_json(json!({
                "status": "pending_confirmation",
                "token": change.token,
                "summary": change.summary,
                "expires_in_seconds": self.pending.ttl().as_secs(),
                "preview": change.preview,
                "next_step": "Show this preview to the user. Only if they approve, call confirm_change with the token; otherwise call discard_pending_change.",
            })),
            Err(e) => tool_error(e.to_string()),
        }
    }

    /// Executes a confirmed plan against the VinylDNS API.
    pub async fn apply(&self, change: PendingChange) -> ToolResult {
        tracing::info!(kind = change.action.kind(), summary = %change.summary, "applying confirmed change");
        let (result, follow_up) = match &change.action {
            PlannedAction::CreateRecordSet { zone_id, body } => (
                self.client
                    .post(&format!("zones/{}/recordsets", segment(zone_id)), &[], Some(body))
                    .await,
                "Processing is asynchronous: poll get_record_set_change with the zone ID, recordSet.id and change id until status is Complete or Failed (it can return 404 for a few seconds while the change is queued).",
            ),
            PlannedAction::UpdateRecordSet {
                zone_id,
                record_set_id,
                body,
            } => (
                self.client
                    .put(
                        &format!("zones/{}/recordsets/{}", segment(zone_id), segment(record_set_id)),
                        body,
                    )
                    .await,
                "Processing is asynchronous: poll get_record_set_change until status is Complete or Failed (it can return 404 for a few seconds while the change is queued).",
            ),
            PlannedAction::DeleteRecordSet { zone_id, record_set_id } => (
                self.client
                    .delete(&format!(
                        "zones/{}/recordsets/{}",
                        segment(zone_id),
                        segment(record_set_id)
                    ))
                    .await,
                "Processing is asynchronous: poll get_record_set_change until status is Complete or Failed (it can return 404 for a few seconds while the change is queued).",
            ),
            PlannedAction::SubmitBatchChange {
                body,
                allow_manual_review,
            } => (
                self.client
                    .post(
                        "zones/batchrecordchanges",
                        &[("allowManualReview", Some(allow_manual_review.to_string()))],
                        Some(body),
                    )
                    .await,
                "Use get_batch_change with the returned id to follow progress. Status PendingReview means an administrator must approve it.",
            ),
            PlannedAction::CancelBatchChange { batch_change_id } => (
                self.client
                    .post(
                        &format!("zones/batchrecordchanges/{}/cancel", segment(batch_change_id)),
                        &[],
                        None,
                    )
                    .await,
                "The batch change is cancelled and will not be applied.",
            ),
        };
        match result {
            Ok(v) => ok_json(json!({ "applied": change.summary, "result": v, "next_step": follow_up })),
            Err(e) => {
                tracing::warn!(error = %e, "VinylDNS rejected confirmed change");
                api_error(e)
            }
        }
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for VinylDnsServer {
    fn get_info(&self) -> ServerConfig {
        let mode = if self.writes_enabled {
            "Writes are ENABLED but always two-step: plan_* tools only return a preview and a token; \
             confirm_change applies it. Always show the preview to the user and get explicit approval before confirm_change."
        } else {
            "Read-only mode: no tools can change DNS."
        };
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(
                Implementation::new("vinyldns-mcp", env!("CARGO_PKG_VERSION"))
                    .with_title("VinylDNS")
                    .with_description("Manage DNS zones, records and batch changes in VinylDNS"),
            )
            .with_instructions(format!(
                "Tools for the VinylDNS DNS management API. Zone and record set IDs are UUIDs: find them with \
                 list_zones / get_zone (by name) and list_record_sets / search_record_sets. Record changes are \
                 processed asynchronously. {mode}"
            ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fqdn_handles_relative_absolute_and_apex() {
        assert_eq!(fqdn("www", "example.com."), "www.example.com.");
        assert_eq!(fqdn("www", "example.com"), "www.example.com.");
        assert_eq!(fqdn("@", "example.com."), "example.com.");
        assert_eq!(fqdn("example.com.", "example.com."), "example.com.");
        assert_eq!(fqdn("a.other.", "example.com."), "a.other.");
    }

    #[test]
    fn record_validation() {
        let rec = |pairs: &[(&str, Value)]| {
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.clone()))
                .collect::<Map<_, _>>()
        };
        assert!(validate_records("A", &[rec(&[("address", json!("10.0.0.1"))])]).is_ok());
        assert!(validate_records("A", &[]).is_err());
        assert!(
            validate_records("MX", &[rec(&[("exchange", json!("mx."))])])
                .unwrap_err()
                .contains("preference")
        );
        let cname = rec(&[("cname", json!("a."))]);
        assert!(validate_records("CNAME", &[cname.clone(), cname]).is_err());
    }

    #[test]
    fn type_and_ttl_validation() {
        assert_eq!(normalize_type(" aaaa ").unwrap(), "AAAA");
        assert!(normalize_type("SOA").unwrap_err().contains("read-only"));
        assert!(normalize_type("BOGUS").is_err());
        assert!(validate_ttl(29).is_err());
        assert!(validate_ttl(30).is_ok());
    }
}
