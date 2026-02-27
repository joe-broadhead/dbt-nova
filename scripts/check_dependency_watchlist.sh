#!/usr/bin/env bash
set -euo pipefail

WATCHLIST_FILE="${1:-dependency-watchlist.toml}"

if [[ ! -f "$WATCHLIST_FILE" ]]; then
  echo "dependency watchlist file not found: $WATCHLIST_FILE" >&2
  exit 1
fi

python3 - "$WATCHLIST_FILE" <<'PY'
import datetime as dt
import pathlib
import sys
import tomllib

watchlist_path = pathlib.Path(sys.argv[1])
root = watchlist_path.parent

try:
    data = tomllib.loads(watchlist_path.read_text(encoding="utf-8"))
except (OSError, tomllib.TOMLDecodeError) as exc:
    print(f"failed to parse {watchlist_path}: {exc}", file=sys.stderr)
    raise SystemExit(1)

items = data.get("item", [])
if not items:
    print(f"no watchlist items found in {watchlist_path}")
    raise SystemExit(0)

today = dt.date.today()
cargo_toml_path = root / "Cargo.toml"
cargo_lock_path = root / "Cargo.lock"

if not cargo_toml_path.exists():
    print(f"missing Cargo.toml at {cargo_toml_path}", file=sys.stderr)
    raise SystemExit(1)
if not cargo_lock_path.exists():
    print(f"missing Cargo.lock at {cargo_lock_path}", file=sys.stderr)
    raise SystemExit(1)

cargo_toml = cargo_toml_path.read_text(encoding="utf-8")
try:
    cargo_lock = tomllib.loads(cargo_lock_path.read_text(encoding="utf-8"))
except tomllib.TOMLDecodeError as exc:
    print(f"failed to parse {cargo_lock_path}: {exc}", file=sys.stderr)
    raise SystemExit(1)

packages = cargo_lock.get("package", [])

def has_package_version(name: str, predicate) -> bool:
    for package in packages:
        if package.get("name") == name:
            version = package.get("version", "")
            if predicate(version):
                return True
    return False

def cargo_toml_has_ort_sys_rc_pin() -> bool:
    return 'ort-sys = "=2.0.0-rc.4"' in cargo_toml

def cargo_lock_has_ort_sys_rc4() -> bool:
    return has_package_version("ort-sys", lambda version: version == "2.0.0-rc.4")

def cargo_lock_has_reqwest_011() -> bool:
    return has_package_version("reqwest", lambda version: version.startswith("0.11."))

def cargo_lock_has_reqwest_012() -> bool:
    return has_package_version("reqwest", lambda version: version.startswith("0.12."))

check_functions = {
    "cargo_toml_has_ort_sys_rc_pin": cargo_toml_has_ort_sys_rc_pin,
    "cargo_lock_has_ort_sys_rc4": cargo_lock_has_ort_sys_rc4,
    "cargo_lock_has_reqwest_011": cargo_lock_has_reqwest_011,
    "cargo_lock_has_reqwest_012": cargo_lock_has_reqwest_012,
}

required_fields = {
    "id",
    "owner",
    "review_by",
    "summary",
    "current_state",
    "upgrade_trigger",
    "upgrade_plan",
    "state_checks",
}

failed = False

for item in items:
    item_id = item.get("id", "<unknown>")
    missing = sorted(field for field in required_fields if not item.get(field))
    if missing:
        print(f"{item_id}: missing required fields: {', '.join(missing)}", file=sys.stderr)
        failed = True
        continue

    try:
        review_by = dt.date.fromisoformat(item["review_by"])
    except ValueError:
        print(f"{item_id}: invalid review_by date: {item['review_by']}", file=sys.stderr)
        failed = True
        continue

    if review_by < today:
        print(
            f"{item_id}: review_by expired on {review_by.isoformat()} (today: {today.isoformat()})",
            file=sys.stderr,
        )
        failed = True

    for check_name in item.get("state_checks", []):
        checker = check_functions.get(check_name)
        if checker is None:
            print(f"{item_id}: unknown state check '{check_name}'", file=sys.stderr)
            failed = True
            continue
        if not checker():
            print(
                f"{item_id}: state check failed: {check_name}. Update dependencies or refresh {watchlist_path.name}.",
                file=sys.stderr,
            )
            failed = True

if failed:
    print("dependency watchlist validation failed", file=sys.stderr)
    raise SystemExit(1)

print(
    f"dependency watchlist validation passed ({len(items)} entries, review date >= {today.isoformat()})"
)
PY
