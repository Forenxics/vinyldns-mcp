#!/usr/bin/env python3
"""Live smoke test for the zone management and batch review tools.

Runs against the VinylDNS quickstart (see smoke_test.py). Uses:
  - the `ok` test user (zone admin) for zone connect/update/ACL/sync/delete,
    on the quickstart's `dummy.` zone;
  - the `support-user` test user to approve a batch change that the quickstart
    routes to manual review (names matching `needs-review.*`).

Usage: scripts/smoke_test_admin.py [path/to/vinyldns-mcp]
"""
import json
import os
import sys
import time

sys.path.insert(0, os.path.dirname(__file__))
import smoke_test as base  # noqa: E402

ZONE = "dummy."
ADMIN_ENV = {"VINYLDNS_MCP_ENABLE_ADMIN": "true"}


def call(mcp, tool, args=None):
    """Tool call that returns (ok, payload) instead of asserting."""
    r = mcp.request("tools/call", {"name": tool, "arguments": args or {}})
    text = r["content"][0]["text"]
    return (not r.get("isError")), (text if r.get("isError") else json.loads(text))


def wait_zone(mcp, zone_id, predicate, what, attempts=40):
    for _ in range(attempts):
        ok, z = call(mcp, "get_zone", {"zone_id": zone_id})
        if ok and predicate(z["zone"]):
            print(f"  ok  {what}")
            return z["zone"]
        time.sleep(1)
    raise AssertionError(f"timed out waiting for: {what}")


def plan_and_confirm(mcp, tool, args):
    plan = mcp.tool(tool, args)
    print(f"      plan: {plan['summary']}")
    return mcp.tool("confirm_change", {"token": plan["token"]})


def main():
    # Make sure the `ok.` zone exists (also created by smoke_test.py).
    ok_zone_id = base.ensure_zone()
    status, group = base.api("POST", "/groups", {
        "name": f"mcp-admin-smoke-{int(time.time())}", "email": "test@test.com",
        "members": [{"id": "ok"}], "admins": [{"id": "ok"}]})
    assert status == 200, (status, group)

    status, existing = base.api("GET", f"/zones/name/{ZONE}")
    if status == 200:  # leftover from an earlier run
        base.api("DELETE", f"/zones/{existing['zone']['id']}")
        time.sleep(5)

    mcp = base.Mcp(**ADMIN_ENV)
    try:
        tools = [t["name"] for t in mcp.request("tools/list", {})["tools"]]
        print(f"{len(tools)} tools registered")
        for t in ["plan_connect_zone", "plan_update_zone", "plan_delete_zone", "plan_approve_batch_change"]:
            assert t in tools, t
        assert mcp.tool("check_connection")["admin_enabled"] is True
        print(f"  backends: {mcp.tool('list_backend_ids')['backend_ids']}")

        # Connect
        applied = plan_and_confirm(mcp, "plan_connect_zone", {
            "name": ZONE.rstrip("."), "email": "test@test.com", "admin_group_id": group["id"]})
        zone_id = applied["result"]["zone"]["id"]
        wait_zone(mcp, zone_id, lambda z: z["status"] == "Active", "zone connected and Active")
        ok, err = call(mcp, "plan_connect_zone", {"name": ZONE, "email": "test@test.com", "admin_group_id": group["id"]})
        assert not ok and "already connected" in err, err
        print("  ok  connecting twice is refused")

        # ACL rule add
        plan_and_confirm(mcp, "plan_add_zone_acl_rule", {
            "zone_id": zone_id, "access_level": "Read", "group_id": group["id"],
            "record_types": ["A", "CNAME"], "description": "mcp smoke"})
        wait_zone(mcp, zone_id, lambda z: len(z["acl"]["rules"]) == 1, "ACL rule added")

        # Update email: ACL and connection must survive
        plan_and_confirm(mcp, "plan_update_zone", {"zone_id": zone_id, "email": "changed@test.com"})
        z = wait_zone(mcp, zone_id, lambda z: z["email"] == "changed@test.com", "zone email updated")
        assert len(z["acl"]["rules"]) == 1, z["acl"]
        print("  ok  ACL rule preserved across zone update")

        # The zone must still accept record changes after the update
        name = f"admin-smoke-{int(time.time())}"
        rs = plan_and_confirm(mcp, "plan_create_record_set", {
            "zone_id": zone_id, "name": name, "record_type": "A", "ttl": 300, "records": [{"address": "192.0.2.50"}]})
        base.wait_complete(mcp, zone_id, rs)

        # ACL rule delete (without description: found by matching)
        plan_and_confirm(mcp, "plan_delete_zone_acl_rule", {
            "zone_id": zone_id, "access_level": "Read", "group_id": group["id"], "record_types": ["cname", "a"]})
        wait_zone(mcp, zone_id, lambda z: len(z["acl"]["rules"]) == 0, "ACL rule removed")

        # Sync (VinylDNS may refuse if the zone synced recently)
        ok, res = call(mcp, "plan_sync_zone", {"zone_id": zone_id})
        assert ok, res
        ok, res = call(mcp, "confirm_change", {"token": res["token"]})
        print(f"  {'ok ' if ok else 'n/a'} sync: {'started' if ok else res.splitlines()[0]}")

        # Delete: wrong name refused, right name works
        ok, err = call(mcp, "plan_delete_zone", {"zone_id": zone_id, "confirm_zone_name": "wrong."})
        assert not ok and "does not match" in err, err
        wait_zone(mcp, zone_id, lambda z: z["status"] == "Active", "zone Active before delete")
        plan_and_confirm(mcp, "plan_delete_zone", {"zone_id": zone_id, "confirm_zone_name": ZONE})
        for _ in range(40):
            ok, _ = call(mcp, "get_zone", {"zone_id": zone_id})
            if not ok:
                break
            time.sleep(1)
        assert not ok, "zone still present after delete"
        print("  ok  zone deleted (abandoned)")
        deleted = mcp.tool("list_deleted_zones", {"name_filter": "dummy"})
        print(f"  ok  list_deleted_zones: {len(deleted.get('zonesDeletedInfo', []))} entry(ies)")

        # Batch change needing review, submitted by `ok`
        plan = mcp.tool("plan_batch_change", {"comments": "mcp admin smoke", "owner_group_id": group["id"], "changes": [
            {"change_type": "Add", "input_name": f"needs-review-{int(time.time())}.ok.", "record_type": "A",
             "ttl": 300, "record": {"address": "192.0.2.99"}}]})
        batch = mcp.tool("confirm_change", {"token": plan["token"]})["result"]
        print(f"  ok  batch {batch['id']} submitted: {batch['approvalStatus']}")
        assert batch["approvalStatus"] == "PendingReview", batch
        ok, err = call(mcp, "confirm_change", {"token": mcp.tool("plan_approve_batch_change", {"id": batch["id"]})["token"]})
        assert not ok and "403" in err, err
        print("  ok  regular user cannot approve (403)")
    finally:
        mcp.close()

    support = base.Mcp("supportUserAccessKey", "supportUserSecretKey", **ADMIN_ENV)
    try:
        plan_and_confirm(support, "plan_approve_batch_change", {"id": batch["id"], "review_comment": "approved by smoke test"})
        for _ in range(30):
            b = support.tool("get_batch_change", {"id": batch["id"]})
            if b["status"] not in ("PendingProcessing", "PendingReview"):
                break
            time.sleep(1)
        print(f"  ok  approved batch: approvalStatus={b['approvalStatus']} status={b['status']}")
        assert b["approvalStatus"] == "ManuallyApproved", b
    finally:
        support.close()
    print("ADMIN SMOKE TEST PASSED")


if __name__ == "__main__":
    if len(sys.argv) > 1:
        base.BINARY = sys.argv[1]
    main()
