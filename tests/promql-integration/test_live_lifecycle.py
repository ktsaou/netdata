#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Check that HTTP retirement requires a verified fixture identity."""

import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import Mock, patch

import live
from storage import REPO, safe_parent


class LiveIdentityTests(unittest.TestCase):
    def test_wrong_http_identity_never_receives_exit(self):
        parent = safe_parent(REPO / ".tmp")
        run = Path(tempfile.mkdtemp(prefix="live-identity-", dir=parent)).resolve(strict=True)
        process = Mock(pid=12345)
        process.poll.return_value = None
        process.wait.return_value = 0

        def launch(command, **options):
            root = Path(options["cwd"])
            (root / "cache/promql-live-ready.json").write_text(json.dumps(
                {"pid": process.pid, "port": 1, "host": "owned-fixture", "base": 1700002800, "child": "child"}))
            for name in sorted(live.HOST_CASES):
                options["stdout"].write((json.dumps({"host_readiness_case": name, "errors": 0}) + "\n").encode())
            return process

        response = {"status": 200}, json.dumps({"uid": "another-fixture", "mirrored_hosts": []}).encode()
        with patch.object(live.subprocess, "Popen", launch), patch.object(live, "request", return_value=response) as request:
            with self.assertRaisesRegex(RuntimeError, "HTTP instance identity differs"):
                live.qualify(run / "unused-netdata", "array", run)
        request.assert_called_once_with(1, "/api/v1/info")
        process.wait.assert_called_once_with(timeout=60)


if __name__ == "__main__":
    unittest.main()
