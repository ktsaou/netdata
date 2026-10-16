#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Check the actual HTTP launch environment without starting an Agent."""

import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import live
from storage import REPO, safe_parent


class LaunchRecorded(Exception):
    pass


class LiveLaunchTests(unittest.TestCase):
    def test_launch_uses_private_runtime_after_filtering_inherited_settings(self):
        parent = safe_parent(REPO / ".tmp")
        run = Path(tempfile.mkdtemp(prefix="live-launch-", dir=parent)).resolve(strict=True)
        captured = {}

        def record_launch(command, **options):
            captured.update(options)
            raise LaunchRecorded

        inherited = {"NETDATA_RUN_DIR": "/inherited-fixture-run",
                     "NETDATA_PIPENAME": "/inherited-fixture.pipe", "NETDATA_OTHER_FIXTURE": "unused"}
        with patch.dict(os.environ, inherited), patch.object(live.subprocess, "Popen", record_launch):
            with self.assertRaises(LaunchRecorded):
                live.qualify(run / "unused-netdata", "array", run)

        environment = captured["env"]
        self.assertEqual({key for key in environment if key.startswith("NETDATA_")},
                         {"NETDATA_RUN_DIR", "NETDATA_PIPENAME"})
        fixture = Path(captured["cwd"])
        runtime = Path(environment["NETDATA_RUN_DIR"]).resolve(strict=True)
        pipe = Path(environment["NETDATA_PIPENAME"])
        self.assertEqual(runtime.parent, fixture)
        self.assertEqual(runtime.stat().st_uid, os.getuid())
        self.assertEqual(runtime.stat().st_mode & 0o077, 0)
        self.assertEqual(pipe.parent.resolve(strict=True), runtime)
        self.assertEqual(environment["PROMQL_PROTOTYPE_FIXTURE_ROOT"], str(fixture))
        self.assertEqual(environment["DO_NOT_TRACK"], "1")
        self.assertLess(len(os.fsencode(pipe)), 104)


if __name__ == "__main__":
    unittest.main()
