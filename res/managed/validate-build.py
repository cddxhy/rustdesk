import base64
import os
import re
import subprocess
import tomllib
from pathlib import Path
from urllib.parse import urlsplit

BASE_COMMIT = "669cb8ac62aa20d3626395877d7dd0357aec7ed2"
VERSION = "1.5.3"


def required(name):
    value = os.environ.get(name, "")
    if not value or value != value.strip() or any(c.isspace() for c in value):
        raise SystemExit(f"{name} must be configured without whitespace")
    return value


def endpoint(name, value, *, https):
    try:
        parsed = urlsplit(value if https else f"//{value}")
        if (
            not parsed.hostname
            or (https and parsed.scheme != "https")
            or parsed.username is not None
            or parsed.password is not None
            or parsed.query
            or parsed.fragment
            or (not https and (parsed.port is None or parsed.path))
            or parsed.port == 0
            or any(char in value for char in "\\?#")
        ):
            raise ValueError()
    except ValueError:
        raise SystemExit(f"{name} must be an {'HTTPS base URL' if https else 'address with port'}") from None


def main():
    manifest = tomllib.loads(Path("Cargo.toml").read_text(encoding="utf-8"))
    if manifest["package"]["version"] != VERSION:
        raise SystemExit(f"Expected pinned RustDesk version {VERSION}")
    flutter_manifest = Path("flutter/pubspec.yaml").read_text(encoding="utf-8")
    if not re.search(rf"(?m)^version: {re.escape(VERSION)}(?:\+[0-9]+)?$", flutter_manifest):
        raise SystemExit(f"Expected pinned Flutter app version {VERSION}")
    subprocess.run(["git", "merge-base", "--is-ancestor", BASE_COMMIT, "HEAD"], check=True)
    for name in ("RUSTDESK_ID_SERVER", "RUSTDESK_RELAY_SERVER", "MANAGEMENT_API_URL"):
        endpoint(name, required(name), https=name == "MANAGEMENT_API_URL")
    public_keys = []
    for name in ("RUSTDESK_SERVER_PUBLIC_KEY", "MANAGEMENT_API_ENCRYPTION_PUBLIC_KEY"):
        try:
            decoded = base64.b64decode(required(name), validate=True)
            if len(decoded) != 32 or not any(decoded):
                raise ValueError()
            public_keys.append(decoded)
        except ValueError:
            raise SystemExit(f"{name} must encode a 32-byte public key in standard Base64") from None
    if public_keys[0] == public_keys[1]:
        raise SystemExit("Use separate RustDesk and management API public keys")
    if not re.fullmatch(r"[A-Za-z0-9._-]{1,64}", required("MANAGEMENT_API_ENCRYPTION_KEY_ID")):
        raise SystemExit("Invalid MANAGEMENT_API_ENCRYPTION_KEY_ID")


if __name__ == "__main__":
    main()
