//! Zone management and batch review tools.
//!
//! The plan tools here are registered only when `VINYLDNS_MCP_ENABLE_ADMIN=true`
//! (see [`super::ADMIN_TOOLS`]); `list_deleted_zones` and `list_backend_ids`
//! are read-only and always available. Every write follows the same plan →
//! `confirm_change` flow as the record tools.

use rmcp::{
    handler::server::wrapper::Parameters,
    schemars::{self, JsonSchema},
    tool, tool_router,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{SUPPORTED_TYPES, ToolResult, VinylDnsServer, api_error, flag, num, ok_json, tool_error};
use crate::client::{ApiError, segment};
use crate::pending::PlannedAction;

const REDACTED: &str = "<redacted>";

/// Zone fields that `PUT /zones/{id}` replaces wholesale. Anything missing from
/// the request is cleared, so updates must carry all of them over.
const ZONE_UPDATE_FIELDS: &[&str] = &[
    "id",
    "name",
    "email",
    "connection",
    "transferConnection",
    "shared",
    "acl",
    "adminGroupId",
    "backendId",
    "recurrenceSchedule",
    "scheduleRequestor",
];

/// Zone fields `plan_update_zone` can change (shown in the before/after diff).
const ZONE_EDITABLE_FIELDS: &[&str] = &["email", "adminGroupId", "shared", "backendId", "recurrenceSchedule"];

// ---------------------------------------------------------------------------
// Parameter types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListDeletedZonesParams {
    /// Only deleted zones whose name contains this text (supports `*` wildcard).
    #[serde(default)]
    pub name_filter: Option<String>,
    /// Include zones the user did not own (support/super users).
    #[serde(default)]
    pub ignore_access: Option<bool>,
    /// Pagination cursor: the `nextId` from a previous response.
    #[serde(default)]
    pub start_from: Option<String>,
    /// Page size, 1-100.
    #[serde(default)]
    pub max_items: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReviewBatchChangeParams {
    /// Batch change ID.
    pub id: String,
    /// Optional explanation stored with the review decision.
    #[serde(default)]
    pub review_comment: Option<String>,
}

/// TSIG algorithm for a zone connection key.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub enum TsigAlgorithm {
    #[serde(rename = "HMAC-MD5")]
    HmacMd5,
    #[serde(rename = "HMAC-SHA1")]
    HmacSha1,
    #[serde(rename = "HMAC-SHA224")]
    HmacSha224,
    #[serde(rename = "HMAC-SHA256")]
    HmacSha256,
    #[serde(rename = "HMAC-SHA384")]
    HmacSha384,
    #[serde(rename = "HMAC-SHA512")]
    HmacSha512,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ZoneConnectionParams {
    /// Name of the TSIG key on the DNS server.
    pub key_name: String,
    /// TSIG secret. VinylDNS encrypts it on receipt; it is redacted from all previews.
    pub key: String,
    /// DNS server address, optionally with a port, e.g. `10.0.0.53` or `ns1.example.com:5300`.
    pub primary_server: String,
    /// TSIG algorithm. Server default: HMAC-MD5.
    #[serde(default)]
    pub algorithm: Option<TsigAlgorithm>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ConnectZoneParams {
    /// Zone name, e.g. `example.com.` (a trailing dot is added if missing).
    pub name: String,
    /// Contact email for the zone.
    pub email: String,
    /// Group ID (UUID) that will administer the zone.
    pub admin_group_id: String,
    /// Configured backend to use (see `list_backend_ids`). Preferred over explicit connections.
    #[serde(default)]
    pub backend_id: Option<String>,
    /// Make the zone shared (only VinylDNS super users may set this).
    #[serde(default)]
    pub shared: Option<bool>,
    /// Explicit DDNS update connection. Omit to use `backend_id` or the server's default keys.
    #[serde(default)]
    pub connection: Option<ZoneConnectionParams>,
    /// Explicit zone transfer (AXFR) connection used for syncing.
    #[serde(default)]
    pub transfer_connection: Option<ZoneConnectionParams>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateZoneParams {
    /// Zone ID (UUID).
    pub zone_id: String,
    /// New contact email.
    #[serde(default)]
    pub email: Option<String>,
    /// New admin group ID (you must also be able to administer with the new group).
    #[serde(default)]
    pub admin_group_id: Option<String>,
    /// Change the shared flag (super users only).
    #[serde(default)]
    pub shared: Option<bool>,
    /// Switch to another configured backend (see `list_backend_ids`).
    #[serde(default)]
    pub backend_id: Option<String>,
    /// Cron expression for scheduled syncs (support/super users only); empty string clears it.
    #[serde(default)]
    pub recurrence_schedule: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ZoneIdParam {
    /// Zone ID (UUID).
    pub zone_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeleteZoneParams {
    /// Zone ID (UUID).
    pub zone_id: String,
    /// The zone's name typed out again (e.g. `example.com.`), as a guard against deleting the wrong zone.
    pub confirm_zone_name: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub enum AccessLevel {
    /// Cannot see matching records.
    NoAccess,
    /// Can view matching records.
    Read,
    /// Can create and update, but not delete, matching records.
    Write,
    /// Full access to matching records.
    Delete,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AclRuleParams {
    /// Zone ID (UUID).
    pub zone_id: String,
    /// Access granted by the rule.
    pub access_level: AccessLevel,
    /// User ID the rule applies to. Set at most one of `user_id` / `group_id`; neither means all users.
    #[serde(default)]
    pub user_id: Option<String>,
    /// Group ID the rule applies to.
    #[serde(default)]
    pub group_id: Option<String>,
    /// Regular expression matched against record names; omit for all records.
    #[serde(default)]
    pub record_mask: Option<String>,
    /// Record types the rule covers, e.g. `["A", "CNAME"]`; omit for all types.
    #[serde(default)]
    pub record_types: Option<Vec<String>>,
    /// Free-text description. When deleting, it only needs to be given to tell otherwise identical rules apart.
    #[serde(default)]
    pub description: Option<String>,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Zone names are absolute: ensure exactly one trailing dot.
fn normalize_zone_name(name: &str) -> String {
    format!("{}.", name.trim().trim_end_matches('.'))
}

fn same_zone_name(a: &str, b: &str) -> bool {
    normalize_zone_name(a).eq_ignore_ascii_case(&normalize_zone_name(b))
}

/// Replaces TSIG secrets with a placeholder so they never reach previews.
fn redact_connections(zone: &Value) -> Value {
    let mut z = zone.clone();
    for field in ["connection", "transferConnection"] {
        if let Some(conn) = z.get_mut(field).and_then(Value::as_object_mut)
            && conn.contains_key("key")
        {
            conn.insert("key".into(), json!(REDACTED));
        }
    }
    z
}

fn connection_json(zone_name: &str, c: &ZoneConnectionParams) -> Value {
    let mut conn = json!({
        "name": zone_name,
        "keyName": c.key_name.trim(),
        "key": c.key,
        "primaryServer": c.primary_server.trim(),
    });
    if let Some(alg) = c.algorithm {
        conn["algorithm"] = json!(alg);
    }
    conn
}

fn validate_email(email: &str) -> Result<(), String> {
    let (local, domain) = email.split_once('@').ok_or("email must contain '@'")?;
    if local.is_empty() || !domain.contains('.') {
        return Err(format!("'{email}' does not look like an email address"));
    }
    Ok(())
}

/// Builds the ACL rule body VinylDNS expects, validating it on the way.
fn acl_rule_json(p: &AclRuleParams) -> Result<Value, String> {
    if p.user_id.is_some() && p.group_id.is_some() {
        return Err("set at most one of user_id and group_id".into());
    }
    let mut types = Vec::new();
    for t in p.record_types.iter().flatten() {
        let upper = t.trim().to_ascii_uppercase();
        if upper != "SOA" && !SUPPORTED_TYPES.contains(&upper.as_str()) {
            return Err(format!("unsupported record type '{t}' in record_types"));
        }
        if !types.contains(&upper) {
            types.push(upper);
        }
    }
    types.sort();
    let mut rule = Map::new();
    rule.insert("accessLevel".into(), json!(p.access_level));
    rule.insert("recordTypes".into(), json!(types));
    for (key, value) in [
        ("userId", &p.user_id),
        ("groupId", &p.group_id),
        ("recordMask", &p.record_mask),
        ("description", &p.description),
    ] {
        if let Some(v) = value.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
            rule.insert(key.into(), json!(v));
        }
    }
    Ok(Value::Object(rule))
}

/// The fields that identify an ACL rule, normalized for comparison.
/// `description` is compared only when `with_description` is set.
fn rule_key(rule: &Value, with_description: bool) -> Value {
    let field = |k: &str| rule.get(k).filter(|v| !v.is_null() && v.as_str() != Some("")).cloned();
    let mut types: Vec<String> = rule
        .get("recordTypes")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_ascii_uppercase)
                .collect()
        })
        .unwrap_or_default();
    types.sort();
    json!({
        "accessLevel": field("accessLevel"),
        "userId": field("userId"),
        "groupId": field("groupId"),
        "recordMask": field("recordMask"),
        "recordTypes": types,
        "description": if with_description { field("description") } else { None },
    })
}

/// Rules from a zone response, as stored (without the display-only `displayName`).
fn zone_rules(zone: &Value) -> Vec<Value> {
    zone.pointer("/acl/rules")
        .and_then(Value::as_array)
        .map(|rules| {
            rules
                .iter()
                .map(|r| {
                    let mut r = r.clone();
                    if let Some(o) = r.as_object_mut() {
                        o.remove("displayName");
                    }
                    r
                })
                .collect()
        })
        .unwrap_or_default()
}

fn describe_rule(rule: &Value) -> String {
    let who = rule
        .get("userId")
        .and_then(Value::as_str)
        .map(|u| format!("user {u}"))
        .or_else(|| {
            rule.get("groupId")
                .and_then(Value::as_str)
                .map(|g| format!("group {g}"))
        })
        .unwrap_or_else(|| "all users".into());
    let types = rule
        .get("recordTypes")
        .and_then(Value::as_array)
        .filter(|a| !a.is_empty())
        .map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(","))
        .unwrap_or_else(|| "all types".into());
    let mask = rule.get("recordMask").and_then(Value::as_str).unwrap_or("all records");
    format!(
        "{} for {who} on {types} matching {mask}",
        rule.get("accessLevel").and_then(Value::as_str).unwrap_or("?")
    )
}

/// Compact view of a batch change for review previews.
fn batch_review_preview(batch: &Value) -> Value {
    let changes: Vec<Value> = batch
        .get("changes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|c| {
            json!({
                "changeType": c.get("changeType"),
                "inputName": c.get("inputName"),
                "type": c.get("type"),
                "ttl": c.get("ttl"),
                "record": c.get("record"),
                "status": c.get("status"),
                "validationErrors": c.get("validationErrors"),
            })
        })
        .collect();
    json!({
        "id": batch.get("id"),
        "submitted_by": batch.get("userName"),
        "created": batch.get("createdTimestamp"),
        "comments": batch.get("comments"),
        "ownerGroupName": batch.get("ownerGroupName"),
        "scheduledTime": batch.get("scheduledTime"),
        "changes": changes,
    })
}

impl VinylDnsServer {
    /// Fetches a zone (`{"zone": {...}}` → the inner object).
    async fn fetch_zone(&self, zone_id: &str) -> Result<Value, ApiError> {
        let v = self.client.get(&format!("zones/{}", segment(zone_id)), &[]).await?;
        Ok(v.get("zone").cloned().unwrap_or(v))
    }

    /// Returns an error message if `backend_id` is not configured on the server.
    async fn check_backend_id(&self, backend_id: &str) -> Result<Option<String>, ApiError> {
        let ids = self.client.get("zones/backendids", &[]).await?;
        let known: Vec<&str> = ids.as_array().into_iter().flatten().filter_map(Value::as_str).collect();
        Ok((!known.contains(&backend_id))
            .then(|| format!("unknown backend_id '{backend_id}'; configured backends: {known:?}")))
    }

    /// Fetches a batch change and checks it is waiting for review.
    async fn pending_review_batch(&self, id: &str) -> Result<Result<Value, String>, ApiError> {
        let batch = self
            .client
            .get(&format!("zones/batchrecordchanges/{}", segment(id)), &[])
            .await?;
        let approval = batch.get("approvalStatus").and_then(Value::as_str).unwrap_or_default();
        if approval != "PendingReview" {
            return Ok(Err(format!(
                "only batch changes in PendingReview can be reviewed; this one is '{approval}'"
            )));
        }
        Ok(Ok(batch))
    }

    fn review_plan(&self, approve: bool, p: ReviewBatchChangeParams, batch: Value) -> ToolResult {
        let mut body = json!({});
        if let Some(c) = p.review_comment.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
            body["reviewComment"] = json!(c);
        }
        let n = batch.get("changes").and_then(Value::as_array).map_or(0, Vec::len);
        let who = batch.get("userName").and_then(Value::as_str).unwrap_or("?");
        let mut warnings = Vec::new();
        if approve && batch.get("scheduledTime").is_some_and(|t| !t.is_null()) {
            warnings.push(
                "This batch change is scheduled; VinylDNS refuses approval (403) before its scheduled time."
                    .to_string(),
            );
        }
        let (summary, action) = if approve {
            (
                format!(
                    "APPROVE batch change {} from {who} ({n} change(s)); it will be applied to DNS",
                    p.id
                ),
                PlannedAction::ApproveBatchChange {
                    batch_change_id: p.id,
                    body: body.clone(),
                },
            )
        } else {
            (
                format!(
                    "REJECT batch change {} from {who} ({n} change(s)); nothing will be applied",
                    p.id
                ),
                PlannedAction::RejectBatchChange {
                    batch_change_id: p.id,
                    body: body.clone(),
                },
            )
        };
        self.store_plan(
            action,
            summary,
            json!({ "batch_change": batch_review_preview(&batch), "request": body, "warnings": warnings }),
        )
    }

    /// Executes zone and review actions for [`VinylDnsServer::apply`].
    pub(super) async fn apply_admin(&self, action: &PlannedAction) -> (Result<Value, ApiError>, &'static str) {
        const ZONE_FOLLOW_UP: &str =
            "Zone changes are processed asynchronously: use get_zone (status) and list_zone_changes to follow it.";
        match action {
            PlannedAction::ApproveBatchChange { batch_change_id, body } => (
                self.client
                    .post(
                        &format!("zones/batchrecordchanges/{}/approve", segment(batch_change_id)),
                        &[],
                        Some(body),
                    )
                    .await,
                "Approved: the changes are being applied. Use get_batch_change to follow progress.",
            ),
            PlannedAction::RejectBatchChange { batch_change_id, body } => (
                self.client
                    .post(
                        &format!("zones/batchrecordchanges/{}/reject", segment(batch_change_id)),
                        &[],
                        Some(body),
                    )
                    .await,
                "Rejected: no changes from this batch will be applied.",
            ),
            PlannedAction::ConnectZone { body } => (self.client.post("zones", &[], Some(body)).await, ZONE_FOLLOW_UP),
            PlannedAction::UpdateZone { zone_id, body } => (
                self.client.put(&format!("zones/{}", segment(zone_id)), body).await,
                ZONE_FOLLOW_UP,
            ),
            PlannedAction::SyncZone { zone_id } => (
                self.client
                    .post(&format!("zones/{}/sync", segment(zone_id)), &[], None)
                    .await,
                "Sync started; the zone shows status Syncing until it finishes. Use get_zone to check.",
            ),
            PlannedAction::DeleteZone { zone_id } => (
                self.client.delete(&format!("zones/{}", segment(zone_id)), None).await,
                "The zone is being disconnected from VinylDNS; its records remain on the DNS server.",
            ),
            PlannedAction::AddZoneAclRule { zone_id, body } => (
                self.client
                    .put(&format!("zones/{}/acl/rules", segment(zone_id)), body)
                    .await,
                ZONE_FOLLOW_UP,
            ),
            PlannedAction::DeleteZoneAclRule { zone_id, body } => (
                self.client
                    .delete(&format!("zones/{}/acl/rules", segment(zone_id)), Some(body))
                    .await,
                ZONE_FOLLOW_UP,
            ),
            other => unreachable!("{} is applied by VinylDnsServer::apply", other.kind()),
        }
    }
}

#[tool_router(router = admin_router, vis = "pub(super)")]
impl VinylDnsServer {
    // ----- read tools (always available) ----------------------------------

    #[tool(
        description = "List zones that were deleted (abandoned) in VinylDNS, with who deleted them and when.",
        annotations(title = "List deleted zones", read_only_hint = true, open_world_hint = true)
    )]
    async fn list_deleted_zones(&self, Parameters(p): Parameters<ListDeletedZonesParams>) -> ToolResult {
        ok_json(api!(
            self.client
                .get(
                    "zones/deleted/changes",
                    &[
                        ("nameFilter", p.name_filter),
                        ("ignoreAccess", flag(p.ignore_access)),
                        ("startFrom", p.start_from),
                        ("maxItems", num(p.max_items)),
                    ],
                )
                .await
        ))
    }

    #[tool(
        description = "List the DNS backend IDs configured on the VinylDNS server (usable as backend_id when connecting or updating a zone).",
        annotations(title = "List backend IDs", read_only_hint = true, open_world_hint = true)
    )]
    async fn list_backend_ids(&self) -> ToolResult {
        ok_json(json!({ "backend_ids": api!(self.client.get("zones/backendids", &[]).await) }))
    }

    // ----- batch review (support/super users) -----------------------------

    #[tool(
        description = "Plan APPROVING a batch change that is waiting for manual review (requires a VinylDNS support or super user). \
                       Shows every change in the batch and returns a confirmation token; nothing happens until confirm_change. \
                       VinylDNS re-validates the batch on approval.",
        annotations(
            title = "Plan: approve batch change",
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = true
        )
    )]
    async fn plan_approve_batch_change(&self, Parameters(p): Parameters<ReviewBatchChangeParams>) -> ToolResult {
        match api!(self.pending_review_batch(&p.id).await) {
            Ok(batch) => self.review_plan(true, p, batch),
            Err(msg) => tool_error(msg),
        }
    }

    #[tool(
        description = "Plan REJECTING a batch change that is waiting for manual review (requires a VinylDNS support or super user). \
                       Returns a confirmation token; nothing happens until confirm_change.",
        annotations(
            title = "Plan: reject batch change",
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = true
        )
    )]
    async fn plan_reject_batch_change(&self, Parameters(p): Parameters<ReviewBatchChangeParams>) -> ToolResult {
        match api!(self.pending_review_batch(&p.id).await) {
            Ok(batch) => self.review_plan(false, p, batch),
            Err(msg) => tool_error(msg),
        }
    }

    // ----- zone management -------------------------------------------------

    #[tool(
        description = "Plan connecting an existing DNS zone to VinylDNS so it can be managed here. Checks the zone is not already \
                       connected, the admin group exists and the backend ID is valid. Prefer backend_id over explicit TSIG \
                       connections; secrets are redacted from previews. Nothing happens until confirm_change.",
        annotations(
            title = "Plan: connect zone",
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = true
        )
    )]
    async fn plan_connect_zone(&self, Parameters(p): Parameters<ConnectZoneParams>) -> ToolResult {
        let name = normalize_zone_name(&p.name);
        if name == "." {
            return tool_error("zone name is empty");
        }
        if let Err(e) = validate_email(p.email.trim()) {
            return tool_error(e);
        }
        match self.client.get(&format!("zones/name/{}", segment(&name)), &[]).await {
            Ok(existing) => {
                let id = existing.pointer("/zone/id").and_then(Value::as_str).unwrap_or("?");
                return tool_error(format!("zone {name} is already connected to VinylDNS (id {id})"));
            }
            Err(e) if e.status().map(|s| s.as_u16()) == Some(404) => {}
            Err(e) => return api_error(e),
        }
        let group = api!(
            self.client
                .get(&format!("groups/{}", segment(&p.admin_group_id)), &[])
                .await
        );
        if let Some(backend) = &p.backend_id
            && let Some(msg) = api!(self.check_backend_id(backend).await)
        {
            return tool_error(msg);
        }

        let mut warnings = Vec::new();
        if p.backend_id.is_some() && (p.connection.is_some() || p.transfer_connection.is_some()) {
            warnings.push(
                "Both backend_id and explicit connections are set; VinylDNS uses the explicit connections.".to_string(),
            );
        }
        if p.shared == Some(true) {
            warnings.push("Only VinylDNS super users can create shared zones; others get 403.".to_string());
        }
        if p.backend_id.is_none() && p.connection.is_none() {
            warnings.push("No backend_id or connection given: VinylDNS will use its default keys.".to_string());
        }

        let mut body = json!({
            "name": name,
            "email": p.email.trim(),
            "adminGroupId": p.admin_group_id,
        });
        if let Some(b) = &p.backend_id {
            body["backendId"] = json!(b);
        }
        if let Some(s) = p.shared {
            body["shared"] = json!(s);
        }
        if let Some(c) = &p.connection {
            body["connection"] = connection_json(&name, c);
        }
        if let Some(c) = &p.transfer_connection {
            body["transferConnection"] = connection_json(&name, c);
        }

        let summary = format!(
            "Connect zone {name}, administered by group {}",
            group.get("name").and_then(Value::as_str).unwrap_or(&p.admin_group_id)
        );
        self.store_plan(
            PlannedAction::ConnectZone { body: body.clone() },
            summary,
            json!({ "request": redact_connections(&body), "warnings": warnings }),
        )
    }

    #[tool(
        description = "Plan updating a zone's email, admin group, shared flag, backend or sync schedule. Fetches the zone and carries \
                       over everything you do not change (connections, ACL rules, ...) because VinylDNS clears omitted fields. \
                       Returns a before/after diff; nothing happens until confirm_change.",
        annotations(
            title = "Plan: update zone",
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = true
        )
    )]
    async fn plan_update_zone(&self, Parameters(p): Parameters<UpdateZoneParams>) -> ToolResult {
        if p.email.is_none()
            && p.admin_group_id.is_none()
            && p.shared.is_none()
            && p.backend_id.is_none()
            && p.recurrence_schedule.is_none()
        {
            return tool_error(
                "nothing to change: provide email, admin_group_id, shared, backend_id and/or recurrence_schedule",
            );
        }
        let zone = api!(self.fetch_zone(&p.zone_id).await);

        let mut body = Map::new();
        for field in ZONE_UPDATE_FIELDS {
            if let Some(v) = zone.get(*field).filter(|v| !v.is_null()) {
                body.insert((*field).into(), v.clone());
            }
        }
        // `acl` comes back with display-only names; send the stored rule shape.
        body.insert("acl".into(), json!({ "rules": zone_rules(&zone) }));

        let mut warnings = Vec::new();
        if let Some(email) = &p.email {
            if let Err(e) = validate_email(email.trim()) {
                return tool_error(e);
            }
            body.insert("email".into(), json!(email.trim()));
        }
        if let Some(group_id) = &p.admin_group_id {
            api!(self.client.get(&format!("groups/{}", segment(group_id)), &[]).await);
            body.insert("adminGroupId".into(), json!(group_id));
        }
        if let Some(shared) = p.shared {
            warnings.push("Changing the shared flag requires a VinylDNS super user.".to_string());
            body.insert("shared".into(), json!(shared));
        }
        if let Some(backend) = &p.backend_id {
            if let Some(msg) = api!(self.check_backend_id(backend).await) {
                return tool_error(msg);
            }
            body.insert("backendId".into(), json!(backend));
        }
        if let Some(schedule) = &p.recurrence_schedule {
            warnings.push("Scheduled syncs can only be set by support or super users.".to_string());
            if schedule.trim().is_empty() {
                body.remove("recurrenceSchedule");
            } else {
                body.insert("recurrenceSchedule".into(), json!(schedule.trim()));
            }
        }

        let pick = |m: &Map<String, Value>| -> Value {
            Value::Object(
                ZONE_EDITABLE_FIELDS
                    .iter()
                    .map(|f| ((*f).to_string(), m.get(*f).cloned().unwrap_or(Value::Null)))
                    .collect(),
            )
        };
        let before = pick(zone.as_object().unwrap_or(&Map::new()));
        let after = pick(&body);
        if before == after {
            return tool_error("the requested values are identical to the current zone settings; nothing to do");
        }
        let changed: Vec<&str> = ZONE_EDITABLE_FIELDS
            .iter()
            .copied()
            .filter(|f| before[*f] != after[*f])
            .collect();

        let body = Value::Object(body);
        let summary = format!(
            "Update zone {} ({})",
            zone.get("name").and_then(Value::as_str).unwrap_or("?"),
            changed.join(", ")
        );
        self.store_plan(
            PlannedAction::UpdateZone {
                zone_id: p.zone_id,
                body: body.clone(),
            },
            summary,
            json!({ "before": before, "after": after, "request": redact_connections(&body), "warnings": warnings }),
        )
    }

    #[tool(
        description = "Plan syncing a zone: VinylDNS re-reads all records from the DNS server (zone transfer) and records any \
                       differences. The zone is unavailable for changes while syncing. Nothing happens until confirm_change.",
        annotations(
            title = "Plan: sync zone",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = true
        )
    )]
    async fn plan_sync_zone(&self, Parameters(p): Parameters<ZoneIdParam>) -> ToolResult {
        let zone = api!(self.fetch_zone(&p.zone_id).await);
        let status = zone.get("status").and_then(Value::as_str).unwrap_or("?");
        let mut warnings = Vec::new();
        if status != "Active" {
            warnings.push(format!(
                "The zone is '{status}', not Active; VinylDNS may refuse the sync."
            ));
        }
        let summary = format!("Sync zone {}", zone.get("name").and_then(Value::as_str).unwrap_or("?"));
        self.store_plan(
            PlannedAction::SyncZone { zone_id: p.zone_id },
            summary,
            json!({
                "zone": zone.get("name"),
                "status": status,
                "latestSync": zone.get("latestSync"),
                "warnings": warnings,
            }),
        )
    }

    #[tool(
        description = "Plan deleting (abandoning) a zone: VinylDNS stops managing it, while its records stay on the DNS server. \
                       Requires confirm_zone_name to match the zone's name. Shows the record set count. Nothing happens until confirm_change.",
        annotations(
            title = "Plan: delete zone",
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = true
        )
    )]
    async fn plan_delete_zone(&self, Parameters(p): Parameters<DeleteZoneParams>) -> ToolResult {
        let zone = api!(self.fetch_zone(&p.zone_id).await);
        let name = zone.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
        if !same_zone_name(&name, &p.confirm_zone_name) {
            return tool_error(format!(
                "confirm_zone_name '{}' does not match zone {} ('{name}'); nothing was planned",
                p.confirm_zone_name, p.zone_id
            ));
        }
        let count = self
            .client
            .get(&format!("zones/{}/recordsetcount", segment(&p.zone_id)), &[])
            .await
            .ok()
            .and_then(|v| v.get("count").cloned());
        self.store_plan(
            PlannedAction::DeleteZone { zone_id: p.zone_id },
            format!("DELETE (abandon) zone {name}"),
            json!({
                "zone": name,
                "adminGroupName": zone.get("adminGroupName"),
                "record_set_count": count,
                "effect": "VinylDNS stops managing this zone and its record sets. The records remain on the DNS server. \
                           Reconnecting later needs plan_connect_zone; VinylDNS ACL rules on the zone are lost.",
            }),
        )
    }

    #[tool(
        description = "Plan adding an ACL rule to a zone, granting a user, a group, or everyone access to matching records. \
                       Nothing happens until confirm_change.",
        annotations(
            title = "Plan: add zone ACL rule",
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = true
        )
    )]
    async fn plan_add_zone_acl_rule(&self, Parameters(p): Parameters<AclRuleParams>) -> ToolResult {
        let rule = match acl_rule_json(&p) {
            Ok(r) => r,
            Err(e) => return tool_error(e),
        };
        let zone = api!(self.fetch_zone(&p.zone_id).await);
        if zone_rules(&zone)
            .iter()
            .any(|r| rule_key(r, true) == rule_key(&rule, true))
        {
            return tool_error("an identical ACL rule already exists on this zone");
        }
        let mut warnings = Vec::new();
        if p.user_id.is_none() && p.group_id.is_none() {
            warnings.push("No user_id or group_id: this rule applies to ALL VinylDNS users.".to_string());
        }
        let summary = format!(
            "Add ACL rule to zone {}: {}",
            zone.get("name").and_then(Value::as_str).unwrap_or("?"),
            describe_rule(&rule)
        );
        self.store_plan(
            PlannedAction::AddZoneAclRule {
                zone_id: p.zone_id,
                body: rule.clone(),
            },
            summary,
            json!({ "rule": rule, "existing_rules": zone_rules(&zone).len(), "warnings": warnings }),
        )
    }

    #[tool(
        description = "Plan removing an ACL rule from a zone. The rule is looked up among the zone's current rules (description only \
                       needs to be given if two rules differ only by it). Nothing happens until confirm_change.",
        annotations(
            title = "Plan: delete zone ACL rule",
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = true
        )
    )]
    async fn plan_delete_zone_acl_rule(&self, Parameters(p): Parameters<AclRuleParams>) -> ToolResult {
        let wanted = match acl_rule_json(&p) {
            Ok(r) => r,
            Err(e) => return tool_error(e),
        };
        let zone = api!(self.fetch_zone(&p.zone_id).await);
        let rules = zone_rules(&zone);
        let with_description = p.description.is_some();
        let matches: Vec<&Value> = rules
            .iter()
            .filter(|r| rule_key(r, with_description) == rule_key(&wanted, with_description))
            .collect();
        let rule = match matches.as_slice() {
            [one] => (*one).clone(),
            [] => {
                return tool_error(format!(
                    "no matching ACL rule on this zone. Current rules:\n{}",
                    if rules.is_empty() {
                        "(none)".to_string()
                    } else {
                        rules
                            .iter()
                            .map(|r| format!("- {}", describe_rule(r)))
                            .collect::<Vec<_>>()
                            .join("\n")
                    }
                ));
            }
            _ => return tool_error("several rules match; add `description` to choose one"),
        };
        let summary = format!(
            "Remove ACL rule from zone {}: {}",
            zone.get("name").and_then(Value::as_str).unwrap_or("?"),
            describe_rule(&rule)
        );
        self.store_plan(
            PlannedAction::DeleteZoneAclRule {
                zone_id: p.zone_id,
                body: rule.clone(),
            },
            summary,
            json!({ "rule": rule }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn acl(access: AccessLevel) -> AclRuleParams {
        AclRuleParams {
            zone_id: "z".into(),
            access_level: access,
            user_id: None,
            group_id: None,
            record_mask: None,
            record_types: None,
            description: None,
        }
    }

    #[test]
    fn zone_names_are_normalized() {
        assert_eq!(normalize_zone_name(" example.com "), "example.com.");
        assert_eq!(normalize_zone_name("example.com.."), "example.com.");
        assert!(same_zone_name("Example.com", "example.com."));
        assert!(!same_zone_name("example.com.", "example.org."));
    }

    #[test]
    fn connection_keys_are_redacted() {
        let zone = json!({ "connection": { "key": "secret", "keyName": "k" }, "transferConnection": null });
        let redacted = redact_connections(&zone);
        assert_eq!(redacted["connection"]["key"], REDACTED);
        assert_eq!(redacted["connection"]["keyName"], "k");
        assert!(!redacted.to_string().contains("secret"));
    }

    #[test]
    fn acl_rules_are_validated_and_normalized() {
        let mut p = acl(AccessLevel::Write);
        p.record_types = Some(vec!["a".into(), "CNAME".into(), "A".into()]);
        p.group_id = Some("g1".into());
        let rule = acl_rule_json(&p).unwrap();
        assert_eq!(
            rule,
            json!({ "accessLevel": "Write", "groupId": "g1", "recordTypes": ["A", "CNAME"] })
        );

        p.user_id = Some("u1".into());
        assert!(acl_rule_json(&p).unwrap_err().contains("at most one"));

        let mut bad = acl(AccessLevel::Read);
        bad.record_types = Some(vec!["BOGUS".into()]);
        assert!(acl_rule_json(&bad).is_err());
    }

    #[test]
    fn rule_matching_ignores_order_display_name_and_optional_description() {
        let stored =
            json!({ "accessLevel": "Read", "recordTypes": ["CNAME", "A"], "description": "ops", "displayName": "x" });
        let wanted = json!({ "accessLevel": "Read", "recordTypes": ["A", "CNAME"] });
        assert_eq!(rule_key(&stored, false), rule_key(&wanted, false));
        assert_ne!(rule_key(&stored, true), rule_key(&wanted, true));
        assert_eq!(
            describe_rule(&wanted),
            "Read for all users on A,CNAME matching all records"
        );
    }

    #[test]
    fn email_validation() {
        assert!(validate_email("dns@example.com").is_ok());
        assert!(validate_email("nope").is_err());
        assert!(validate_email("a@b").is_err());
    }
}
