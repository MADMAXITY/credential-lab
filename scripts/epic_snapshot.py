"""
Epic Games File Snapshot Tool

Takes a snapshot of ALL Epic-related files (sizes + hashes) and registry keys.
Run before and after login to diff what changed.

Usage:
  python epic_snapshot.py before    # Take "before" snapshot
  python epic_snapshot.py after     # Take "after" snapshot
  python epic_snapshot.py diff      # Show what changed between before and after
"""

import os
import sys
import json
import hashlib
import winreg
from datetime import datetime

SNAPSHOT_DIR = os.path.dirname(os.path.abspath(__file__))

def get_epic_paths():
    """All Epic-related directories to scan."""
    local = os.environ.get("LOCALAPPDATA", "")
    appdata = os.environ.get("APPDATA", "")
    progdata = os.environ.get("PROGRAMDATA", "")

    return [
        os.path.join(local, "EpicGamesLauncher"),
        os.path.join(local, "EADesktop"),  # bonus: EA too
        os.path.join(local, "Epic Games"),
        os.path.join(appdata, "Epic"),
        os.path.join(progdata, "Epic"),
    ]

def hash_file(path):
    try:
        with open(path, "rb") as f:
            return hashlib.md5(f.read()).hexdigest()
    except:
        return "ERROR"

def scan_files():
    """Scan all Epic files, return dict of path -> {size, hash, mtime}."""
    files = {}
    for base_dir in get_epic_paths():
        if not os.path.exists(base_dir):
            continue
        for root, dirs, filenames in os.walk(base_dir):
            # Skip huge cache dirs
            rel = os.path.relpath(root, base_dir)
            if any(skip in rel.lower() for skip in ["logs", "crashreport", "deriveddatacache", "httpcache", "gpucache"]):
                continue
            for fname in filenames:
                if fname.endswith(".log"):
                    continue
                path = os.path.join(root, fname)
                try:
                    stat = os.stat(path)
                    files[path] = {
                        "size": stat.st_size,
                        "mtime": stat.st_mtime,
                        "hash": hash_file(path),
                    }
                except:
                    pass
    return files

def scan_registry():
    """Scan Epic-related registry keys."""
    registry = {}
    keys_to_check = [
        (winreg.HKEY_CURRENT_USER, r"Software\Epic Games"),
        (winreg.HKEY_CURRENT_USER, r"Software\Epic Games\Unreal Engine\Identifiers"),
        (winreg.HKEY_CURRENT_USER, r"Software\Valve\Steam"),
        (winreg.HKEY_CURRENT_USER, r"Software\Valve\Steam\ActiveProcess"),
    ]

    for hive, key_path in keys_to_check:
        try:
            key = winreg.OpenKey(hive, key_path)
            i = 0
            while True:
                try:
                    name, value, vtype = winreg.EnumValue(key, i)
                    registry[f"{key_path}\\{name}"] = str(value)
                    i += 1
                except OSError:
                    break
        except:
            pass

    return registry

def take_snapshot(label):
    print(f"Taking '{label}' snapshot...")
    snapshot = {
        "label": label,
        "timestamp": datetime.now().isoformat(),
        "files": scan_files(),
        "registry": scan_registry(),
    }

    path = os.path.join(SNAPSHOT_DIR, f"epic_snapshot_{label}.json")
    with open(path, "w") as f:
        json.dump(snapshot, f, indent=2, default=str)

    print(f"Saved {len(snapshot['files'])} files + {len(snapshot['registry'])} registry keys to {path}")

def show_diff():
    before_path = os.path.join(SNAPSHOT_DIR, "epic_snapshot_before.json")
    after_path = os.path.join(SNAPSHOT_DIR, "epic_snapshot_after.json")

    if not os.path.exists(before_path) or not os.path.exists(after_path):
        print("Need both 'before' and 'after' snapshots. Run with 'before' and 'after' args first.")
        return

    with open(before_path) as f:
        before = json.load(f)
    with open(after_path) as f:
        after = json.load(f)

    print(f"\n{'='*80}")
    print(f"DIFF: {before['label']} ({before['timestamp']}) -> {after['label']} ({after['timestamp']})")
    print(f"{'='*80}\n")

    # File changes
    all_paths = set(list(before["files"].keys()) + list(after["files"].keys()))

    new_files = []
    deleted_files = []
    changed_files = []

    for path in sorted(all_paths):
        b = before["files"].get(path)
        a = after["files"].get(path)

        if b is None and a is not None:
            new_files.append((path, a))
        elif b is not None and a is None:
            deleted_files.append((path, b))
        elif b["hash"] != a["hash"]:
            changed_files.append((path, b, a))

    if new_files:
        print(f"[NEW] NEW FILES ({len(new_files)}):")
        for path, info in new_files:
            print(f"  + {path} ({info['size']} bytes)")
        print()

    if deleted_files:
        print(f"[DEL] DELETED FILES ({len(deleted_files)}):")
        for path, info in deleted_files:
            print(f"  - {path} ({info['size']} bytes)")
        print()

    if changed_files:
        print(f"[MOD] CHANGED FILES ({len(changed_files)}):")
        for path, b, a in changed_files:
            size_change = a["size"] - b["size"]
            sign = "+" if size_change >= 0 else ""
            print(f"  ~ {path}")
            print(f"    Size: {b['size']} -> {a['size']} ({sign}{size_change})")
        print()

    # Registry changes
    all_reg = set(list(before["registry"].keys()) + list(after["registry"].keys()))
    reg_changes = []

    for key in sorted(all_reg):
        b = before["registry"].get(key)
        a = after["registry"].get(key)
        if b != a:
            reg_changes.append((key, b, a))

    if reg_changes:
        print(f"[REG] REGISTRY CHANGES ({len(reg_changes)}):")
        for key, b, a in reg_changes:
            print(f"  {key}")
            print(f"    Before: {b}")
            print(f"    After:  {a}")
        print()

    if not new_files and not deleted_files and not changed_files and not reg_changes:
        print("No changes detected.")

    total = len(new_files) + len(deleted_files) + len(changed_files) + len(reg_changes)
    print(f"\nTotal changes: {total}")

if __name__ == "__main__":
    if len(sys.argv) < 2:
        print("Usage: python epic_snapshot.py [before|after|diff]")
        sys.exit(1)

    cmd = sys.argv[1]
    if cmd == "diff":
        show_diff()
    else:
        take_snapshot(cmd)
