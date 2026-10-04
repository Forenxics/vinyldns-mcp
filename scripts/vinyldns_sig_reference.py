#!/usr/bin/env python3
"""Independent reference implementation of the VinylDNS server's signature check.

This is a line-by-line port of `Aws4Authenticator.scala` (vinyldns/vinyldns,
modules/api/src/main/scala/vinyldns/api/route/Aws4Authenticator.scala). It is
used to generate the signature fixtures in tests/fixtures/ so the Rust signer
is verified against the server's algorithm rather than against itself.

Usage: vinyldns_sig_reference.py METHOD URL BODY AMZ_DATE ACCESS_KEY SECRET [REGION] [SERVICE]
Prints the hex signature.
"""
import hashlib
import hmac
import sys
from urllib.parse import parse_qsl, quote, unquote, urlsplit


def encode(s: str) -> str:
    # java.net.URLEncoder + VinylDNS's fix-ups == RFC 3986 unreserved set kept.
    return quote(s, safe="-_.~")


def canonical_request(method, url, amz_date, body):
    parts = urlsplit(url)
    host = parts.hostname + (f":{parts.port}" if parts.port else "")
    path = unquote(parts.path) or "/"
    params = sorted((encode(k), encode(v)) for k, v in parse_qsl(parts.query, keep_blank_values=True))
    query = "&".join(f"{k}={v}" for k, v in params)
    lines = [method, path, query, f"host:{host}", f"x-amz-date:{amz_date}", "", "host;x-amz-date",
             hashlib.sha256(body.encode()).hexdigest()]
    return "\n".join(lines)


def signature(method, url, body, amz_date, secret, region="us-east-1", service="VinylDNS"):
    scope = f"{amz_date[:8]}/{region}/{service}/aws4_request"
    creq = canonical_request(method, url, amz_date, body)
    sts = "\n".join(["AWS4-HMAC-SHA256", amz_date, scope, hashlib.sha256(creq.encode()).hexdigest()])
    key = ("AWS4" + secret).encode()
    for part in scope.split("/"):
        key = hmac.new(key, part.encode(), hashlib.sha256).digest()
    return hmac.new(key, sts.encode(), hashlib.sha256).hexdigest()


if __name__ == "__main__":
    a = sys.argv[1:]
    print(signature(a[0], a[1], a[2], a[3], a[5], *a[6:8]))
