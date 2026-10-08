#!/usr/bin/env python3
"""Agent Tree plugin: Explorer-style workspace/tab tree for the Herdr sidebar.

Modes:
  --rows            print the sidebar tree as JSON rows for Herdr to render
  --focus-tab       focus the tab identified by $HERDR_TAB_ID
  --open-workspace  focus $HERDR_WORKSPACE_ID, or toggle its tab list when it
                    is already focused (collapse state persists per session)

Environment provided by Herdr:
  HERDR_BIN_PATH, HERDR_SOCKET_PATH, HERDR_TAB_ID, HERDR_WORKSPACE_ID,
  HERDR_PLUGIN_STATE_DIR
"""

from __future__ import annotations

import json
import os
import subprocess
import sys

STATUS_ICONS = {
    "working": "●",
    "blocked": "●",
    "done": "●",
    "idle": "○",
    "unknown": "·",
}


def herdr() -> str:
    return os.environ.get("HERDR_BIN_PATH") or "herdr"


def api(args: list[str]) -> dict:
    proc = subprocess.run(
        [herdr(), *args],
        capture_output=True,
        text=True,
    )
    if proc.returncode != 0:
        print(proc.stderr, file=sys.stderr)
        sys.exit(proc.returncode)
    envelope = json.loads(proc.stdout)
    result = envelope.get("result")
    if result is None:
        raise SystemExit(f"unexpected response for {' '.join(args)}: {envelope}")
    return result


def state_path() -> str:
    base = os.environ.get("HERDR_PLUGIN_STATE_DIR")
    if not base:
        return os.devnull
    os.makedirs(base, exist_ok=True)
    return os.path.join(base, "collapsed.json")


def load_collapsed() -> set[str]:
    try:
        with open(state_path(), encoding="utf-8") as handle:
            return set(json.load(handle))
    except (OSError, ValueError):
        return set()


def save_collapsed(collapsed: set[str]) -> None:
    path = state_path()
    if path == os.devnull:
        return
    with open(path, "w", encoding="utf-8") as handle:
        json.dump(sorted(collapsed), handle)


def icon(status: str) -> str:
    return STATUS_ICONS.get(status, STATUS_ICONS["unknown"])


def rows() -> list[dict]:
    collapsed = load_collapsed()
    workspaces = api(["workspace", "list"]).get("workspaces", [])
    tabs = api(["tab", "list"]).get("tabs", [])
    out: list[dict] = []
    for workspace in workspaces:
        ws_id = workspace["workspace_id"]
        ws_tabs = [tab for tab in tabs if tab.get("workspace_id") == ws_id]
        collapsed_now = ws_id in collapsed
        chevron = "▸" if collapsed_now else ("▾" if ws_tabs else " ")
        marker = "❯" if workspace.get("focused") else " "
        title = f"{chevron} {icon(workspace.get('agent_status', 'unknown'))} {marker} {workspace['label']}"
        if collapsed_now and ws_tabs:
            title = f"{title} ({len(ws_tabs)})"
        out.append(
            {
                "title": title,
                "action": "open-workspace",
                "workspace_id": ws_id,
            }
        )
        if collapsed_now:
            continue
        for tab in ws_tabs:
            focused = tab.get("focused")
            tab_marker = "❯" if focused else " "
            out.append(
                {
                    "title": f"    {icon(tab.get('agent_status', 'unknown'))} {tab_marker} {tab['label']}",
                    "tab_id": tab["tab_id"],
                    "workspace_id": ws_id,
                }
            )
    return out


def focus_tab() -> None:
    tab_id = os.environ.get("HERDR_TAB_ID")
    if not tab_id:
        raise SystemExit("focus-tab requires HERDR_TAB_ID")
    api(["tab", "focus", tab_id])


def open_workspace() -> None:
    ws_id = os.environ.get("HERDR_WORKSPACE_ID")
    if not ws_id:
        raise SystemExit("open-workspace requires HERDR_WORKSPACE_ID")
    workspaces = api(["workspace", "list"]).get("workspaces", [])
    focused = next(
        (
            workspace
            for workspace in workspaces
            if workspace["workspace_id"] == ws_id and workspace.get("focused")
        ),
        None,
    )
    if focused is None:
        api(["workspace", "focus", ws_id])
        return
    collapsed = load_collapsed()
    if ws_id in collapsed:
        collapsed.discard(ws_id)
    else:
        collapsed.add(ws_id)
    save_collapsed(collapsed)


def main() -> None:
    if len(sys.argv) < 2:
        raise SystemExit("mode required: --rows, --focus-tab, --open-workspace")
    mode = sys.argv[1]
    if mode == "--rows":
        print(json.dumps(rows()))
    elif mode == "--focus-tab":
        focus_tab()
    elif mode == "--open-workspace":
        open_workspace()
    else:
        raise SystemExit(f"unknown mode: {mode}")


if __name__ == "__main__":
    main()
